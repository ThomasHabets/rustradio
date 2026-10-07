#![cfg(feature = "unstable")]

use wasm_bindgen::prelude::*;

mod mainthread;
mod worker;

/// Connection settings are passed once to the worker. The server supplies the
/// sample rate and encoding; the viewer never changes either during a session.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Connection {
    url: String,
    source: String,
    allow_gaps: bool,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum AppMessage {
    Stop,
    Connected { sample_rate: f32, source: String },
}
impl rustradio_ui::ApplicationSpecific for AppMessage {
    type App = Self;
    type Start = Connection;
    type End = String;
    type Ready = rustradio_ui::AppEmpty;
}
type MainToWorker = rustradio_ui::MainToWorker<AppMessage>;
type WorkerToMain = rustradio_ui::WorkerToMain<AppMessage>;

#[wasm_bindgen]
pub async fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    if web_sys::window().is_none() {
        worker::setup().await
    } else {
        rustradio_ui::dom_logger::init_logging::<AppMessage>("log-output", log::LevelFilter::Info)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        mainthread::setup()
    }
}
