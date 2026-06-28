#![doc = include_str!("../README.md")]
use serde::{Deserialize, Serialize};

use rustradio::{Complex, Float, stream::Tag};

/// Application specific extensions to MainToWorker and WorkerToMain.
pub trait ApplicationSpecific {
    // Can't default. https://github.com/rust-lang/rust/issues/29661
    type App: Serialize;
    type Start: Serialize;
    type Ready: Serialize;
    type End: Serialize;
}

#[derive(Debug)]
pub struct BootstrapMpsc<App1: ApplicationSpecific, App2: ApplicationSpecific> {
    pub rx: async_channel::Receiver<MainToWorker<App1>>,
    pub tx: async_channel::Sender<WorkerToMain<App2>>,
}

impl<App1: ApplicationSpecific, App2: ApplicationSpecific> BootstrapMpsc<App1, App2> {
    // TODO: make this pub(crate).
    pub fn from_ptr(ptr: usize) -> Self {
        let boot = unsafe { Box::from_raw(ptr as *mut Self) };
        *boot
    }
}

/// Messages going from main (UI) thread to worker.
///
/// These can be sent as worker messages (a bunch of copies and serde), or
/// directly on an MPSC.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(bound(
    serialize = "App::App: Serialize, App::Start: Serialize",
    deserialize = "App::App: Deserialize<'de>, App::Start: Deserialize<'de>",
))]
#[non_exhaustive]
pub enum MainToWorker<App: ApplicationSpecific> {
    /// A boxed pointer to BootstrapMpsc.
    ///
    /// Applications should not use this directly.
    BootstrapMpsc(usize),

    /// Start the graph with the relevant startup parameters.
    Start(App::Start),

    /// Application specific stuff.
    ApplicationSpecific(App::App),

    /// This is useful for sending RTL-SDR raw bytes from the main UI thread
    /// (which gets it from WebUSB) to the worker thread.
    Bytes(String, Vec<TaggedVec<u8>>),

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

    /// This is used for streams as well as for fixed block sized things like
    /// spectrums and waterfalls.
    Floats(String, Vec<TaggedVec<Float>>),

    Complexes(String, Vec<TaggedVec<Complex>>),

    /// At the end of execution, provide the result as a string.
    End(App::End),

    /// A worker log line to be emitted through the main thread logger.
    LogLine {
        level: log::Level,
        line: String,
    },
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

/// Data and tags. Used to send from worker to the UI for display.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct TaggedVec<T> {
    pub data: Vec<T>,
    pub tags: Vec<Tag>,
}

/// Stream of floats going between worker and UI thread.
// TODO: remove this now? If not, then at least make it use TaggedVec?
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
}
