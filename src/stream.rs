/*! Streams connecting blocks.

Blocks are connected with streams. A block can have zero or more input
streams, and write to zero or more output streams.

Both endpoints share a generated name such as `stream-42` and an optional sample
rate. Use `with_name("filtered")` and `with_sample_rate(48_000.)?` when connecting
blocks, including in `blockchain!` constructor arguments. Read metadata with
`name()` and `sample_rate()`. Duplicate names receive a unique suffix; empty
naming requests leave the name unchanged. Generated `stream-{id}` names are
reserved for their owning stream, including IDs not yet allocated. Rates must be
finite and positive; blocks do not infer or propagate them. Use [`streams()`] to
list live process streams and their metadata. The inventory does not keep stream buffers or endpoints alive.
*/
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use async_trait::async_trait;

use crate::{Error, Float, Len, Result};

/// Tag position in the current stream.
pub type TagPos = usize;

/// Enum of tag values.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TagValue {
    /// String value.
    String(String),

    /// Float value.
    Float(Float),

    /// Bool value.
    Bool(bool),

    /// U64 value.
    U64(u64),

    /// I64 value.
    I64(i64),
}

impl std::fmt::Display for TagValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            TagValue::String(s) => write!(f, "String:{s}"),
            TagValue::Float(s) => write!(f, "Float:{s}"),
            TagValue::Bool(s) => write!(f, "Bool:{s}"),
            TagValue::U64(s) => write!(f, "U64:{s}"),
            TagValue::I64(s) => write!(f, "I64:{s}"),
        }
    }
}

/// Tags associated with a stream.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Clone, PartialOrd)]
pub struct Tag {
    pos: TagPos,
    key: String,
    val: TagValue,
}

impl Tag {
    /// Create new tag.
    #[must_use]
    pub fn new<T: Into<String>>(pos: TagPos, key: T, val: TagValue) -> Self {
        Self {
            pos,
            key: key.into(),
            val,
        }
    }

    /// Get pos.
    ///
    /// Relative to the current window.
    #[must_use]
    pub fn pos(&self) -> TagPos {
        self.pos
    }

    /// Set pos.
    ///
    /// Relative to the current window.
    pub fn set_pos(&mut self, pos: TagPos) {
        self.pos = pos;
    }

    /// Get tag key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Get tag value.
    #[must_use]
    pub fn val(&self) -> &TagValue {
        &self.val
    }
}

/// Default stream size. Must be a multiple of the system page size.
///
/// Larger means better batching, but more RAM used. Twice as much virtual
/// address space as RAM is used.
///
/// Some experimentation with the multithreaded `GraphRunner` on 2025-02-15 with
/// ax25-1200-rx, in real time:
/// * 40_000KiB: 0.929s
/// *  4_000KiB: 1.066
/// *    400KiB: 1.228s
pub(crate) const DEFAULT_STREAM_SIZE: usize = 4_096_000;

const DEFAULT_NOCOPY_CAPACITY: usize = 1_000;

/// Result of waiting for samples or space on a stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitOutcome {
    /// The requested number of samples or slots is available.
    Ready,
    /// The wait ended without enough samples or slots, but the peer is open.
    /// Backends that cannot block may return this immediately.
    TimedOut,
    /// There are too few samples or slots and the peer has disconnected.
    Closed,
}

impl WaitOutcome {
    fn from_available(have: usize, need: usize, closed: bool) -> Self {
        if have >= need {
            Self::Ready
        } else if closed {
            Self::Closed
        } else {
            Self::TimedOut
        }
    }
}

/// Wait on a stream.
///
/// For `ReadStream`, wait until there's enough to read.
/// For `WriteStream`, wait until there's enough to write something.
#[async_trait]
pub trait StreamWait {
    /// ID shared between read and write side.
    #[must_use]
    fn id(&self) -> usize;

    /// Wait for "a while" or until `need` samples are available/space available.
    ///
    /// Buffered data that satisfies `need` returns [`WaitOutcome::Ready`]
    /// even after the peer disconnects. [`WaitOutcome::Closed`] means the
    /// requested amount is unavailable and the peer has disconnected.
    #[must_use]
    fn wait(&self, need: usize) -> WaitOutcome;

    /// Return true if the other end of this stream is disconnected.
    #[must_use]
    fn closed(&self) -> bool;

    /// Wait for "a while" or until `need` samples are available/space available.
    ///
    /// Returns the same outcomes as [`Self::wait`].
    ///
    /// `must_use` implied by async/Future.
    #[cfg(feature = "async")]
    async fn wait_async(&self, need: usize) -> WaitOutcome;
}

#[async_trait]
impl<T: Copy + Sync + Send + 'static> StreamWait for ReadStream<T> {
    fn id(&self) -> usize {
        self.circ.id()
    }
    fn wait(&self, need: usize) -> WaitOutcome {
        self.wait_for_read(need)
    }
    fn closed(&self) -> bool {
        self.refcount() == 1
    }
    #[cfg(feature = "async")]
    async fn wait_async(&self, need: usize) -> WaitOutcome {
        self.wait_for_read_async(need).await
    }
}

#[async_trait]
impl<T: Copy + Send + Sync> StreamWait for WriteStream<T> {
    fn id(&self) -> usize {
        self.circ.id()
    }
    fn wait(&self, need: usize) -> WaitOutcome {
        self.wait_for_write(need)
    }
    fn closed(&self) -> bool {
        self.refcount() == 1
    }
    #[cfg(feature = "async")]
    async fn wait_async(&self, need: usize) -> WaitOutcome {
        self.wait_for_write_async(need).await
    }
}

/// `ReadStream` is the reading side of a stream.
///
/// From the `ReadStream` you can get windows into the current stream by calling
/// `read_buf()`.
#[derive(Debug)]
pub struct ReadStream<T> {
    circ: Arc<crate::sys::Buffer<T>>,
}

impl<T: Default + Copy> ReadStream<T> {
    /// Create a new stream with initial data in it.
    #[cfg(test)]
    #[must_use]
    pub fn from_slice(data: &[T]) -> Self
    where
        T: Send + 'static,
    {
        let circ = Arc::new(crate::sys::Buffer::new(DEFAULT_STREAM_SIZE).unwrap()); // TODO
        let mut wb = circ.clone().write_buf().unwrap();
        wb.fill_from_slice(data);
        wb.produce(data.len(), &[]);
        circ.close_writer();
        Self { circ }
    }

    /// Return total length of underlying circular buffer (before the
    /// mapping doubling).
    #[must_use]
    pub fn total_size(&self) -> usize {
        self.circ.total_size()
    }

    /// Return a `BufferReader` allowing you to read from the stream, and
    /// "consume" from it.
    ///
    /// Tags are returned in ascending position order, preserving the order of
    /// tags at the same position.
    ///
    /// Only one read window may be outstanding, until it is consumed or
    /// dropped. A second checkout returns an error.
    ///
    /// See [`WriteStream::write_buf`] for details about the refcount checks.
    pub fn read_buf(&self) -> Result<(crate::sys::BufferReader<T>, Vec<Tag>)> {
        let refcount = Arc::strong_count(&self.circ);
        debug_assert!(refcount < 4, "read_buf() called with refcount {refcount}");
        if refcount > 3 {
            return Err(Error::msg(format!(
                "read_buf() called with refcount {refcount}"
            )));
        }
        Arc::clone(&self.circ).read_buf()
    }
}
impl<T: Copy> ReadStream<T> {
    /// Wait for samples, returning readiness, timeout, or peer closure.
    #[must_use]
    pub fn wait_for_read(&self, need: usize) -> WaitOutcome {
        WaitOutcome::from_available(self.circ.wait_for_read(need), need, self.refcount() == 1)
    }

    /// Wait asynchronously for samples, with the same outcomes as [`Self::wait_for_read`].
    #[cfg(feature = "async")]
    #[must_use]
    pub async fn wait_for_read_async(&self, need: usize) -> WaitOutcome {
        WaitOutcome::from_available(
            self.circ.wait_for_read_async(need).await,
            need,
            self.refcount() == 1,
        )
    }
}

impl<T> ReadStream<T> {
    /// Return true if there is nothing more ever to read from the stream.
    #[must_use]
    pub fn eof(&self) -> bool {
        // Fast path.
        let refcount = Arc::strong_count(&self.circ);
        if refcount != 1 {
            return false;
        }
        // Refcount 1 means that that the WriteStream has closed. No more data is coming. So as
        // long as the buffer is empty, that's it then.
        self.circ.is_empty()
    }

    #[must_use]
    pub(crate) fn refcount(&self) -> usize {
        Arc::strong_count(&self.circ)
    }
}

/// The write part of a stream.
#[derive(Debug)]
pub struct WriteStream<T> {
    circ: Arc<crate::sys::Buffer<T>>,
}

impl<T: Default + Copy + Send + 'static> WriteStream<T> {
    /// Create a stream pair, returning buffer creation errors.
    ///
    /// See [`new_stream`] for supported layouts and possible errors.
    pub fn new() -> Result<(WriteStream<T>, ReadStream<T>)> {
        new_stream()
    }
}

impl<T> StreamReadSide for WriteStream<T> {
    type ReadSide = ReadStream<T>;
}

impl<T: Copy> WriteStream<T> {
    /// Return free space in the stream, in samples.
    #[must_use]
    pub fn free(&self) -> usize {
        self.circ.free()
    }

    /// Return a `BufferWriter` for writing to the stream.
    ///
    /// Only one write window may be outstanding. A second checkout returns
    /// an error until the first window is produced or dropped. A read window
    /// may coexist with a write window because their sample ranges are disjoint.
    ///
    /// Ideally having a `BufferWriter` active on a stream should be prevented
    /// statically, but I've not come up with a way to do that.
    ///
    /// Having `write_buf` hold on to a mutable reference won't work, because
    /// streams are owned by blocks, and blocks need to be able to call their
    /// own mutable methods.
    ///
    /// `BufferWriters` do get an Arc to the circ buffer, though, so there should
    /// never be more than four references:
    /// * The source block.
    /// * The destination block.
    /// * The source `BufferWriter`.
    /// * The destination `BufferReader`.
    ///
    /// So this function needs to be called when the refcount is 3 or lower.
    ///
    /// Having more than four references is a definite coding bug, and hopefully
    /// will be caught by `MTGraph` testing during development.
    ///
    /// The above also goes for [`ReadStream::read_buf`].
    pub fn write_buf(&self) -> Result<crate::sys::BufferWriter<T>> {
        let refcount = Arc::strong_count(&self.circ);
        debug_assert!(refcount < 4, "write_buf() called with refcount {refcount}");
        if refcount > 3 {
            return Err(Error::msg(format!(
                "write_buf() called with refcount {refcount}"
            )));
        }
        Arc::clone(&self.circ).write_buf()
    }

    /// Wait for space, returning readiness, timeout, or peer closure.
    #[must_use]
    pub fn wait_for_write(&self, need: usize) -> WaitOutcome {
        WaitOutcome::from_available(self.circ.wait_for_write(need), need, self.refcount() == 1)
    }

    /// Wait asynchronously for space, with the same outcomes as [`Self::wait_for_write`].
    #[cfg(feature = "async")]
    #[must_use]
    pub async fn wait_for_write_async(&self, need: usize) -> WaitOutcome {
        WaitOutcome::from_available(
            self.circ.wait_for_write_async(need).await,
            need,
            self.refcount() == 1,
        )
    }

    #[must_use]
    pub(crate) fn refcount(&self) -> usize {
        Arc::strong_count(&self.circ)
    }
}

/// Create a new stream for data elements that implements Copy.
///
/// That's not to say that a bunch of Copy happens, but that it makes sense to
/// create sync blocks that take samples by value.
///
/// Basically anything that GNU Radio would *not* call a message port.
/// Storage starts with valid default values; zero bits need not represent a
/// valid element. Zero-sized element types are rejected at compile time.
///
/// # Errors
///
/// Items must be owned (`'static`) and transferable between threads (`Send`).
///
/// Returns buffer creation errors, including temporary-file or memory-mapping
/// failures and element layouts incompatible with the fixed buffer size.
pub fn new_stream<T: Default + Copy + Send + 'static>() -> Result<(WriteStream<T>, ReadStream<T>)> {
    let circ = Arc::new(crate::sys::Buffer::new(DEFAULT_STREAM_SIZE)?);
    Ok((WriteStream { circ: circ.clone() }, ReadStream { circ }))
}

struct NCEntry<T> {
    val: T,
    tags: Vec<Tag>,
}

struct NCState<T> {
    queue: VecDeque<NCEntry<T>>,
    observers: crate::stream_observer::Observers<T>,
    position: u64,
}

struct NCInner<T> {
    lock: Mutex<NCState<T>>,
    // Observer access may temporarily retain this owner. Endpoint closure must
    // therefore be independent of its Arc count; the mutex orders queue data.
    reader_closed: AtomicBool,
    writer_closed: AtomicBool,
    _control: Option<Arc<crate::stream_observer::TypedControl<T>>>,
    metadata: Arc<crate::stream_metadata::Metadata>,
    cv: Condvar,
    capacity: usize,

    // Waiting for read.
    #[cfg(feature = "async")]
    acvr: tokio::sync::Notify,

    // Waiting for write.
    #[cfg(feature = "async")]
    acvw: tokio::sync::Notify,
}

/// A stream of noncopyable objects (e.g. Vec / PDUs).
pub struct NCReadStream<T> {
    id: usize,
    inner: Arc<NCInner<T>>,
}

#[async_trait]
impl<T: Send> StreamWait for NCReadStream<T> {
    fn id(&self) -> usize {
        self.id
    }
    fn wait(&self, need: usize) -> WaitOutcome {
        let (s, _) = self
            .inner
            .cv
            .wait_timeout_while(
                self.inner.lock.lock().unwrap(),
                std::time::Duration::from_millis(100),
                |s| s.queue.len() < need && !self.closed(),
            )
            .unwrap();
        WaitOutcome::from_available(s.queue.len(), need, self.closed())
    }

    #[cfg(feature = "async")]
    async fn wait_async(&self, need: usize) -> WaitOutcome {
        tokio::time::timeout(std::time::Duration::from_millis(100), async {
            loop {
                let notified = self.inner.acvr.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let outcome = WaitOutcome::from_available(
                    self.inner.lock.lock().unwrap().queue.len(),
                    need,
                    self.closed(),
                );
                if outcome != WaitOutcome::TimedOut {
                    return outcome;
                }
                notified.await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            WaitOutcome::from_available(
                self.inner.lock.lock().unwrap().queue.len(),
                need,
                self.closed(),
            )
        })
    }
    fn closed(&self) -> bool {
        self.inner.writer_closed.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl<T: Send> StreamWait for NCWriteStream<T> {
    fn id(&self) -> usize {
        self.id
    }
    fn wait(&self, need: usize) -> WaitOutcome {
        let capacity = self.inner.capacity;
        let (s, _) = self
            .inner
            .cv
            .wait_timeout_while(
                self.inner.lock.lock().unwrap(),
                std::time::Duration::from_millis(100),
                |s| capacity.saturating_sub(s.queue.len()) < need && !self.closed(),
            )
            .unwrap();
        WaitOutcome::from_available(capacity.saturating_sub(s.queue.len()), need, self.closed())
    }
    #[cfg(feature = "async")]
    async fn wait_async(&self, need: usize) -> WaitOutcome {
        tokio::time::timeout(std::time::Duration::from_millis(100), async {
            loop {
                let notified = self.inner.acvw.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let outcome = WaitOutcome::from_available(self.remaining(), need, self.closed());
                if outcome != WaitOutcome::TimedOut {
                    return outcome;
                }
                notified.await;
            }
        })
        .await
        .unwrap_or_else(|_| WaitOutcome::from_available(self.remaining(), need, self.closed()))
    }
    fn closed(&self) -> bool {
        self.inner.reader_closed.load(Ordering::Relaxed)
    }
}

/// A stream of noncopyable objects (e.g. Vec / PDUs).
pub struct NCWriteStream<T> {
    id: usize,
    inner: Arc<NCInner<T>>,
}

/// Create a new stream for data elements that do not implement Copy.
///
/// This is likely going to be frames, packets, and (in GNU Radio) "messages",
/// which you would not want to just copy willy nilly. Messages must be owned
/// (`'static`) and transferable between threads (`Send`).
#[must_use]
pub fn new_nocopy_stream<T: Send + 'static>() -> (NCWriteStream<T>, NCReadStream<T>) {
    let id = crate::NEXT_STREAM_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let metadata = crate::stream_metadata::Metadata::new::<T>(id, true);
    let inner = Arc::new_cyclic(|owner| {
        let control = (metadata.kind != crate::stream_observer::Kind::Unsupported).then(|| {
            Arc::new(crate::stream_observer::TypedControl(Arc::new(NCControl {
                inner: owner.clone(),
            })))
        });
        NCInner {
            metadata,
            lock: Mutex::new(NCState {
                queue: VecDeque::new(),
                observers: Default::default(),
                position: 0,
            }),
            reader_closed: AtomicBool::new(false),
            writer_closed: AtomicBool::new(false),
            _control: control,
            cv: Condvar::new(),
            capacity: DEFAULT_NOCOPY_CAPACITY,
            #[cfg(feature = "async")]
            acvr: tokio::sync::Notify::new(),
            #[cfg(feature = "async")]
            acvw: tokio::sync::Notify::new(),
        }
    });
    // Publish only once the control's weak owner can be upgraded.
    if let Some(control) = &inner._control {
        crate::stream_metadata::Metadata::register(id, control.clone());
    }
    (
        NCWriteStream {
            id,
            inner: inner.clone(),
        },
        NCReadStream { id, inner },
    )
}

impl<T> NCReadStream<T> {
    /// Pop one owned item and its tags.
    ///
    /// Tags have positions relative to the item and are sorted in nondecreasing
    /// order. Equal-position tags preserve their supplied order.
    #[must_use]
    pub fn pop(&self) -> Option<(T, Vec<Tag>)> {
        let ret = self
            .inner
            .lock
            .lock()
            .unwrap()
            .queue
            .pop_front()
            .map(|v| (v.val, v.tags));
        self.inner.cv.notify_all();
        #[cfg(feature = "async")]
        self.inner.acvw.notify_one();
        ret
    }

    /// Return true if there is nothing more ever to read from the stream.
    #[must_use]
    pub fn eof(&self) -> bool {
        if self.inner.lock.lock().unwrap().queue.is_empty() {
            self.inner.writer_closed.load(Ordering::Relaxed)
        } else {
            false
        }
    }

    /// Return true is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let has = self.inner.lock.lock().unwrap().queue.len();
        has == 0
    }
}

/// Trait that helps finding the read side type of a write stream.
///
/// Used to simplify macros. You're unlikely to want to use this directly.
pub trait StreamReadSide {
    type ReadSide;
}

impl<T> StreamReadSide for NCWriteStream<T> {
    type ReadSide = NCReadStream<T>;
}
impl<T> NCWriteStream<T> {
    /// Create a new stream pair.
    #[must_use]
    pub fn new() -> (NCWriteStream<T>, NCReadStream<T>)
    where
        T: Send + 'static,
    {
        new_nocopy_stream()
    }
    /// Push one sample, handing off ownership.
    /// Ideally this should only be `NoCopy`.
    ///
    /// This function doesn't enforce capacity. If there's a risk of
    /// overflowing, then check `remaining()` before pushing.
    ///
    /// Tags must be sorted by nondecreasing position. Tags at the same position
    /// retain their supplied order; this method stores tags without sorting.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if tag positions are not sorted.
    pub fn push<Tags: Into<Vec<Tag>>>(&self, val: T, tags: Tags) {
        let tags = tags.into();
        debug_assert!(
            tags.windows(2).all(|pair| pair[0].pos() <= pair[1].pos()),
            "no-copy stream tags must be sorted by position"
        );
        let mut state = self.inner.lock.lock().unwrap();
        if !state.observers.0.is_empty() {
            let position = state.position;
            state
                .observers
                .samples(std::slice::from_ref(&val), &tags, position);
        }
        state.position = state
            .position
            .checked_add(1)
            .expect("absolute stream position overflow");
        state.queue.push_back(NCEntry { val, tags });
        drop(state);
        self.inner.cv.notify_all();
        #[cfg(feature = "async")]
        self.inner.acvr.notify_waiters();
    }

    /// Remaining capacity.
    #[must_use]
    pub fn remaining(&self) -> usize {
        let has = self.inner.lock.lock().unwrap().queue.len();
        self.inner.capacity.saturating_sub(has)
    }
}

impl<T: Len> NCReadStream<T> {
    /// Get the size of the front packet.
    #[must_use]
    pub fn peek_size(&self) -> Option<usize> {
        self.inner
            .lock
            .lock()
            .unwrap()
            .queue
            .front()
            .map(|e| e.val.len())
    }
}

struct NCControl<T> {
    inner: std::sync::Weak<NCInner<T>>,
}
impl<T: Send> crate::stream_observer::Control<T> for NCControl<T> {
    fn with_state(
        &self,
        action: &mut dyn FnMut(&mut crate::stream_observer::Observers<T>, u64, bool),
    ) {
        let Some(inner) = self.inner.upgrade() else {
            action(&mut Default::default(), 0, true);
            return;
        };
        let mut state = inner.lock.lock().unwrap();
        let position = state.position;
        let ended = inner.writer_closed.load(Ordering::Relaxed);
        action(&mut state.observers, position, ended);
    }
}
impl<T> Drop for WriteStream<T> {
    fn drop(&mut self) {
        self.circ.close_writer();
    }
}

impl<T> Drop for NCReadStream<T> {
    fn drop(&mut self) {
        self.inner.reader_closed.store(true, Ordering::Relaxed);
    }
}

impl<T> Drop for NCWriteStream<T> {
    fn drop(&mut self) {
        let mut state = self.inner.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.inner.writer_closed.store(true, Ordering::Relaxed);
        self.inner.metadata.set_ended();
        state.observers.end();
    }
}

pub use crate::stream_registry::{StreamInfo, streams};

macro_rules! stream_metadata {
    ($stream:ident, $field:ident) => {
        impl<T> $stream<T> {
            /// Snapshot of the stream's name, shared by both endpoints.
            #[must_use]
            pub fn name(&self) -> String {
                self.$field.metadata().name()
            }
            /// Name this stream. Both endpoints share the name.
            /// Duplicate names receive a unique suffix; read `name()` for the result.
            /// Empty names leave the metadata unchanged.
            #[must_use]
            pub fn with_name(self, name: impl Into<String>) -> Self {
                self.$field.metadata().set_name(name);
                self
            }
            /// Sample (or message) rate per second, when supplied.
            #[must_use]
            pub fn sample_rate(&self) -> Option<f64> {
                self.$field.metadata().sample_rate()
            }
            /// Attach a finite, positive rate in samples (or messages) per second.
            /// Both endpoints share the rate; blocks do not propagate it.
            pub fn with_sample_rate(self, rate: impl Into<f64>) -> Result<Self> {
                self.$field.metadata().set_sample_rate(rate.into())?;
                Ok(self)
            }
            /// Attach a finite, positive sample (or message) rate to this stream.
            pub fn with_sample_rate_hz(self, rate: f64) -> Result<Self> {
                self.with_sample_rate(rate)
            }
        }
    };
}
stream_metadata!(ReadStream, circ);
stream_metadata!(WriteStream, circ);
stream_metadata!(NCReadStream, inner);
stream_metadata!(NCWriteStream, inner);

impl<T> NCInner<T> {
    fn metadata(&self) -> &crate::stream_metadata::Metadata {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn observer_controls_do_not_retain_primary_storage() -> Result<()> {
        let (writer, reader) = new_stream::<u8>()?;
        let buffer = Arc::downgrade(&writer.circ);
        let (_metadata, control) = crate::stream_registry::lookup::<u8>(writer.id()).unwrap();
        drop(writer);
        assert!(reader.eof());
        drop(reader);
        assert!(buffer.upgrade().is_none());
        control.with_state(&mut |_, _, ended| assert!(ended));

        let (writer, reader) = new_nocopy_stream::<String>();
        let queue = Arc::downgrade(&writer.inner);
        let (_metadata, control) = crate::stream_registry::lookup::<String>(writer.id()).unwrap();
        writer.push("pending".into(), []);
        drop(writer);
        drop(reader);
        assert!(queue.upgrade().is_none());
        control.with_state(&mut |_, _, ended| assert!(ended));
        Ok(())
    }

    #[test]
    fn observer_access_does_not_change_copying_endpoint_counts() -> Result<()> {
        let (writer, reader) = new_stream::<u8>()?;
        let (_, control) = crate::stream_registry::lookup::<u8>(writer.id()).unwrap();
        control.with_state(&mut |_, _, _| {
            assert_eq!(reader.refcount(), 2);
            assert_eq!(writer.refcount(), 2);
        });
        Ok(())
    }

    #[test]
    fn observer_access_does_not_mask_nocopy_endpoint_closure() {
        let (writer, reader) = new_nocopy_stream::<String>();
        let (_, control) = crate::stream_registry::lookup::<String>(writer.id()).unwrap();
        let mut reader = Some(reader);
        control.with_state(&mut |_, _, _| {
            drop(reader.take());
            assert!(writer.closed());
        });

        let (writer, reader) = new_nocopy_stream::<String>();
        let (_, control) = crate::stream_registry::lookup::<String>(writer.id()).unwrap();
        writer.push("pending".into(), []);
        drop(writer);
        control.with_state(&mut |_, _, ended| {
            assert!(ended);
            assert!(reader.closed());
        });
        assert!(!reader.eof());
        reader.pop().unwrap();
        assert!(reader.eof());
    }

    #[test]
    fn producer_completion_does_not_wait_for_buffered_data_or_read_windows() -> Result<()> {
        let (writer, reader) = new_stream::<u8>()?;
        assert!(!reader.circ.metadata().ended());
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[1, 2]);
        window.produce(2, &[]);
        let (reading, _) = reader.read_buf()?;
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[3, 4]);
        drop(writer);
        assert!(!reader.circ.metadata().ended());
        window.produce(2, [Tag::new(0, "last", TagValue::Bool(true))].as_slice());
        assert!(reader.circ.metadata().ended());
        assert_eq!(reading.slice(), &[1, 2]);
        drop(reading);
        assert!(!reader.eof());
        let (reading, tags) = reader.read_buf()?;
        assert_eq!(reading.slice(), &[1, 2, 3, 4]);
        assert_eq!(tags, [Tag::new(2, "last", TagValue::Bool(true))]);
        reading.consume(4);
        assert!(reader.eof());
        Ok(())
    }

    #[test]
    fn producer_completion_waits_for_dropped_or_zero_length_write_windows() -> Result<()> {
        for produce_zero in [false, true] {
            let (writer, reader) = new_stream::<u8>()?;
            let window = writer.write_buf()?;
            drop(writer);
            assert!(!reader.circ.metadata().ended());
            if produce_zero {
                window.produce(0, &[]);
            } else {
                drop(window);
            }
            assert!(reader.circ.metadata().ended());
            assert!(reader.eof());
        }
        Ok(())
    }

    #[test]
    fn producer_completion_requires_writer_drop() -> Result<()> {
        let (writer, reader) = new_stream::<u8>()?;
        drop(writer.write_buf()?);
        assert!(!reader.circ.metadata().ended());
        writer.write_buf()?.produce(0, &[]);
        assert!(!reader.circ.metadata().ended());
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[1]);
        window.produce(1, &[]);
        assert!(!reader.circ.metadata().ended());
        reader.read_buf()?.0.consume(1);
        drop(writer);
        assert!(reader.circ.metadata().ended());
        assert!(reader.eof());
        Ok(())
    }

    #[test]
    fn producer_completion_survives_write_window_unwinding() -> Result<()> {
        let (writer, reader) = new_stream::<u8>()?;
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[1]);
        drop(writer);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            window.produce(1, [Tag::new(1, "invalid", TagValue::Bool(true))].as_slice());
        }));
        assert!(result.is_err());
        assert!(reader.circ.metadata().ended());
        Ok(())
    }

    #[test]
    fn nocopy_producer_completion_precedes_queue_drain() {
        let (writer, reader) = new_nocopy_stream::<String>();
        assert!(!reader.inner.metadata.ended());
        writer.push("message".into(), []);
        drop(writer);
        assert!(reader.inner.metadata.ended());
        assert!(!reader.eof());
        assert_eq!(reader.pop().unwrap().0, "message");
        assert!(reader.eof());
    }

    #[test]
    fn copy_read_wait_distinguishes_readiness_timeout_and_closure() -> Result<()> {
        let (writer, reader) = new_stream::<u8>()?;
        assert_eq!(reader.wait_for_read(1), WaitOutcome::TimedOut);
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[1, 2]);
        window.produce(2, &[]);
        assert_eq!(reader.wait_for_read(2), WaitOutcome::Ready);
        drop(writer);
        assert_eq!(reader.wait_for_read(2), WaitOutcome::Ready);
        assert_eq!(reader.wait_for_read(3), WaitOutcome::Closed);
        reader.read_buf()?.0.consume(2);
        assert_eq!(reader.wait_for_read(1), WaitOutcome::Closed);
        Ok(())
    }

    #[test]
    fn copy_write_wait_distinguishes_readiness_timeout_and_closure() -> Result<()> {
        let (writer, reader) = new_stream::<u8>()?;
        assert_eq!(writer.wait_for_write(1), WaitOutcome::Ready);
        let mut window = writer.write_buf()?;
        let size = window.len();
        window.slice().fill(0);
        window.produce(size, &[]);
        assert_eq!(writer.wait_for_write(1), WaitOutcome::TimedOut);
        reader.read_buf()?.0.consume(1);
        assert_eq!(writer.wait_for_write(1), WaitOutcome::Ready);
        drop(reader);
        assert_eq!(writer.wait_for_write(2), WaitOutcome::Closed);
        Ok(())
    }

    #[test]
    fn nocopy_waits_distinguish_outcomes_and_preserve_advisory_capacity() {
        let (writer, reader) = new_nocopy_stream::<u8>();
        assert_eq!(reader.wait(1), WaitOutcome::TimedOut);
        assert_eq!(writer.wait(1), WaitOutcome::Ready);
        for _ in 0..=DEFAULT_NOCOPY_CAPACITY {
            writer.push(1, []);
        }
        assert_eq!(writer.remaining(), 0);
        assert_eq!(reader.wait(DEFAULT_NOCOPY_CAPACITY + 1), WaitOutcome::Ready);
        assert_eq!(writer.wait(1), WaitOutcome::TimedOut);
        let _ = reader.pop();
        let _ = reader.pop();
        assert_eq!(writer.wait(1), WaitOutcome::Ready);
        drop(writer);
        assert_eq!(reader.wait(1), WaitOutcome::Ready);
        assert_eq!(reader.wait(DEFAULT_NOCOPY_CAPACITY), WaitOutcome::Closed);
        let (writer, reader) = new_nocopy_stream::<u8>();
        for _ in 0..DEFAULT_NOCOPY_CAPACITY {
            writer.push(1, []);
        }
        drop(reader);
        assert_eq!(writer.wait(1), WaitOutcome::Closed);
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_copy_waits_distinguish_outcomes() -> Result<()> {
        let (writer, reader) = new_stream::<u8>()?;
        assert_eq!(reader.wait_async(1).await, WaitOutcome::TimedOut);
        assert_eq!(writer.wait_async(1).await, WaitOutcome::Ready);
        let mut window = writer.write_buf()?;
        let size = window.len();
        window.slice().fill(0);
        window.produce(size, &[]);
        assert_eq!(writer.wait_async(1).await, WaitOutcome::TimedOut);
        assert_eq!(reader.wait_async(size).await, WaitOutcome::Ready);
        drop(writer);
        assert_eq!(reader.wait_async(size).await, WaitOutcome::Ready);
        reader.read_buf()?.0.consume(size);
        assert_eq!(reader.wait_async(1).await, WaitOutcome::Closed);

        let (writer, reader) = new_stream::<u8>()?;
        let mut window = writer.write_buf()?;
        let size = window.len();
        window.slice().fill(0);
        window.produce(size, &[]);
        drop(reader);
        assert_eq!(writer.wait_async(1).await, WaitOutcome::Closed);
        Ok(())
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_nocopy_waits_distinguish_outcomes_and_wake_for_data() {
        let (writer, reader) = new_nocopy_stream::<u8>();
        assert_eq!(reader.wait_async(1).await, WaitOutcome::TimedOut);
        let (outcome, ()) = tokio::join!(reader.wait_async(2), async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            writer.push(1, []);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            writer.push(2, []);
        });
        assert_eq!(outcome, WaitOutcome::Ready);
        drop(writer);
        assert_eq!(reader.wait_async(2).await, WaitOutcome::Ready);
        assert_eq!(reader.wait_async(3).await, WaitOutcome::Closed);

        let (writer, reader) = new_nocopy_stream::<u8>();
        assert_eq!(writer.wait_async(1).await, WaitOutcome::Ready);
        for _ in 0..DEFAULT_NOCOPY_CAPACITY {
            writer.push(1, []);
        }
        assert_eq!(writer.wait_async(1).await, WaitOutcome::TimedOut);
        let (outcome, ()) = tokio::join!(writer.wait_async(1), async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let _ = reader.pop();
        });
        assert_eq!(outcome, WaitOutcome::Ready);
        writer.push(1, []);
        drop(reader);
        assert_eq!(writer.wait_async(1).await, WaitOutcome::Closed);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn too_many_references_to_buffer() {
        let (tx, rx) = new_stream::<u8>().unwrap();
        let _writer = tx.write_buf();

        // This fails normally. Refcount is not too bad.
        assert!(tx.write_buf().is_err());

        // With this line there's now one read buf and write buf. No more is
        // allowed.
        let _t = rx.read_buf().unwrap();

        // This asserts because the refcount is completely unbelievable.
        let _ = rx.read_buf();
    }

    #[test]
    fn windows_are_exclusive_per_endpoint_and_released_on_drop() -> Result<()> {
        let (tx, rx) = new_stream::<u8>()?;
        let mut writer = tx.write_buf()?;
        assert!(tx.write_buf().is_err());
        let (reader, _) = rx.read_buf()?;
        //assert!(rx.read_buf().is_err());
        // Opposite endpoints can hold windows at the same time.
        writer.fill_from_slice(&[1, 2]);
        writer.produce(2, &[]);
        assert!(reader.is_empty());
        drop(reader);
        let (reader, _) = rx.read_buf()?;
        assert_eq!(reader.slice(), [1, 2]);
        let writer = tx.write_buf()?;
        //assert!(tx.write_buf().is_err());
        reader.consume(2);
        drop(writer);
        assert!(tx.write_buf().is_ok());
        assert!(rx.read_buf()?.0.is_empty());
        Ok(())
    }

    #[test]
    fn reader_cannot_consume_samples_produced_after_checkout() -> Result<()> {
        let (tx, rx) = new_stream::<u8>()?;
        let (reader, _) = rx.read_buf()?;
        let mut writer = tx.write_buf()?;
        writer.fill_from_slice(&[1]);
        writer.produce(1, &[]);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader.consume(1))).is_err()
        );
        assert_eq!(rx.read_buf()?.0.slice(), [1]);
        Ok(())
    }

    #[test]
    fn writer_cannot_produce_space_freed_after_checkout() -> Result<()> {
        let (tx, rx) = new_stream::<u8>()?;
        let mut writer = tx.write_buf()?;
        let len = writer.len();
        writer.slice().fill(1);
        writer.produce(len, &[]);
        let writer = tx.write_buf()?;
        rx.read_buf()?.0.consume(1);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer.produce(1, &[])))
                .is_err()
        );
        assert_eq!(rx.read_buf()?.0.len(), len - 1);
        Ok(())
    }

    use super::*;

    #[test]
    fn nocopy_tags_preserve_nondecreasing_position_order() {
        let (writer, reader) = new_nocopy_stream();
        for tags in [
            vec![],
            vec![Tag::new(3, "single", TagValue::Bool(true))],
            vec![
                Tag::new(0, "first", TagValue::Bool(true)),
                Tag::new(0, "second", TagValue::Bool(false)),
                Tag::new(3, "last", TagValue::Bool(true)),
            ],
        ] {
            writer.push(42u8, tags.clone());
            assert_eq!(reader.pop(), Some((42, tags)));
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "no-copy stream tags must be sorted by position")]
    fn nocopy_push_rejects_unsorted_tags_in_debug_builds() {
        let (writer, _reader) = new_nocopy_stream();
        writer.push(
            42u8,
            [
                Tag::new(2, "later", TagValue::Bool(true)),
                Tag::new(1, "earlier", TagValue::Bool(true)),
            ],
        );
    }
}
