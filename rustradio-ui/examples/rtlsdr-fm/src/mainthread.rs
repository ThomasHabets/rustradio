use log::info;
use wasm_bindgen::prelude::*;

use rustradio_ui::AppEmpty;
use rustradio_ui::mainthread::{get_button, send_message_sync};

use crate::{MainToWorker, MyMainToWorker, MyWorkerToMain, WorkerToMain};

const ID_START: &str = "button-start";

async fn worker_msg(msg: WorkerToMain) -> Result<(), JsValue> {
    match msg {
        WorkerToMain::LogLine { .. } => {}
        WorkerToMain::Ready(_) => {
            info!("Worker says it's ready");
            let btn = get_button(ID_START)?;
            btn.set_disabled(false);
        }
        ref _other => info!("Got worker msg: {msg:?}"),
    }
    Ok(())
}

fn handle_start() -> Result<(), JsValue> {
    Ok(send_message_sync(MainToWorker::Start(AppEmpty {}))?)
}

pub(crate) async fn setup() -> Result<(), JsValue> {
    {
        let handler = Closure::<dyn FnMut() -> Result<(), JsValue>>::new(handle_start);
        let btn = get_button(ID_START)?;
        btn.add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    rustradio_ui::mainthread::start_worker::<MyMainToWorker, MyWorkerToMain, _, _>(worker_msg);
    Ok(())
}
