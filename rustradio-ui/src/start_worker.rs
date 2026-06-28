/// Handle the bootstrapping phase of the worker. While this code is running,
/// the worker is still running JS, not WASM.
///
/// We need this bit in order to start the worker with memory shared.
///
/// Well, we need some of it. There's one or two messages here that just logs
/// nicely how far the JS init has progressed.
use js_sys::{Object, Reflect};
use log::{error, info};
use wasm_bindgen::prelude::*;
use web_sys::js_sys;
use web_sys::{MessageEvent, Worker};

fn post_worker_init(worker: Worker) -> Result<(), JsValue> {
    let msg = Object::new();
    Reflect::set(
        &msg,
        &JsValue::from_str("type"),
        &JsValue::from_str("ruwasm-init"),
    )?;
    Reflect::set(&msg, &JsValue::from_str("memory"), &wasm_bindgen::memory())?;
    Reflect::set(
        &msg,
        &JsValue::from_str("module"),
        &Reflect::get(&js_sys::global(), &JsValue::from_str("__ruwasmModule"))?,
    )?;
    worker.post_message(&msg)
}

fn is_worker_bootstrap_ready(e: &MessageEvent) -> bool {
    Reflect::get(&e.data(), &JsValue::from_str("type"))
        .ok()
        .and_then(|v| v.as_string())
        .is_some_and(|msg_type| msg_type == "ruwasm-bootstrap-ready")
}

fn is_worker_bootstrap_init_received(e: &MessageEvent) -> bool {
    Reflect::get(&e.data(), &JsValue::from_str("type"))
        .ok()
        .and_then(|v| v.as_string())
        .is_some_and(|msg_type| msg_type == "ruwasm-bootstrap-init-received")
}

fn is_worker_bootstrap_init_complete(e: &MessageEvent) -> bool {
    Reflect::get(&e.data(), &JsValue::from_str("type"))
        .ok()
        .and_then(|v| v.as_string())
        .is_some_and(|msg_type| msg_type == "ruwasm-bootstrap-init-complete")
}

#[allow(clippy::nonminimal_bool)]
fn worker_bootstrap_error(e: &MessageEvent) -> Option<String> {
    if !Reflect::get(&e.data(), &JsValue::from_str("type"))
        .ok()
        .and_then(|v| v.as_string())
        .is_some_and(|msg_type| msg_type == "ruwasm-bootstrap-error")
    {
        return None;
    }

    let message = Reflect::get(&e.data(), &JsValue::from_str("message"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "unknown worker bootstrap error".to_string());
    let stack = Reflect::get(&e.data(), &JsValue::from_str("stack"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();

    Some(if stack.is_empty() {
        message
    } else {
        format!("{message}\n{stack}")
    })
}

/// Call this with early messages until it returns `true`. After that, regular
/// messages can be sent.
pub(crate) fn msg(worker: Worker, e: &MessageEvent) -> bool {
    if is_worker_bootstrap_ready(e) {
        info!("Main: Worker bootstrap ready");
        if let Err(e) = post_worker_init(worker) {
            error!("Main: failed to post worker init: {e:?}");
        }
    } else if is_worker_bootstrap_init_received(e) {
        info!("Main: Worker bootstrap init received");
    } else if is_worker_bootstrap_init_complete(e) {
        info!("Main: Worker bootstrap init complete");
        return true;
    } else if let Some(e) = worker_bootstrap_error(e) {
        error!("Main: Worker bootstrap failed: {e}");
    } else {
        error!("Main: got unexpected message during worker bootstrap: {e:?}");
    }
    false
}
