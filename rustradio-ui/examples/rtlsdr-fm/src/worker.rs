use std::cell::OnceCell;

use async_channel::{Receiver, Sender};
use log::{error, info};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use rustradio_ui::worker::{send_message, source};
use rustradio_ui::{AppEmpty, TaggedVec};

use crate::{MainToWorker, WorkerToMain};

thread_local! {
static SOURCE: OnceCell<Sender<source::Msg<u8>>> = const { OnceCell::new() };
}

async fn run_graph() -> Result<(), JsValue> {
    Ok(())
}

async fn worker_msg(msg: MainToWorker) -> Result<(), JsValue> {
    match msg {
        MainToWorker::Start(_) => {
            info!("Got Start message");
            run_graph().await?;
            send_message(WorkerToMain::End(AppEmpty {})).await?;
        }
        MainToWorker::Bytes(_name, streams) => {
            send_source_msg(source_msg_from_streams(streams)?).await?
        }
        other => error!("Got unknown message {other:?}"),
    }
    Ok(())
}

async fn send_source_msg(msg: source::Msg<u8>) -> Result<(), JsValue> {
    let Some(tx) = SOURCE.with(|cell| {
        let cell = cell.clone();
        cell.get().cloned()
    }) else {
        return Err(JsValue::from_str(
            "tried to send bytes before graph was started",
        ));
    };
    tx.send(msg)
        .await
        .map_err(|e| JsValue::from_str(&format!("SendError: {e:?}")))?;
    Ok(())
}

fn source_msg_from_streams(mut streams: Vec<TaggedVec<u8>>) -> Result<source::Msg<u8>, JsValue> {
    if streams.len() != 1 {
        return Err(JsValue::from_str(&format!(
            "Got bytes with {} streams, want 1",
            streams.len()
        )));
    }
    let stream = streams.pop().expect("can't happen");
    if stream.data.is_empty() {
        Ok(source::Msg::Eof)
    } else {
        Ok(source::Msg::Extend(stream.data))
    }
}

fn ready(rx: Receiver<MainToWorker>) {
    spawn_local(async move {
        rustradio_ui::worker::send_message(WorkerToMain::Ready(AppEmpty {}))
            .await
            .expect("failed to send ready message");
        while let Ok(msg) = rx.recv().await {
            if let Err(e) = worker_msg(msg).await {
                error!("Failed to handle worker message: {e:?}");
            }
        }
    })
}

pub(crate) async fn setup() -> Result<(), JsValue> {
    rustradio_ui::worker::setup::<_, crate::MyWorkerToMain, _>(&ready).await?;
    Ok(())
}
