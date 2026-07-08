use std::cell::OnceCell;

use async_channel::{Receiver, Sender};
use log::{error, info, trace};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use rustradio::blockchain;
use rustradio::graph::GraphRunner;
use rustradio_ui::worker::{send_message, source};
use rustradio_ui::{spawn, AppEmpty, TaggedVec};

use crate::{MainToWorker, MyWorkerToMain, WorkerToMain};

pub(crate) const RCV_SOURCE_ID: &str = "rtl-sdr";
pub(crate) const STREAM_AUDIO: &str = "audio";
pub(crate) const STREAM_SPECTRUM: &str = "spectrum";
pub(crate) const AUDIO_SAMPLE_RATE: usize = 44_100;
const SOURCE_CHANNEL_SIZE: usize = 10;
const SPECTRUM_SIZE: usize = 2048;
const DECI: u32 = crate::mainthread::SAMPLE_RATE / 50_000;

thread_local! {
static SOURCE: OnceCell<Sender<source::Msg<u8>>> = const { OnceCell::new() };
static GRAPH_POKE: OnceCell<Sender<()>> = const { OnceCell::new() };
}

async fn run_graph() -> Result<(), rustradio::Error> {
    use rustradio::blocks::*;
    use rustradio_ui::worker::{FloatPduSink, FloatSink};

    let mut g = rustradio::wasm::wasm_graph::WasmGraph::default();
    let (src, prev, src_tx) = source::WasmSource::<MyWorkerToMain, _>::new(RCV_SOURCE_ID);

    let samp_rate = crate::mainthread::SAMPLE_RATE as f32;
    let filter1 = rustradio::fir::low_pass_complex(
        samp_rate,
        90_000.0,
        30_000.0,
        rustradio::window::WindowType::Hamming,
    );

    let prev = blockchain![
        g,
        prev,
        (src, prev),
        RtlSdrDecode::new(prev),
        {
            let mut dropcount = 0;
            let (tee, a, prev) = Tee::new(prev);
            let prev = blockchain![
                g,
                prev,
                StreamChunks::new(prev, SPECTRUM_SIZE),
                NCMap::new(prev, "downsample", move |i, tags| {
                    dropcount += 1;
                    if dropcount == 15 {
                        dropcount = 0;
                        vec![(i, tags)]
                    } else {
                        vec![]
                    }
                }),
                Fft::from_fft_size(prev, SPECTRUM_SIZE)?,
                NCMap::new(prev, "fft_power_db", |v, tags| {
                    vec![(
                        v.iter()
                            .map(|bin| {
                                let power = (bin.norm_sqr() / SPECTRUM_SIZE as f32).max(1.0e-20);
                                10.0 * power.log10()
                            })
                            .collect(),
                        tags,
                    )]
                }),
            ];
            g.add(Box::new(FloatPduSink::<MyWorkerToMain>::new(
                prev,
                STREAM_SPECTRUM,
            )));
            (tee, a)
        },
        FirFilter::builder(filter1).deci(DECI as usize).build(prev),
        QuadratureDemod::new(prev, 1.0),
        RationalResampler::builder()
            .deci((samp_rate as usize) / (DECI as usize))
            .interp(AUDIO_SAMPLE_RATE)
            .build(prev)?,
    ];
    g.add(Box::new(FloatSink::<MyWorkerToMain>::new(
        prev,
        STREAM_AUDIO,
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
            spawn(async {
                run_graph().await?;
                send_message(WorkerToMain::End(AppEmpty {})).await?;
                Ok(())
            })
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
    let Some(tx) = GRAPH_POKE.with(|cell| {
        let cell = cell.clone();
        cell.get().cloned()
    }) else {
        return Err(JsValue::from_str("tried to send bytes with poke unset"));
    };
    trace!("Waking up graph");
    tx.send(())
        .await
        .map_err(|e| JsValue::from_str(&format!("SendError: {e:?}")))?;
    trace!("Graph should have more now");
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
            trace!("Worker received {msg:?}");
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
