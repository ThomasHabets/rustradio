#![doc = include_str!("../README.md")]
use std::fmt::Debug;

use serde::{Deserialize, Serialize};

use rustradio::{Complex, Float, stream::Tag};

#[cfg(feature = "audio")]
pub mod browser_audio;

pub mod dom_logger;
pub mod mainthread;
mod start_worker;
pub mod worker;

/// Application specific extensions to MainToWorker and WorkerToMain.
///
/// When not applicable, set a type to [`AppEmpty`].
pub trait ApplicationSpecific: Debug + Send + 'static {
    // Can't default. https://github.com/rust-lang/rust/issues/29661
    type App: Serialize + Debug + for<'de> Deserialize<'de>;
    type Start: Serialize + Debug + for<'de> Deserialize<'de>;
    type Ready: Serialize + Debug + for<'de> Deserialize<'de>;
    type End: Serialize + Debug + for<'de> Deserialize<'de>;
}

/// Bootstrap MPSC message. Should not be used directly by applications, and
/// ideally should be private to this crate.
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

    /// Worker requests some more data on the named stream.
    RequestData(String, usize),

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

/// No application specific messages required.
#[derive(Debug, Serialize, Deserialize)]
pub enum AppEmpty {}

impl ApplicationSpecific for AppEmpty {
    type App = AppEmpty;
    type Start = AppEmpty;
    type Ready = AppEmpty;
    type End = AppEmpty;
}
