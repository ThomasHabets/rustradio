use std::cell::OnceCell;

use async_channel::{Receiver, Sender};
use log::{error, info};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use rustradio::blockchain;
use rustradio::graph::GraphRunner;
use rustradio_ui::worker::{send_message, source};
use rustradio_ui::{AppEmpty, TaggedVec};

use crate::{MainToWorker, MyWorkerToMain, WorkerToMain};

const RCV_SOURCE_ID: &str = "rcv";
const AUDIO_SAMPLE_RATE: usize = 44_100;
const SOURCE_CHANNEL_SIZE: usize = 10;

thread_local! {
static SOURCE: OnceCell<Sender<source::Msg<u8>>> = const { OnceCell::new() };
static GRAPH_POKE: OnceCell<Sender<()>> = const { OnceCell::new() };
}

async fn run_graph() -> Result<(), rustradio::Error> {
    use rustradio::blocks::*;
    use rustradio_ui::worker::FloatSink;

    let mut g = rustradio::wasm::wasm_graph::WasmGraph::default();
    let (src, prev, src_tx) = source::WasmSource::<MyWorkerToMain, _>::new(RCV_SOURCE_ID);

    let samp_rate = 250_000.0f32;
    let deci = 5;
    let filter1 = rustradio::fir::low_pass_complex(
        samp_rate,
        10_000.0,
        15_000.0,
        rustradio::window::WindowType::Hamming,
    );

    let prev = blockchain![
        g,
        prev,
        (src, prev),
        RtlSdrDecode::new(prev),
        FirFilter::builder(filter1).deci(deci).build(prev),
        QuadratureDemod::new(prev, 1.0),
        RationalResampler::builder()
            .deci((samp_rate as usize) / deci)
            .interp(AUDIO_SAMPLE_RATE)
            .build(prev)?,
    ];
    g.add(Box::new(FloatSink::<MyWorkerToMain>::new(
        prev,
        "audio".into(),
    )));
    info!("Running graph…");
    let (tx, rx) = async_channel::bounded(SOURCE_CHANNEL_SIZE);
    SOURCE.with(|slot| {
        slot.get_or_init(move || src_tx);
    });
    GRAPH_POKE.with(|slot| {
        slot.get_or_init(move || tx);
    });
    g.run_async(rx).await?;
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
