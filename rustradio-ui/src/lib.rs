//! Rustradio UI library, running as WASM in the browser.
//!
//! This code is in the process of being migrated / cleaned up from
//! <https://github.com/ThomasHabets/ruwasm>.
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
/// While this could potentially be used for deserialization, it's mainly for
/// avoiding a copy while serializing.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(bound(
    serialize = "App::App: Serialize, App::Ready: Serialize",
    deserialize = "App::App: Deserialize<'de>, App::Ready: Deserialize<'de>",
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
    Result(&'a str),

    /// A worker log line to be emitted through the main thread logger.
    LogLine { level: log::Level, line: &'a str },

    #[serde(skip_deserializing)]
    FloatStreams(Vec<FloatStreamRef<'a>>),

    #[serde(skip_deserializing)]
    ComplexStreams(Vec<ComplexStreamRef<'a>>),
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
#[derive(Serialize)]
pub struct FloatStreamRef<'a> {
    pub name: &'a str,
    pub tags: Vec<rustradio::stream::Tag>,
    pub samples: &'a [rustradio::Float],
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
pub struct AppEmpty {}

impl ApplicationSpecific for AppEmpty {
    type App = AppEmpty;
    type Start = AppEmpty;
    type Ready = AppEmpty;
    type End = AppEmpty;
}
