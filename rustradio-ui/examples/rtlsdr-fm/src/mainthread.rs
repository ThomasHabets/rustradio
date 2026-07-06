use log::info;
use wasm_bindgen::prelude::*;

use crate::{MyMainToWorker, MyWorkerToMain, WorkerToMain};

async fn worker_msg(msg: WorkerToMain) -> Result<(), JsValue> {
    match msg {
        WorkerToMain::LogLine { .. } => {}
        ref _other => info!("Got worker msg: {msg:?}"),
    }
    Ok(())
}

pub(crate) async fn setup() -> Result<(), JsValue> {
    rustradio_ui::mainthread::start_worker::<MyMainToWorker, MyWorkerToMain, _, _>(worker_msg);
    Ok(())
}
