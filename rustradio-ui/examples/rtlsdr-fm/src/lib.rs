use log::info;
use wasm_bindgen::prelude::*;

mod mainthread;
mod worker;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum MyWorkerToMain {}

impl rustradio_ui::ApplicationSpecific for MyWorkerToMain {
    type App = rustradio_ui::AppEmpty;
    type Start = rustradio_ui::AppEmpty;
    type End = rustradio_ui::AppEmpty;
    type Ready = rustradio_ui::AppEmpty;
}

pub(crate) type WorkerToMain = rustradio_ui::WorkerToMain<MyWorkerToMain>;
pub(crate) type MainToWorker = rustradio_ui::MainToWorker<rustradio_ui::AppEmpty>;

#[wasm_bindgen]
pub async fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    if web_sys::window().is_none() {
        info!("Worker: Starting");
        worker::setup().await
    } else {
        rustradio_ui::dom_logger::init_logging::<MyWorkerToMain>(
            mainthread::ID_LOG_OUTPUT,
            log::LevelFilter::Info,
        )
        .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
        info!("Main UI: Starting");
        mainthread::setup().await
    }
}
