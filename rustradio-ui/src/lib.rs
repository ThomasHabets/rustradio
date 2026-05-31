#![doc = include_str!("../README.md")]
use serde::{Deserialize, Serialize};

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

/// Cow-like float stream payload for `WorkerToMainRef`.
///
/// Use `Borrowed` for zero-copy serialization. Deserialization always produces
/// `Owned`, since `FloatStreamRef` contains borrowed slices.
#[derive(Serialize)]
#[serde(untagged)]
pub enum FloatStreamCow<'a> {
    Borrowed(FloatStreamRef<'a>),
    Owned(FloatStream),
}

impl<'a> FloatStreamCow<'a> {
    pub fn into_owned(self) -> FloatStream {
        match self {
            Self::Borrowed(stream) => FloatStream {
                name: stream.name.to_owned(),
                tags: stream.tags.to_vec(),
                samples: stream.samples.to_vec(),
            },
            Self::Owned(stream) => stream,
        }
    }
    pub fn borrow(&self) -> FloatStreamRef<'_> {
        match self {
            Self::Borrowed(stream) => stream.clone(),
            Self::Owned(stream) => FloatStreamRef {
                name: &stream.name,
                tags: &stream.tags,
                samples: &stream.samples,
            },
        }
    }
}

impl<'de, 'a> Deserialize<'de> for FloatStreamCow<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        FloatStream::deserialize(deserializer).map(Self::Owned)
    }
}

impl<'a> From<FloatStreamRef<'a>> for FloatStreamCow<'a> {
    fn from(stream: FloatStreamRef<'a>) -> Self {
        Self::Borrowed(stream)
    }
}

impl<'a> From<FloatStream> for FloatStreamCow<'a> {
    fn from(stream: FloatStream) -> Self {
        Self::Owned(stream)
    }
}

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
#[derive(Serialize)]
pub struct ComplexStreamRef<'a> {
    pub name: &'a str,
    pub tags: Vec<rustradio::stream::Tag>,
    pub samples: &'a [rustradio::Complex],
}

/// Cow-like complex stream payload for `WorkerToMainRef`.
///
/// Use `Borrowed` for zero-copy serialization. Deserialization always produces
/// `Owned`, since `ComplexStreamRef` contains borrowed slices.
#[derive(Serialize)]
#[serde(untagged)]
pub enum ComplexStreamCow<'a> {
    Borrowed(ComplexStreamRef<'a>),
    Owned(ComplexStream),
}

impl<'a> ComplexStreamCow<'a> {
    pub fn into_owned(self) -> ComplexStream {
        match self {
            Self::Borrowed(stream) => ComplexStream {
                name: stream.name.to_owned(),
                tags: stream.tags,
                samples: stream.samples.to_vec(),
            },
            Self::Owned(stream) => stream,
        }
    }
}

impl<'de, 'a> Deserialize<'de> for ComplexStreamCow<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        ComplexStream::deserialize(deserializer).map(Self::Owned)
    }
}

impl<'a> From<ComplexStreamRef<'a>> for ComplexStreamCow<'a> {
    fn from(stream: ComplexStreamRef<'a>) -> Self {
        Self::Borrowed(stream)
    }
}

impl<'a> From<ComplexStream> for ComplexStreamCow<'a> {
    fn from(stream: ComplexStream) -> Self {
        Self::Owned(stream)
    }
}

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
                tags: Vec::new(),
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
}
