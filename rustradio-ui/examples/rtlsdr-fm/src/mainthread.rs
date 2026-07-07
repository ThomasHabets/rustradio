use log::{info, trace, warn};
use std::cell::OnceCell;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use rustradio_ui::mainthread::{
    get_button, get_input, send_message, send_message_sync, spectrum_sink,
};
use rustradio_ui::{AppEmpty, TaggedVec};

use crate::{MainToWorker, MyMainToWorker, MyWorkerToMain, WorkerToMain};

// HTML DOM IDs.
pub(crate) const ID_LOG_OUTPUT: &str = "log-output";
const ID_START: &str = "button-start";
const ID_FREQUENCY: &str = "input-frequency";
const ID_WATERFALL: &str = "waterfall";

pub(crate) const SAMPLE_RATE: u32 = 250_000;

thread_local! {
    static WATERFALL_SINK: OnceCell<spectrum_sink::WaterfallSink> = const { OnceCell::new() };
}

/// Borrow the application-owned waterfall sink handle from main-thread
/// callbacks.
fn with_waterfall_sink<T>(
    f: impl FnOnce(&spectrum_sink::WaterfallSink) -> rustradio::Result<T>,
) -> rustradio::Result<T> {
    WATERFALL_SINK.with(|slot| {
        let Some(sink) = slot.get() else {
            return Err(rustradio::Error::msg(
                "waterfall sink has not been initialized",
            ));
        };
        f(sink)
    })
}

async fn worker_msg(msg: WorkerToMain) -> Result<(), JsValue> {
    match msg {
        WorkerToMain::LogLine { .. } => {}
        WorkerToMain::Ready(_) => {
            info!("Worker says it's ready");
            get_button(ID_START)?.set_disabled(false);
            get_input(ID_FREQUENCY)?.set_disabled(false);
        }
        WorkerToMain::Floats(name, streams) => match name.as_str() {
            crate::worker::STREAM_AUDIO => {
                assert_eq!(streams.len(), 1);
                trace!("Got audio samples {}", streams[0].data.len());
                rustradio_ui::browser_audio::enqueue(streams[0].data.iter().copied())?;
            }
            crate::worker::STREAM_SPECTRUM => {
                trace!("Got spectrum size {}", streams[0].data.len());
                with_waterfall_sink(|sink| sink.update(&streams))?;
            }
            _ => {}
        },
        WorkerToMain::RequestData(_, _) => {}
        ref _other => info!("Got worker msg: {msg:?}"),
    }
    Ok(())
}

async fn run_rtlsdr_source(mut sdr: rtlsdr_pure::RtlSdr) -> Result<(), JsValue> {
    let sample_rate: u32 = SAMPLE_RATE;
    let gain_mode = rtlsdr_pure::GainMode::Auto;
    let freq = get_input(ID_FREQUENCY)?
        .value()
        .parse::<f64>()
        .map_err(|e| JsValue::from_str(&format!("{e:?}")))?
        * 1e6;
    let freq = freq as u32;
    info!(
        "RTLSDR manufacturer: {}",
        sdr.manufacturer().unwrap_or("<unknown>")
    );
    info!("RTLSDR product: {}", sdr.product().unwrap_or("<unknown>"));
    info!("RTLSDR tuner: {:?}", sdr.tuner_kind());
    let actual_rate = sdr.set_sample_rate(sample_rate).await?;
    info!("sample rate: {actual_rate} Hz");

    if sdr.tuner_kind().is_supported() {
        sdr.set_tuner_gain(gain_mode).await?;
        info!("RTLSDR tuner gain: {gain_mode:?}");
        sdr.set_center_frequency(freq).await?;
        info!("center frequency: {freq} Hz");
    } else {
        info!("center frequency: skipped for unsupported tuner");
    }
    sdr.reset_buffer().await?;

    let read_len = 16384usize; // 33ms.
    info!("Running the rtlsdr loop");
    //let mut deadline = js_performance_now() + 1000.0f64; // One second. Basically infin    ite time.
    loop {
        /*
        let now = js_performance_now();
        if now > deadline {
            warn!(
                "Slow to read from RTLSDR! Missed it by {} ms",
                now - deadline
            );
        }
        */
        let bytes = sdr.read_bytes(read_len).await?;
        /*
        deadline =
            js_performance_now() + 1_000.0f64 * ((bytes.len() / 2) as f64) / f64::from(actual_rate);
        */
        assert!(bytes.len().is_multiple_of(2));
        //log::trace!("Read {} bytes from rtlsdr", bytes.len());
        send_message(MainToWorker::Bytes(
            crate::worker::RCV_SOURCE_ID.into(),
            vec![TaggedVec {
                data: bytes,
                tags: vec![],
            }],
        ))
        .await
        .map_err(|_| JsValue::from_str("failed to send to worker"))?;
    }
}

fn handle_start() -> Result<(), JsValue> {
    get_button(ID_START)?.set_disabled(true);
    get_input(ID_FREQUENCY)?.set_disabled(true);
    rustradio_ui::browser_audio::set_volume(0.2);
    spawn_local(async move {
        // Get the RTLSDR.
        let sdr = match rtlsdr_pure::open_first().await {
            Err(e) => {
                warn!("Failed to open RTLSDR: {e}");
                return;
            }
            Ok(sdr) => {
                info!(
                    "opened {:04x}:{:04x} {}",
                    sdr.vendor_id(),
                    sdr.product_id(),
                    sdr.known_name().unwrap_or("RTL-SDR")
                );
                sdr
            }
        };
        if let Err(e) = run_rtlsdr_source(sdr).await {
            warn!("RTL SDR source failed: {e:?}");
        }
    });
    send_message_sync(MainToWorker::Start(AppEmpty {}))?;
    Ok(())
}

pub(crate) async fn setup() -> Result<(), JsValue> {
    {
        let handler = Closure::<dyn FnMut() -> Result<(), JsValue>>::new(handle_start);
        let btn = get_button(ID_START)?;
        btn.add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    {
        let water = spectrum_sink::WaterfallSink::mount_by_id(
            ID_WATERFALL,
            spectrum_sink::WaterfallSinkOptions {
                title: "Waterfall".into(),
                subtitle: "FFT power history".into(),
                sample_rate: SAMPLE_RATE as f32,
                ..Default::default()
            },
        )?;
        let _ = WATERFALL_SINK.with(|slot| slot.set(water));
    }
    rustradio_ui::mainthread::start_worker::<MyMainToWorker, MyWorkerToMain, _, _>(worker_msg);
    Ok(())
}
