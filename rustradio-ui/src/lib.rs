#![doc = include_str!("../README.md")]
use std::any::Any;
use std::fmt::Debug;

use log::error;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;

use rustradio::{Complex, Float, stream::Tag};

#[cfg(feature = "audio")]
pub mod browser_audio;

pub mod dom_logger;
pub mod mainthread;
pub mod worker;

/// Application specific extensions to MainToWorker and WorkerToMain.
///
/// When not applicable, set a type to [`AppEmpty`].
/// Payload types must be `Send` because channels transfer them between the
/// main thread and worker through shared WASM memory.
pub trait ApplicationSpecific: Debug + Send + 'static {
    // Can't default. https://github.com/rust-lang/rust/issues/29661
    type App: Serialize + Debug + Send + for<'de> Deserialize<'de>;
    type Start: Serialize + Debug + Send + for<'de> Deserialize<'de>;
    type Ready: Serialize + Debug + Send + for<'de> Deserialize<'de>;
    type End: Serialize + Debug + Send + for<'de> Deserialize<'de>;
}

/// Channel endpoints transferred from the main thread to the worker.
#[derive(Debug)]
struct BootstrapMpsc<App1: ApplicationSpecific, App2: ApplicationSpecific> {
    pub rx: async_channel::Receiver<MainToWorker<App1>>,
    pub tx: async_channel::Sender<WorkerToMain<App2>>,
}

/// All bootstrap pointers address the same envelope layout. The endpoints
/// retain their concrete type so the worker can check it before taking them.
struct BootstrapEnvelope {
    endpoints: Box<dyn Any + Send>,
}

impl<App1: ApplicationSpecific, App2: ApplicationSpecific> BootstrapMpsc<App1, App2> {
    /// Transfer ownership through a pointer to a non-generic envelope.
    fn into_ptr(self) -> usize {
        Box::into_raw(Box::new(BootstrapEnvelope {
            endpoints: Box::new(self),
        })) as usize
    }

    /// Claim the endpoints after checking the main and worker channel types.
    /// Mismatched endpoints are dropped using their original concrete type.
    ///
    /// # Safety
    ///
    /// `ptr` must come from `BootstrapMpsc::into_ptr` in the same address space
    /// and identify a live, unclaimed allocation. WASM threads must share the
    /// same module and linear memory.
    /// Both success and failure consume it, so it must never be claimed again.
    unsafe fn from_ptr(ptr: usize) -> rustradio::Result<Self> {
        // SAFETY: every into_ptr allocation has this non-generic layout.
        // The caller guarantees that the allocation is live and unclaimed.
        let boot = unsafe { Box::from_raw(ptr as *mut BootstrapEnvelope) };
        // Any::downcast checks TypeId before interpreting the endpoints as
        // this worker's pair of application-specific channel types.
        boot.endpoints
            .downcast::<Self>()
            .map(|endpoints| *endpoints)
            .map_err(|_| rustradio::Error::msg("bootstrap channel types do not match worker setup"))
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
    /// A pointer to an owned, type-checked bootstrap envelope.
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

    /// Send complex sample streams from the main UI thread to the worker.
    Complexes(String, Vec<TaggedVec<Complex>>),

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
///
/// Tag positions are relative to the start of `data` and must be sorted in
/// nondecreasing order. Equal-position tags preserve their supplied order.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct TaggedVec<T> {
    pub data: Vec<T>,
    pub tags: Vec<Tag>,
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

/// Like `spawn_local`, but logs errors.
pub fn spawn<F>(f: F)
where
    F: std::future::Future<Output = Result<(), JsValue>> + 'static,
{
    spawn_local(async {
        if let Err(e) = f.await {
            error!("{e:?}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    struct LargeMessages;

    impl ApplicationSpecific for LargeMessages {
        type App = [u64; 32];
        type Start = [u64; 32];
        type Ready = [u64; 32];
        type End = [u64; 32];
    }

    #[test]
    fn bootstrap_matching_types_transfer_endpoints() {
        let (main_tx, worker_rx) = async_channel::bounded(1);
        let (worker_tx, main_rx) = async_channel::bounded(1);
        let ptr = BootstrapMpsc::<AppEmpty, LargeMessages> {
            rx: worker_rx,
            tx: worker_tx,
        }
        .into_ptr();
        main_tx.try_send(MainToWorker::Ping(12.0)).unwrap();
        std::thread::spawn(move || {
            // SAFETY: ptr is a live allocation from into_ptr, claimed once.
            let endpoints =
                unsafe { BootstrapMpsc::<AppEmpty, LargeMessages>::from_ptr(ptr) }.unwrap();
            assert!(matches!(
                endpoints.rx.try_recv(),
                Ok(MainToWorker::Ping(12.0))
            ));
            endpoints.tx.try_send(WorkerToMain::Ready([7; 32])).unwrap();
        })
        .join()
        .unwrap();
        let WorkerToMain::Ready(payload) = main_rx.try_recv().unwrap() else {
            panic!("expected ready message");
        };
        assert_eq!(payload, [7; 32]);
    }

    /// Mismatches must release the actual endpoints without interpreting them
    /// as the worker's expected message representations.
    fn assert_bootstrap_mismatch<Main: ApplicationSpecific, Worker: ApplicationSpecific>() {
        let (main_tx, worker_rx) = async_channel::bounded(1);
        let (worker_tx, main_rx) = async_channel::bounded(1);
        let ptr = BootstrapMpsc::<AppEmpty, AppEmpty> {
            rx: worker_rx,
            tx: worker_tx,
        }
        .into_ptr();
        // SAFETY: ptr is live and unclaimed. The expected channel types may
        // differ; from_ptr checks them before accessing the endpoints.
        let result = unsafe { BootstrapMpsc::<Main, Worker>::from_ptr(ptr) };
        let err = result.expect_err("mismatched bootstrap must fail");
        assert!(
            err.to_string()
                .contains("bootstrap channel types do not match worker setup")
        );
        assert!(main_tx.is_closed());
        assert!(main_rx.is_closed());
    }

    #[test]
    fn bootstrap_rejects_mismatched_main_types() {
        assert_bootstrap_mismatch::<LargeMessages, AppEmpty>();
    }

    #[test]
    fn bootstrap_rejects_mismatched_worker_types() {
        assert_bootstrap_mismatch::<AppEmpty, LargeMessages>();
    }

    #[test]
    fn main_to_worker_complexes_round_trip() {
        let message = MainToWorker::<AppEmpty>::Complexes(
            "receiver".into(),
            vec![TaggedVec {
                data: vec![Complex::new(1.0, -0.5)],
                tags: vec![],
            }],
        );

        let json = serde_json::to_string(&message).unwrap();
        let decoded: MainToWorker<AppEmpty> = serde_json::from_str(&json).unwrap();
        let MainToWorker::Complexes(name, streams) = decoded else {
            panic!("decoded the wrong MainToWorker variant");
        };
        assert_eq!(name, "receiver");
        assert_eq!(
            streams,
            vec![TaggedVec {
                data: vec![Complex::new(1.0, -0.5)],
                tags: vec![],
            }]
        );
    }
}
