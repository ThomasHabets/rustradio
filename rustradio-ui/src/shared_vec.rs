use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use rustradio::stream::Tag;
use rustradio::{Complex, Float};

/// Buffer ID needs to encode which buffer type it is, and than 64bit. But JS
/// can't encode u64 so we need to split it manually.
///
/// An initial test encoded as string, but that was incredibly slow.
pub type BufferId = (u8, u32, u32);

thread_local! {
    pub(crate) static PENDING_SHARED_BUFFERS_U8: RefCell<HashMap<u64, Vec<u8>>> = RefCell::new(HashMap::new());
    static PENDING_SHARED_BUFFERS_FLOAT: RefCell<HashMap<u64, Vec<Float>>> = RefCell::new(HashMap::new());
    static PENDING_SHARED_BUFFERS_COMPLEX: RefCell<HashMap<u64, Vec<Complex>>> = RefCell::new(HashMap::new());
    static NEXT_SHARED_BUFFER_ID: Cell<u64> = const { Cell::new(1) };
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedVecType {
    Byte = 1,
    Float = 2,
    Complex = 3,
}

/// When a shared vec is sent to a different worker, it has to remain living in
/// the original worker / main UI. So when creating a `SharedVec` we insert it
/// into a buffer, and never look at it again.
///
/// When the other worker sends back a message saying it's done with the shared
/// vec, we remove it and thus drop it.
///
/// This "registry" is strongly typed, so every type of `SharedVec` needs a
/// registry implementation.
pub trait SharedVecRegistry: Sized {
    /// Move ownership of this Vec into the registry.
    ///
    /// It must stay there until the remote worker is done with it.
    fn registry_insert(v: Vec<Self>) -> BufferId;

    /// Remove the `Vec`. This drops it, and any further access by *any* thread
    /// is Bad.
    ///
    /// This removal is generally triggered by the `SharedVec` destructor in the
    /// remote worker, so it should be safe.
    fn registry_remove(id: BufferId);
}

macro_rules! impl_shared_vec_registry {
    ($ty:ty, $num:expr, $registry:ident) => {
        impl SharedVecRegistry for $ty {
            fn registry_insert(v: Vec<Self>) -> BufferId {
                let id = NEXT_SHARED_BUFFER_ID.with(|slot: &Cell<u64>| {
                    let n= slot.get();
                    slot.set(n.wrapping_add(1));
                    eprintln!("Set next to {n}");
                    assert_ne!(n, 0, "64 bit buffer counter wrapped");
                    ($num as u8, (n >> 32) as u32, n as u32)
                });
                let n = ((id.1 as u64) << 32) + (id.2 as u64);
                eprintln!("{id:?} {n}");
                if let Some(_) = $registry.with(|slot| slot.borrow_mut().insert(n, v)) {
                    // This can't happen unless we wrap an usize.
                    log::error!("SharedVecRegistry double-registered {n} {id:?}");
                }
                id
            }
            fn registry_remove(id: BufferId) {
                assert_eq!(id.0, $num as u8);
                let n = ((id.1 as u64) << 32) + (id.2 as u64);
                if $registry.with(|slot| {
                    slot.borrow_mut().remove(&n)
                }).is_none() {
                    panic!("SharedVecRegistry tried to double-free id {id:?}. This probably means memory corrucption has already happened.");
                }
            }
        }
    };
}

impl_shared_vec_registry!(u8, SharedVecType::Byte, PENDING_SHARED_BUFFERS_U8);
impl_shared_vec_registry!(Float, SharedVecType::Float, PENDING_SHARED_BUFFERS_FLOAT);
impl_shared_vec_registry!(
    Complex,
    SharedVecType::Complex,
    PENDING_SHARED_BUFFERS_COMPLEX
);

pub fn discard_shared_vec(id: BufferId) {
    match id.0 {
        1 => u8::registry_remove(id),
        2 => Float::registry_remove(id),
        3 => Complex::registry_remove(id),
        _ => panic!("Invalid id {id:?}"),
    }
}

/// This is the struct sent over the serialization layer for shared buffer data.
///
/// When received, a SharedVecPtr MUST reassembled into a `SharedVec` after
/// reception, or the memory will leak.
///
/// We can't trigger the drop from `SharedVecPtr`, because that would make it
/// trigger in the sending worker, and we don't want that (it'd be sent to the
/// wrong worker, which has no ability to drop it from the registry).
///
/// We could have the drop message trigger only on `SharedVecPtr` created by
/// deserialization, not by `new()`, but that would probably be too surprising.
/// Also it would misfire if the sender (for some reason) were to serialize and
/// deserialize locally).
///
/// There is no `Ref` version of `SharedVecPtr`, because it's expected that the
/// non-sample part of the message has minimal overhead from an extra copy. This
/// assumption should be verified.
#[derive(Debug, Serialize, Deserialize)]
pub struct SharedVecPtr<T> {
    // ID is the tracker used for holding the vector alive. When the receiver
    // of a shared vec is done with it, it sends this ID back to free it.
    id: BufferId,

    // This is the data's location in memory, and its size in number of
    // elements.
    ptr: usize,
    len: usize,

    // Tags are expected to be much smaller than the samples, so we send them
    // unshared for now.
    tags: Vec<Tag>,

    dummy: std::marker::PhantomData<T>,
}

impl<T: SharedVecRegistry> SharedVecPtr<T> {
    #[must_use]
    pub fn new(v: impl Into<Vec<T>>, tags: impl Into<Vec<Tag>>) -> Self {
        let v = v.into();
        let ptr = v.as_ptr() as usize;
        let len = v.len();
        let id = T::registry_insert(v);
        Self {
            id,
            ptr,
            len,
            tags: tags.into(),
            dummy: std::marker::PhantomData,
        }
    }
}

/// A `SharedVec` is used on the receiver side of a message, reading the data.
///
/// When dropped, it calls a user provided function that should tell the
/// original worker to drop the underlying `Vec` it's holding in its registry.
///
/// The callback can't be avoided because the messages and way of sending the
/// messages are different depending on if this is `WorkerToMain` or
/// `MainToWorker`.
// TODO: though this is serialized with the type name, so maybe we can actually
// serialize something that works for either one.
#[derive(Debug)]
pub struct SharedVec<T> {
    ptr: SharedVecPtr<T>,
    post: fn(BufferId) -> rustradio::Result<()>,
}

impl<T> SharedVec<T> {
    /// Create a usable reader of a shared vector.
    ///
    /// When `SharedVec` is dropped, the `post` callback is called. That
    /// callback should send a message to the owner of the underlying `Vec` so
    /// telling it that the `Vec` can be dropped.
    #[must_use]
    pub fn new(ptr: SharedVecPtr<T>, post: fn(BufferId) -> rustradio::Result<()>) -> Self {
        Self { ptr, post }
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.ptr.len
    }
}

impl<T> std::convert::AsRef<[T]> for SharedVec<T> {
    fn as_ref(&self) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.ptr.ptr as *const T, self.ptr.len) }
    }
}

impl<T> std::ops::Index<usize> for SharedVec<T> {
    type Output = T;
    fn index(&self, i: usize) -> &Self::Output {
        &self.as_ref()[i]
    }
}

impl<T> Drop for SharedVec<T> {
    fn drop(&mut self) {
        let id = std::mem::take(&mut self.ptr.id);
        if let Err(e) = (self.post)(id) {
            log::error!("SharedVec failed to unregister. Likely memory leak resulting: {e}");
        }
    }
}
