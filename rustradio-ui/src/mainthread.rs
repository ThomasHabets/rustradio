use std::any::Any;
use std::cell::{OnceCell, RefCell};

use async_channel::{Receiver, Sender};
use log::{error, info, warn};
use serde::Serialize;
use wasm_bindgen::prelude::{Closure, JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{MessageEvent, Worker};

use crate::ApplicationSpecific;
use crate::{MainToWorker, WorkerToMain};

pub mod constellation_sink;
pub mod spectrum_sink;
pub mod time_sink;
mod worker_startup;

pub(crate) const CLASS_SINK: &str = "rr-sink-section";

thread_local! {
    static WORKER: OnceCell<Worker> = const { OnceCell::new() };
    static WORKER_TX: RefCell<Option<Box<dyn Any>>> = const { RefCell::new(None)} ;
}

/// Post a message using the slower message passing method.
pub fn post_message<T: Serialize + ?Sized>(msg: &T) -> rustradio::Result<()> {
    let msg: JsValue = serde_wasm_bindgen::to_value(msg)
        .map_err(|e| rustradio::Error::msg(format!("JS error serializing: {e:?}")))?;
    worker()
        .post_message(&msg)
        .map_err(|e| rustradio::Error::msg(format!("JS error: {e:?}")))
}

async fn with_worker_tx<App, R>(f: impl FnOnce(&Sender<MainToWorker<App>>) -> R) -> Option<R>
where
    App: ApplicationSpecific + 'static,
{
    WORKER_TX.with(|slot| {
        let slot = slot.borrow();

        let tx = slot.as_ref()?.downcast_ref::<Sender<MainToWorker<App>>>()?;

        Some(f(tx))
    })
}

/// Send a message to the main UI thread via the fast mpsc channel.
pub async fn send_message<App>(msg: MainToWorker<App>) -> rustradio::Result<()>
where
    App: ApplicationSpecific + 'static,
{
    let inited = WORKER_TX.with(|slot| slot.borrow().is_some());
    if inited {
        let tx = with_worker_tx::<App, _>(Clone::clone)
        .await
        .ok_or_else(|| {
            rustradio::Error::msg("MAIN_UI_TX has wrong type or was not initialized")
        })?;
        tx.send(msg)
            .await
            .map_err(|e| rustradio::Error::msg(format!("worker channel is closed: {e}")))
    } else {
        error!("Tried to send before worker channel set up. Falling back to posting");
        post_message(&msg)
    }
}

/// Send a message to the main UI thread via the fast mpsc channel, from sync
/// code.
pub fn send_message_sync<App>(msg: MainToWorker<App>) -> rustradio::Result<()>
where
    App: ApplicationSpecific + 'static,
{
    spawn_local(async move {
        if let Err(e) = send_message(msg).await {
            error!("Failed to send message: {e:?}");
        }
    });
    Ok(())
}

/// Get HTML button by ID.
pub fn get_button(id: &str) -> Result<web_sys::HtmlButtonElement, JsValue> {
    Ok(get_element(id)?.dyn_into()?)
}

/// Get HTML input element by ID.
pub fn get_input(id: &str) -> Result<web_sys::HtmlInputElement, JsValue> {
    Ok(get_element(id)?.dyn_into()?)
}

/// Get HTML element by ID.
pub fn get_element(id: &str) -> Result<web_sys::Element, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("can't find element with id {id}")))
}

/// Start the worker.
///
/// This should be called by the main UI thread. Likely it should be called in
/// the setup function. It only starts the worker, not the rustradio graph.
pub fn start_worker<AppMain, AppWorker, F, Ret>(worker_msg: F) -> Worker
where
    for<'de> AppWorker: crate::ApplicationSpecific + serde::Deserialize<'de>,
    for<'de> AppMain: crate::ApplicationSpecific + serde::Deserialize<'de>,
    F: Fn(WorkerToMain<AppWorker>) -> Ret + Copy + 'static,
    Ret: Future<Output = Result<(), JsValue>>,
{
    WORKER.with(|cell| {
        cell.get_or_init(|| {
            info!("Main: Starting the worker");
            let opts = web_sys::WorkerOptions::new();
            opts.set_type(web_sys::WorkerType::Module);
            opts.set_name("RustRadio worker");
            let worker = Worker::new_with_options("./wasm-mod.js", &opts).unwrap();
            let mut bootstrapped = false;

            // Set message handler.
            // Cloning a worker handle is cheap.
            let handler_worker = worker.clone();
            let onmessage = Closure::<dyn FnMut(MessageEvent) -> Result<(), JsValue>>::new(
                move |e: MessageEvent| {
                    let worker_msg = worker_msg;
                    if !bootstrapped {
                        // Cloning a worker handle is cheap.
                        bootstrapped = worker_startup::msg(handler_worker.clone(), &e);

                        if bootstrapped {
                            info!("Worker bootstrap done");
                            let (wtx, mrx): (
                                Sender<WorkerToMain<AppWorker>>,
                                Receiver<WorkerToMain<AppWorker>>,
                            ) = async_channel::bounded(10);
                            let (mtx, wrx): (
                                Sender<MainToWorker<AppMain>>,
                                Receiver<MainToWorker<AppMain>>,
                            ) = async_channel::bounded(10);
                            let b = Box::new(crate::BootstrapMpsc { rx: wrx, tx: wtx });
                            post_message(&MainToWorker::<AppMain>::BootstrapMpsc(
                                Box::into_raw(b) as usize
                            ))
                            .unwrap();
                            WORKER_TX.with(|slot| *slot.borrow_mut() = Some(Box::new(mtx)));
                            spawn_local(async move {
                                while let Ok(msg) = mrx.recv().await {
                                    //info!("Received message {msg:?}");
                                    if let Err(e) = worker_msg(msg).await {
                                        error!("Error handling send message: {e:?}");
                                    }
                                }
                            });
                        }
                        return Ok(());
                    }
                    spawn_local(async move {
                        match e.data().try_into() {
                            Ok(msg) => {
                                match &msg {
                                    WorkerToMain::LogLine { level, line } => {
                                        log::log!(*level, "[worker] {line}");
                                    }
                                    _other => warn!("Main thread received posted {msg:?}"),
                                }
                                if let Err(e) = worker_msg(msg).await {
                                    error!("Main: Inner receiver thing: {e:?}");
                                }
                            }
                            Err(err) => {
                                error!("Failed to deserialize posted message {e:?}: {err:?}");
                            }
                        }
                    });
                    Ok(())
                },
            );
            worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            onmessage.forget();

            // Set messageerror handler.
            let onmsgerr = Closure::<dyn FnMut(MessageEvent) -> Result<(), JsValue>>::new(
                move |e: MessageEvent| {
                    error!("Main: Message Error: {e:?}");
                    Ok(())
                },
            );
            worker.set_onmessageerror(Some(onmsgerr.as_ref().unchecked_ref()));
            onmsgerr.forget();

            // Set error handler.
            let onerr = Closure::<dyn FnMut(MessageEvent) -> Result<(), JsValue>>::new(
                move |e: MessageEvent| {
                    error!("Main: Worker error: {e:?}");
                    Ok(())
                },
            );
            worker.set_onerror(Some(onerr.as_ref().unchecked_ref()));
            onerr.forget();

            worker
        })
        .clone()
    })
}

fn worker() -> Worker {
    WORKER.with(|cell| cell.get().unwrap().clone())
}
