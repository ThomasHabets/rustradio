#![doc = include_str!("../README.md")]
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use rustradio::stream::Tag;
use rustradio::{Complex, Float};

thread_local! {
    static PENDING_SHARED_BUFFERS_U8: RefCell<HashMap<usize, Vec<u8>>> = RefCell::new(HashMap::new());
    static PENDING_SHARED_BUFFERS_FLOAT: RefCell<HashMap<usize, Vec<Float>>> = RefCell::new(HashMap::new());
    static PENDING_SHARED_BUFFERS_COMPLEX: RefCell<HashMap<usize, Vec<Complex>>> = RefCell::new(HashMap::new());
    static NEXT_SHARED_BUFFER_ID: Cell<usize> = const { Cell::new(1) };
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
    fn registry_insert(v: Vec<Self>) -> usize;

    /// Remove the `Vec`. This drops it, and any further access by *any* thread
    /// is Bad.
    ///
    /// This removal is generally triggered by the `SharedVec` destructor in the
    /// remote worker, so it should be safe.
    fn registry_remove(id: usize);
}

macro_rules! impl_shared_vec_registry {
    ($ty:ty, $registry:ident) => {
        impl SharedVecRegistry for $ty {
            fn registry_insert(v: Vec<Self>) -> usize {
                let id = NEXT_SHARED_BUFFER_ID.with(|slot: &Cell<usize>| {
                    let id: usize = slot.get();
                    slot.set(id.wrapping_add(1));
                    id
                });
                if let Some(_) = $registry.with(|slot| slot.borrow_mut().insert(id, v)) {
                    // This can't happen unless we wrap an usize.
                    log::error!("SharedVecRegistry double-registered {id}");
                }
                id
            }
            fn registry_remove(id: usize) {
                if $registry.with(|slot| {
                    slot.borrow_mut().remove(&id)
                }).is_none() {
                    panic!("SharedVecRegistry tried to double-free id {id}. This probably means memory corrucption has already happened.");
                }
            }
        }
    };
}

impl_shared_vec_registry!(u8, PENDING_SHARED_BUFFERS_U8);
impl_shared_vec_registry!(Float, PENDING_SHARED_BUFFERS_FLOAT);
impl_shared_vec_registry!(Complex, PENDING_SHARED_BUFFERS_COMPLEX);

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
    id: usize,

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
    post: fn(usize) -> rustradio::Result<()>,
}

impl<T> SharedVec<T> {
    /// Create a usable reader of a shared vector.
    ///
    /// When `SharedVec` is dropped, the `post` callback is called. That
    /// callback should send a message to the owner of the underlying `Vec` so
    /// telling it that the `Vec` can be dropped.
    #[must_use]
    pub fn new(ptr: SharedVecPtr<T>, post: fn(usize) -> rustradio::Result<()>) -> Self {
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

impl<T> Drop for SharedVec<T> {
    fn drop(&mut self) {
        if let Err(e) = (self.post)(self.ptr.id) {
            log::error!("SharedVec failed to unregister. Likely memory leak resulting: {e}");
        }
    }
}

/// Application specific extensions to MainToWorker and WorkerToMain.
pub trait ApplicationSpecific {
    // Can't default. https://github.com/rust-lang/rust/issues/29661
    type App: Serialize;
    type Start: Serialize;
    type Ready: Serialize;
    type End: Serialize;
}

/// Messages going from main (UI) thread to worker.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(bound(
    serialize = "App::App: Serialize, App::Start: Serialize",
    deserialize = "App::App: Deserialize<'de>, App::Start: Deserialize<'de>",
))]
pub enum MainToWorker<App: ApplicationSpecific> {
    /// Start the graph with the relevant startup parameters.
    Start(App::Start),

    /// Application specific stuff.
    ApplicationSpecific(App::App),

    /// Raw DATA_STREAM protocol bytes received from the selected input source.
    DataStream(Vec<u8>),

    /// u8 vectors held in a shared buffer.
    ///
    /// This is useful for sending RTL-SDR raw bytes from the main UI thread
    /// (which gets it from WebUSB) to the worker thread.
    SharedByte(String, Vec<SharedVecPtr<u8>>),

    /// Tell the remote end that we are done with this shared buffer.
    DiscardSharedVec(usize),

    /// Send a ping with a `performance.now()` timestamp.
    /// The timestamp will be reflected in the Pong.
    Ping(f64),

    /// Reply to a ping from the worker.
    ///
    /// Original ping timestamp is returned.
    Pong(f64),
}

impl<App: ApplicationSpecific> TryInto<wasm_bindgen::JsValue> for MainToWorker<App> {
    type Error = wasm_bindgen::JsValue;
    fn try_into(self) -> Result<wasm_bindgen::JsValue, Self::Error> {
        Ok(serde_wasm_bindgen::to_value(&self)?)
    }
}

impl<App> TryFrom<wasm_bindgen::JsValue> for MainToWorker<App>
where
    App: ApplicationSpecific,
    App::App: serde::de::DeserializeOwned,
    App::Start: serde::de::DeserializeOwned,
{
    type Error = wasm_bindgen::JsValue;
    fn try_from(js: wasm_bindgen::JsValue) -> Result<MainToWorker<App>, Self::Error> {
        Ok(serde_wasm_bindgen::from_value(js)?)
    }
}

/// Messages from the worker to the main (UI) thread.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(bound(
    serialize = "App::App: Serialize, App::Ready: Serialize, App::End: Serialize",
    deserialize = "App::App: Deserialize<'de>, App::Ready: Deserialize<'de>, App::End: Deserialize<'de>",
))]
pub enum WorkerToMain<App: ApplicationSpecific = AppEmpty> {
    /// Worker notifying the main UI thread that the rustradio graph has
    /// successfully started.
    Ready(App::Ready),

    /// Application specific messages.
    ApplicationSpecific(App::App),

    /// Send a ping with a `performance.now()` timestamp.
    /// The timestamp will be reflected in the Pong.
    Ping(f64),

    /// Reply to a ping from the main thread.
    ///
    /// Original ping timestamp is returned.
    Pong(f64),

    /// Raw DATA_STREAM protocol bytes to send to the selected input source.
    DataStream(Vec<u8>),

    /// Float vectors held in a shared buffer.
    ///
    /// This is used for streams as well as for fixed block sized things like
    /// spectrums and waterfalls.
    SharedFloat(String, Vec<SharedVecPtr<Float>>),

    /// Complex vectors held in a shared buffer.
    SharedComplex(String, Vec<SharedVecPtr<Complex>>),

    /// At the end of execution, provide the result as a string.
    End(App::End),

    /// A worker log line to be emitted through the main thread logger.
    LogLine { level: log::Level, line: String },

    /// Float streams captured in the worker graph.
    ///
    /// TODO: This should be one receiver, multiple streams.
    FloatStreams(Vec<FloatStream>),

    /// Complex streams captured in the worker graph.
    /// TODO: This should be one receiver, multiple streams.
    ComplexStreams(Vec<ComplexStream>),

    /// Float PDU streams captured in the worker graph.
    ///
    /// TODO: this should only be the one packet per packet, right?
    FloatPduStreams(Vec<FloatPduStream>),
}

/// Borrowed version of WorkerToMain. Must serialize the same.
///
/// This is mainly for avoiding a copy while serializing. Some variants can also
/// be deserialized, but stream reference payloads deserialize as owned streams
/// because their slice fields cannot be borrowed from every serde input.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(bound(
    serialize = "App::App: Serialize, App::Ready: Serialize, App::End: Serialize",
    deserialize = "App::App: Deserialize<'de>, App::Ready: Deserialize<'de>, App::End: Deserialize<'de>",
))]
pub enum WorkerToMainRef<'a, App: ApplicationSpecific = AppEmpty> {
    /// Worker notifying the main UI thread that the rustradio graph has
    /// successfully started.
    Ready(App::Ready),

    /// Application specific messages.
    ApplicationSpecific(App::App),

    /// Send a ping with a `performance.now()` timestamp.
    /// The timestamp will be reflected in the Pong.
    Ping(f64),

    /// Reply to a ping from the main thread.
    ///
    /// Original ping timestamp is returned.
    Pong(f64),

    /// Raw DATA_STREAM protocol bytes to send to the selected input source.
    DataStream(&'a [u8]),

    /// At the end of execution, provide the result as a string.
    End(App::End),

    /// A worker log line to be emitted through the main thread logger.
    LogLine {
        level: log::Level,
        line: &'a str,
    },

    FloatStreams(Vec<FloatStreamCow<'a>>),

    ComplexStreams(Vec<ComplexStreamCow<'a>>),

    /// Float PDU streams captured in the worker graph.
    ///
    /// TODO: this should only be the one packet per packet, right?
    /// TODO: make this borrow.
    FloatPduStreams(Vec<FloatPduStream>),
}

impl<App: ApplicationSpecific> TryInto<wasm_bindgen::JsValue> for WorkerToMain<App> {
    type Error = wasm_bindgen::JsValue;
    fn try_into(self) -> Result<wasm_bindgen::JsValue, Self::Error> {
        Ok(serde_wasm_bindgen::to_value(&self)?)
    }
}

impl<App> TryFrom<wasm_bindgen::JsValue> for WorkerToMain<App>
where
    App: ApplicationSpecific,
    App::App: serde::de::DeserializeOwned,
    App::Ready: serde::de::DeserializeOwned,
    App::End: serde::de::DeserializeOwned,
{
    type Error = wasm_bindgen::JsValue;
    fn try_from(js: wasm_bindgen::JsValue) -> Result<WorkerToMain<App>, Self::Error> {
        Ok(serde_wasm_bindgen::from_value(js)?)
    }
}

/// Stream of floats going between worker and UI thread.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, PartialOrd)]
pub struct FloatStream {
    pub name: String,
    pub tags: Vec<rustradio::stream::Tag>,
    pub samples: Vec<rustradio::Float>,
}

/// Borrow version of `FloatStream`.
///
/// Used to avoid copies when e.g. sending directly from a RustRadio stream.
///
/// Must serialize the same as `FloatStream`.
#[derive(Serialize, Clone)]
pub struct FloatStreamRef<'a> {
    pub name: &'a str,
    pub tags: &'a [rustradio::stream::Tag],
    pub samples: &'a [rustradio::Float],
}

impl FloatStreamRef<'_> {
    fn to_owned(&self) -> FloatStream {
        FloatStream {
            name: self.name.to_string(),
            tags: self.tags.to_vec(),
            samples: self.samples.to_vec(),
        }
    }
}

/// Owned stream type that can produce an equivalent borrowed stream reference.
pub trait StreamPayload: Sized {
    type Ref<'a>: Serialize
    where
        Self: 'a;

    /// Return the object as the Ref type.
    fn as_stream_ref(&self) -> Self::Ref<'_>;

    /// Rebuild a borrowed stream ref with the shorter lifetime of `stream`.
    ///
    /// Generic code cannot assume that `Self::Ref<'long>` can be used as
    /// `Self::Ref<'short>`, even when `'long: 'short`. This hook makes that
    /// lifetime-shortening operation explicit for each ref type.
    ///
    /// A mere `clone()` would preserve the same lifetime, so it can't be used
    /// directly. In other words this is just a `clone()` on the Ref type with
    /// more lifetime control.
    fn reborrow_ref<'short, 'long>(stream: &'short Self::Ref<'long>) -> Self::Ref<'short>
    where
        'long: 'short,
        Self: 'long;

    /// Create an owned type from the ref type. Data will be copied.
    fn from_stream_ref(stream: Self::Ref<'_>) -> Self;
}

impl StreamPayload for FloatStream {
    type Ref<'a> = FloatStreamRef<'a>;

    fn as_stream_ref(&self) -> Self::Ref<'_> {
        FloatStreamRef {
            name: &self.name,
            tags: &self.tags,
            samples: &self.samples,
        }
    }

    fn reborrow_ref<'short, 'long>(stream: &'short Self::Ref<'long>) -> Self::Ref<'short>
    where
        'long: 'short,
        Self: 'long,
    {
        FloatStreamRef {
            name: stream.name,
            tags: stream.tags,
            samples: stream.samples,
        }
    }

    fn from_stream_ref(stream: Self::Ref<'_>) -> Self {
        stream.to_owned()
    }
}

/// Cow-like stream payload for `WorkerToMainRef`.
///
/// Use `Borrowed` for zero-copy serialization. Deserialization always produces
/// `Owned`, since stream reference payloads contain borrowed slices.
pub enum StreamCow<'a, T: StreamPayload + 'a> {
    Borrowed(T::Ref<'a>),
    Owned(T),
}

impl<'a, T> StreamCow<'a, T>
where
    T: StreamPayload + 'a,
{
    pub fn borrowed(stream: T::Ref<'a>) -> Self {
        Self::Borrowed(stream)
    }

    pub fn owned(stream: T) -> Self {
        Self::Owned(stream)
    }

    pub fn into_owned(self) -> T {
        match self {
            Self::Borrowed(stream) => T::from_stream_ref(stream),
            Self::Owned(stream) => stream,
        }
    }
}

impl<'a, T> StreamCow<'a, T>
where
    T: StreamPayload + 'a,
{
    pub fn borrow<'short>(&'short self) -> T::Ref<'short>
    where
        'a: 'short,
    {
        match self {
            Self::Borrowed(stream) => T::reborrow_ref(stream),
            Self::Owned(stream) => stream.as_stream_ref(),
        }
    }
}

impl<'a, T> Serialize for StreamCow<'a, T>
where
    T: StreamPayload + 'a,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Borrowed(stream) => stream.serialize(serializer),
            Self::Owned(stream) => stream.as_stream_ref().serialize(serializer),
        }
    }
}

impl<'de, 'a, T> Deserialize<'de> for StreamCow<'a, T>
where
    T: StreamPayload + Deserialize<'de> + 'a,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self::Owned)
    }
}

impl<'a, T> From<T> for StreamCow<'a, T>
where
    T: StreamPayload + 'a,
{
    fn from(stream: T) -> Self {
        Self::Owned(stream)
    }
}

impl<'a> From<FloatStreamRef<'a>> for StreamCow<'a, FloatStream> {
    fn from(stream: FloatStreamRef<'a>) -> Self {
        Self::Borrowed(stream)
    }
}

pub type FloatStreamCow<'a> = StreamCow<'a, FloatStream>;

/// Stream of data between worker and main UI.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ComplexStream {
    pub name: String,
    pub tags: Vec<rustradio::stream::Tag>,
    pub samples: Vec<rustradio::Complex>,
}

/// Borrow version of `ComplexStream`.
///
/// Used to avoid copies when e.g. sending directly from a RustRadio stream.
///
/// Must serialize the same as `ComplexStream`.
#[derive(Serialize, Clone)]
pub struct ComplexStreamRef<'a> {
    pub name: &'a str,
    pub tags: &'a [rustradio::stream::Tag],
    pub samples: &'a [rustradio::Complex],
}

impl ComplexStreamRef<'_> {
    fn to_owned(&self) -> ComplexStream {
        ComplexStream {
            name: self.name.to_string(),
            tags: self.tags.to_vec(),
            samples: self.samples.to_vec(),
        }
    }
}

impl StreamPayload for ComplexStream {
    type Ref<'a> = ComplexStreamRef<'a>;

    fn as_stream_ref(&self) -> Self::Ref<'_> {
        ComplexStreamRef {
            name: &self.name,
            tags: &self.tags,
            samples: &self.samples,
        }
    }

    fn reborrow_ref<'short, 'long>(stream: &'short Self::Ref<'long>) -> Self::Ref<'short>
    where
        'long: 'short,
        Self: 'long,
    {
        ComplexStreamRef {
            name: stream.name,
            tags: stream.tags,
            samples: stream.samples,
        }
    }

    fn from_stream_ref(stream: Self::Ref<'_>) -> Self {
        stream.to_owned()
    }
}

impl<'a> From<ComplexStreamRef<'a>> for StreamCow<'a, ComplexStream> {
    fn from(stream: ComplexStreamRef<'a>) -> Self {
        Self::Borrowed(stream)
    }
}

pub type ComplexStreamCow<'a> = StreamCow<'a, ComplexStream>;

/// Stream of PDUs of floats for sending between worker and main UI.
///
/// This is used by the frequency and waterfall sinks.
///
/// There's currently no borrow version of `FloatPduStream`, since PDUs are
/// generally passed by value anyway. If a need comes up, it can be added.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, PartialOrd)]
pub struct FloatPduStream {
    pub name: String,
    pub sample_rate: rustradio::Float,
    pub samples: Vec<rustradio::Float>,
}

/// No application specific messages required.
#[derive(Debug, Serialize, Deserialize)]
pub enum AppEmpty {}

impl ApplicationSpecific for AppEmpty {
    type App = AppEmpty;
    type Start = AppEmpty;
    type Ready = AppEmpty;
    type End = AppEmpty;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    struct TestAppMessage {
        name: String,
        payload: String,
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct TestStart {
        sample_rate: u64,
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct TestReady {
        channels: u8,
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct TestAppMessageRef<'a> {
        name: &'a str,
        payload: &'a str,
    }

    #[derive(Debug)]
    struct TestApp;

    impl ApplicationSpecific for TestApp {
        type App = TestAppMessage;
        type Start = TestStart;
        type Ready = TestReady;
        type End = AppEmpty;
    }

    #[derive(Debug)]
    struct TestAppRef<'a>(std::marker::PhantomData<&'a ()>);

    impl<'a> ApplicationSpecific for TestAppRef<'a> {
        type App = TestAppMessageRef<'a>;
        type Start = TestStart;
        type Ready = TestReady;
        type End = AppEmpty;
    }

    fn expected_app_message() -> TestAppMessage {
        TestAppMessage {
            name: "test app message".to_string(),
            payload: "test payload".to_string(),
        }
    }

    fn assert_main_to_worker_app_message(msg: MainToWorker<TestApp>, expected: &TestAppMessage) {
        match msg {
            MainToWorker::ApplicationSpecific(app) => assert_eq!(app, *expected),
            other => panic!("expected MainToWorker::ApplicationSpecific, got {other:?}"),
        }
    }

    fn assert_worker_to_main_app_message(msg: WorkerToMain<TestApp>, expected: &TestAppMessage) {
        match msg {
            WorkerToMain::ApplicationSpecific(app) => assert_eq!(app, *expected),
            other => panic!("expected WorkerToMain::ApplicationSpecific, got {other:?}"),
        }
    }

    fn assert_main_to_worker_ref_app_message(
        msg: MainToWorker<TestAppRef<'_>>,
        expected: &TestAppMessage,
    ) {
        match msg {
            MainToWorker::ApplicationSpecific(app) => {
                assert_eq!(app.name, expected.name);
                assert_eq!(app.payload, expected.payload);
            }
            other => panic!("expected MainToWorker::ApplicationSpecific, got {other:?}"),
        }
    }

    fn assert_worker_to_main_ref_app_message(
        msg: WorkerToMainRef<'_, TestAppRef<'_>>,
        expected: &TestAppMessage,
    ) {
        match msg {
            WorkerToMainRef::ApplicationSpecific(app) => {
                assert_eq!(app.name, expected.name);
                assert_eq!(app.payload, expected.payload);
            }
            _ => panic!("expected WorkerToMainRef::ApplicationSpecific"),
        }
    }

    fn expected_float_stream() -> FloatStream {
        FloatStream {
            name: "float stream".to_string(),
            tags: Vec::new(),
            samples: vec![1.25, -2.5, 3.75],
        }
    }

    fn expected_complex_stream() -> ComplexStream {
        ComplexStream {
            name: "complex stream".to_string(),
            tags: Vec::new(),
            samples: vec![
                rustradio::Complex::new(1.0, -2.0),
                rustradio::Complex::new(-3.5, 4.25),
            ],
        }
    }

    fn reborrow_float_stream<'short, 'long>(
        stream: &'short FloatStreamCow<'long>,
    ) -> FloatStreamRef<'short>
    where
        'long: 'short,
    {
        stream.borrow()
    }

    struct TestCowStream {
        value: u8,
    }

    #[derive(Clone)]
    struct TestCowStreamRef<'a> {
        value: &'a u8,
    }

    impl serde::Serialize for TestCowStream {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            serializer.serialize_str("owned")
        }
    }

    impl serde::Serialize for TestCowStreamRef<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            serializer.serialize_str("ref")
        }
    }

    impl StreamPayload for TestCowStream {
        type Ref<'a> = TestCowStreamRef<'a>;

        fn as_stream_ref(&self) -> Self::Ref<'_> {
            TestCowStreamRef { value: &self.value }
        }

        fn reborrow_ref<'short, 'long>(stream: &'short Self::Ref<'long>) -> Self::Ref<'short>
        where
            'long: 'short,
            Self: 'long,
        {
            TestCowStreamRef {
                value: stream.value,
            }
        }

        fn from_stream_ref(stream: Self::Ref<'_>) -> Self {
            TestCowStream {
                value: *stream.value,
            }
        }
    }

    #[test]
    fn application_specific_main_to_worker_serializes_between_owned_and_ref_payloads() {
        let expected = expected_app_message();

        let owned_json = serde_json::to_value(MainToWorker::<TestApp>::ApplicationSpecific(
            expected.clone(),
        ))
        .unwrap();
        let ref_json = serde_json::to_value(MainToWorker::<TestAppRef<'_>>::ApplicationSpecific(
            TestAppMessageRef {
                name: "test app message",
                payload: "test payload",
            },
        ))
        .unwrap();

        assert_eq!(owned_json, ref_json);

        let decoded: MainToWorker<TestApp> = serde_json::from_value(ref_json).unwrap();
        assert_main_to_worker_app_message(decoded, &expected);
    }

    #[test]
    fn application_specific_worker_to_main_serializes_between_owned_and_ref_payloads() {
        let expected = expected_app_message();

        let owned_json = serde_json::to_value(WorkerToMain::<TestApp>::ApplicationSpecific(
            expected.clone(),
        ))
        .unwrap();
        let ref_json = serde_json::to_value(
            WorkerToMainRef::<TestAppRef<'_>>::ApplicationSpecific(TestAppMessageRef {
                name: "test app message",
                payload: "test payload",
            }),
        )
        .unwrap();

        assert_eq!(owned_json, ref_json);

        let decoded: WorkerToMain<TestApp> = serde_json::from_value(ref_json).unwrap();
        assert_worker_to_main_app_message(decoded, &expected);
    }

    #[test]
    fn application_specific_main_to_worker_deserializes_from_json_into_ref_payload() {
        let expected = expected_app_message();
        let json = serde_json::to_string(&MainToWorker::<TestApp>::ApplicationSpecific(
            expected.clone(),
        ))
        .unwrap();

        let decoded: MainToWorker<TestAppRef<'_>> = serde_json::from_str(&json).unwrap();

        assert_main_to_worker_ref_app_message(decoded, &expected);
    }

    #[test]
    fn application_specific_worker_to_main_deserializes_from_json_into_ref_payload() {
        let expected = expected_app_message();
        let json = serde_json::to_string(&WorkerToMain::<TestApp>::ApplicationSpecific(
            expected.clone(),
        ))
        .unwrap();

        let decoded: WorkerToMainRef<'_, TestAppRef<'_>> = serde_json::from_str(&json).unwrap();

        assert_worker_to_main_ref_app_message(decoded, &expected);
    }

    #[test]
    fn stream_cow_owned_variant_serializes_through_ref_type() {
        let json = serde_json::to_value(StreamCow::owned(TestCowStream { value: 7 })).unwrap();

        assert_eq!(json, serde_json::json!("ref"));
    }

    #[test]
    fn stream_cow_borrow_reborrows_borrowed_variant() {
        let tags = Vec::new();
        let samples = [1.0, 2.0];
        let stream = FloatStreamCow::borrowed(FloatStreamRef {
            name: "float stream",
            tags: &tags,
            samples: &samples,
        });

        let borrowed = reborrow_float_stream(&stream);

        assert_eq!(borrowed.name, "float stream");
        assert_eq!(borrowed.tags, tags.as_slice());
        assert_eq!(borrowed.samples, samples.as_slice());
    }

    #[test]
    fn worker_to_main_ref_float_streams_serialize_like_owned_streams() {
        let expected = expected_float_stream();
        let samples = expected.samples.clone();

        let owned_json = serde_json::to_value(WorkerToMain::<AppEmpty>::FloatStreams(vec![
            expected.clone(),
        ]))
        .unwrap();
        let ref_json = serde_json::to_value(WorkerToMainRef::<AppEmpty>::FloatStreams(vec![
            FloatStreamCow::Borrowed(FloatStreamRef {
                name: &expected.name,
                tags: &expected.tags,
                samples: &samples,
            }),
        ]))
        .unwrap();

        assert_eq!(owned_json, ref_json);
    }

    #[test]
    fn worker_to_main_ref_float_streams_deserialize_as_owned_streams() {
        let expected = expected_float_stream();
        let json = serde_json::to_string(&WorkerToMain::<AppEmpty>::FloatStreams(vec![
            expected.clone(),
        ]))
        .unwrap();

        let decoded: WorkerToMainRef<'_, AppEmpty> = serde_json::from_str(&json).unwrap();

        match decoded {
            WorkerToMainRef::FloatStreams(mut streams) => {
                assert_eq!(streams.len(), 1);
                match streams.remove(0) {
                    FloatStreamCow::Owned(stream) => assert_eq!(stream, expected),
                    FloatStreamCow::Borrowed(_) => panic!("expected owned float stream"),
                }
            }
            _ => panic!("expected WorkerToMainRef::FloatStreams"),
        }
    }

    #[test]
    fn worker_to_main_ref_complex_streams_serialize_like_owned_streams() {
        let expected = expected_complex_stream();
        let samples = expected.samples.clone();

        let owned_json = serde_json::to_value(WorkerToMain::<AppEmpty>::ComplexStreams(vec![
            expected.clone(),
        ]))
        .unwrap();
        let ref_json = serde_json::to_value(WorkerToMainRef::<AppEmpty>::ComplexStreams(vec![
            ComplexStreamCow::Borrowed(ComplexStreamRef {
                name: &expected.name,
                tags: &expected.tags,
                samples: &samples,
            }),
        ]))
        .unwrap();

        assert_eq!(owned_json, ref_json);
    }

    #[test]
    fn worker_to_main_ref_complex_streams_deserialize_as_owned_streams() {
        let expected = expected_complex_stream();
        let json = serde_json::to_string(&WorkerToMain::<AppEmpty>::ComplexStreams(vec![
            expected.clone(),
        ]))
        .unwrap();

        let decoded: WorkerToMainRef<'_, AppEmpty> = serde_json::from_str(&json).unwrap();

        match decoded {
            WorkerToMainRef::ComplexStreams(mut streams) => {
                assert_eq!(streams.len(), 1);
                match streams.remove(0) {
                    ComplexStreamCow::Owned(stream) => assert_eq!(stream, expected),
                    ComplexStreamCow::Borrowed(_) => panic!("expected owned complex stream"),
                }
            }
            _ => panic!("expected WorkerToMainRef::ComplexStreams"),
        }
    }

    #[test]
    fn shared_vec_lifetime() {
        assert!(PENDING_SHARED_BUFFERS_U8.with(|slot| slot.borrow().is_empty()));
        let buf: Vec<u8> = Vec::new();
        let shared_vec_ptr = SharedVecPtr::new(buf, vec![]);
        assert_eq!(
            PENDING_SHARED_BUFFERS_U8.with(|slot| slot.borrow().len()),
            1
        );
        let shared_vec = SharedVec::new(shared_vec_ptr, |id| {
            u8::registry_remove(id);
            Ok(())
        });
        assert_eq!(
            PENDING_SHARED_BUFFERS_U8.with(|slot| slot.borrow().len()),
            1
        );
        drop(shared_vec);
        assert!(PENDING_SHARED_BUFFERS_U8.with(|slot| slot.borrow().is_empty()));
    }
}
