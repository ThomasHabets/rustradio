use std::any::Any;
use std::cell::RefCell;

use async_channel::Sender;
use log::error;
use serde::Serialize;
use wasm_bindgen::prelude::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::DedicatedWorkerGlobalScope;

use crate::{ApplicationSpecific, WorkerToMain};

thread_local! {
    // TODO: This should be global for all worker threads.
    static MAIN_UI_TX: RefCell<Option<Box<dyn Any>>> = const { RefCell::new(None)} ;
}

pub mod complex_sink;
pub mod float_pdu_sink;
pub mod float_sink;

pub use complex_sink::ComplexSink;
pub use float_pdu_sink::FloatPduSink;
pub use float_sink::FloatSink;

/// Store the fast mpsc-based communication channel to the main UI.
pub fn set_main_ui_tx<App>(tx: Sender<WorkerToMain<App>>)
where
    App: ApplicationSpecific + 'static,
{
    MAIN_UI_TX.with(|slot| *slot.borrow_mut() = Some(Box::new(tx)));
}

/// Post a message to the main UI thread using the slower worker message method.
pub fn post_message<T: Serialize + ?Sized>(msg: &T) -> rustradio::Result<()> {
    let msg = serde_wasm_bindgen::to_value(msg)
        .map_err(|e| rustradio::Error::msg(format!("JS error serializing: {e:?}")))?;
    let scope = web_sys::js_sys::global()
        .dyn_into::<DedicatedWorkerGlobalScope>()
        .map_err(|e| rustradio::Error::msg(format!("JS error getting worker scope: {e:?}")))?;
    scope
        .post_message(&msg)
        .map_err(|e| rustradio::Error::msg(format!("JS error: {e:?}")))
}

async fn with_main_ui_tx<App, R>(f: impl FnOnce(&Sender<WorkerToMain<App>>) -> R) -> Option<R>
where
    App: ApplicationSpecific + 'static,
{
    MAIN_UI_TX.with(|slot| {
        let slot = slot.borrow();

        let tx = slot.as_ref()?.downcast_ref::<Sender<WorkerToMain<App>>>()?;

        Some(f(tx))
    })
}

/// Send a message to the main UI thread via the fast mpsc channel.
pub async fn send_message<App>(msg: WorkerToMain<App>) -> rustradio::Result<()>
where
    App: ApplicationSpecific + 'static,
{
    let inited = MAIN_UI_TX.with(|slot| slot.borrow().is_some());
    if inited {
        with_main_ui_tx::<App, _>(|tx| {
            let _ = tx.try_send(msg);
        })
        .await
        .ok_or_else(|| rustradio::Error::msg("MAIN_UI_TX has wrong type or was not initialized"))
    } else {
        error!("Tried to send before worker channel set up. Falling back to posting");
        post_message(&msg)
    }
}

/// Send a message to the main UI thread via the fast mpsc channel, from sync
/// code.
pub fn send_message_sync<App>(msg: WorkerToMain<App>) -> rustradio::Result<()>
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
