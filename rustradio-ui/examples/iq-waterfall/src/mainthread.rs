use std::cell::OnceCell;

use rustradio_ui::mainthread::spectrum_sink::{WaterfallSink, WaterfallSinkOptions};
use rustradio_ui::mainthread::{get_button, get_element, get_input, send_message};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::{AppMessage, Connection, MainToWorker, WorkerToMain};

thread_local! {
    static WATERFALL: OnceCell<WaterfallSink> = const { OnceCell::new() };
}

fn status(text: &str) -> Result<(), JsValue> {
    get_element("status")?.set_text_content(Some(text));
    Ok(())
}

fn controls(active: bool) -> Result<(), JsValue> {
    get_button("connect")?.set_disabled(active);
    get_button("disconnect")?.set_disabled(!active);
    for name in ["host", "port", "source", "tls"] {
        get_input(name)?.set_disabled(active);
    }
    get_element("loss-policy")?
        .dyn_into::<web_sys::HtmlSelectElement>()?
        .set_disabled(active);
    Ok(())
}

fn connection() -> Result<Connection, JsValue> {
    let host = get_input("host")?.value().trim().to_owned();
    if host.is_empty()
        || host.contains(['/', '?', '#', '@'])
        || host.chars().any(char::is_whitespace)
    {
        return Err(JsValue::from_str(
            "Enter a hostname or IP address without a URL or path.",
        ));
    }
    let port = get_input("port")?
        .value()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| JsValue::from_str("Port must be between 1 and 65535."))?;
    // Bracket bare IPv6 addresses before constructing the browser URL.
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host
    };
    let scheme = if get_input("tls")?.checked() {
        "wss"
    } else {
        "ws"
    };
    let url = web_sys::Url::new(&format!("{scheme}://{host}:{port}/iq/v1/stream"))?;
    let source = get_input("source")?.value().trim().to_owned();
    if source.is_empty() {
        return Err(JsValue::from_str("Enter the sink's source identifier."));
    }
    let allow_gaps = get_element("loss-policy")?
        .dyn_into::<web_sys::HtmlSelectElement>()?
        .value()
        == "gaps";
    Ok(Connection {
        url: url.href(),
        source,
        allow_gaps,
    })
}

fn connect() {
    let result = (|| {
        let settings = connection()?;
        WATERFALL
            .with(|slot| slot.get().expect("mounted waterfall").clear())
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        controls(true)?;
        status("Connecting…")?;
        spawn_local(async move {
            if let Err(e) = send_message(MainToWorker::Start(settings)).await {
                let _ = status(&format!("Connection failed: {e}"));
                let _ = controls(false);
            }
        });
        Ok::<_, JsValue>(())
    })();
    if let Err(e) = result {
        let _ = status(&e.as_string().unwrap_or_else(|| format!("{e:?}")));
    }
}

async fn worker_msg(message: WorkerToMain) -> Result<(), JsValue> {
    match message {
        WorkerToMain::Ready(_) => {
            controls(false)?;
            status("Ready to connect")?;
        }
        WorkerToMain::ApplicationSpecific(AppMessage::Connected {
            sample_rate,
            source,
        }) => {
            WATERFALL
                .with(|slot| {
                    slot.get()
                        .expect("mounted waterfall")
                        .set_sample_rate(sample_rate)
                })
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            status(&format!("Streaming {source} · {sample_rate} samples/s"))?;
        }
        WorkerToMain::Floats(name, frames) if name == worker::SPECTRUM => {
            WATERFALL
                .with(|slot| slot.get().expect("mounted waterfall").update(&frames))
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
        }
        WorkerToMain::End(message) => {
            status(&message)?;
            controls(false)?;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn setup() -> Result<(), JsValue> {
    let waterfall = WaterfallSink::mount_by_id(
        "waterfall",
        WaterfallSinkOptions {
            title: "I/Q waterfall".into(),
            subtitle: "Frequency relative to the stream center".into(),
            ..Default::default()
        },
    )
    .map_err(|e| JsValue::from_str(&e.to_string()))?;
    WATERFALL.with(|slot| {
        let _ = slot.set(waterfall);
    });
    let connect_handler = Closure::<dyn FnMut()>::new(connect);
    get_button("connect")?
        .add_event_listener_with_callback("click", connect_handler.as_ref().unchecked_ref())?;
    connect_handler.forget();
    let disconnect_handler = Closure::<dyn FnMut()>::new(|| {
        let _ = get_button("disconnect").map(|button| button.set_disabled(true));
        let _ = status("Disconnecting…");
        spawn_local(async {
            if let Err(e) = send_message(MainToWorker::ApplicationSpecific(AppMessage::Stop)).await
            {
                let _ = status(&e.to_string());
                let _ = controls(false);
            }
        });
    });
    get_button("disconnect")?
        .add_event_listener_with_callback("click", disconnect_handler.as_ref().unchecked_ref())?;
    disconnect_handler.forget();
    get_input("tls")?.set_checked(
        web_sys::window()
            .expect("main thread")
            .location()
            .protocol()?
            == "https:",
    );
    rustradio_ui::mainthread::start_worker::<AppMessage, AppMessage, _, _>(worker_msg);
    Ok(())
}

use crate::worker;
