use std::cell::OnceCell;
use std::time::Duration;

use async_channel::Sender;
use futures_timer::Delay;
use log::{info, trace, warn};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Event, HtmlSelectElement, Response, js_sys};

use rustradio_ui::mainthread::{
    get_button, get_element, get_input, send_message, send_message_sync, spectrum_sink, time_sink,
};
use rustradio_ui::{AppEmpty, TaggedVec, spawn};

use crate::{MainToWorker, MyWorkerToMain, WorkerToMain};

// HTML DOM IDs.
pub(crate) const ID_LOG_OUTPUT: &str = "log-output";

// Controls.
const ID_START: &str = "button-start";
const ID_SOURCE: &str = "select-source";
const ID_FREQUENCY: &str = "input-frequency";
const ID_TUNE: &str = "button-tune";
const ID_GAIN: &str = "input-gain";
const ID_GAIN_APPLY: &str = "button-gain";
const ID_VOLUME: &str = "input-volume";

const B200_FIRMWARE_IMAGE: &str = "usrp_b200_fw.hex";
const B200_FPGA_IMAGE: &str = "usrp_b200_fpga.bin";
const B200_REENUMERATION_DELAY: Duration = Duration::from_secs(1);

// Visuals.
const ID_WATERFALL: &str = "waterfall";
const ID_WAVEFORM: &str = "audio-waveform";

pub(crate) const SAMPLE_RATE: u32 = 250_000;

thread_local! {
    static SDR_OPS: OnceCell<Sender<SdrOp>> = const {OnceCell::new() };
    static GRAPH_STARTED: OnceCell<()> = const { OnceCell::new() };
    static WATERFALL_SINK: OnceCell<spectrum_sink::WaterfallSink> = const { OnceCell::new() };
    static WAVEFORM_SINK: OnceCell<time_sink::TimeSink> = const { OnceCell::new() };
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

/// Borrow the application-owned waterfall sink handle from main-thread
/// callbacks.
fn with_waveform_sink<T>(
    f: impl FnOnce(&time_sink::TimeSink) -> rustradio::Result<T>,
) -> rustradio::Result<T> {
    WAVEFORM_SINK.with(|slot| {
        let Some(sink) = slot.get() else {
            return Err(rustradio::Error::msg(
                "waveform sink has not been initialized",
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
            get_select(ID_SOURCE)?.set_disabled(false);
            get_input(ID_FREQUENCY)?.set_disabled(false);
            get_input(ID_GAIN)?.set_disabled(false);
        }
        WorkerToMain::Floats(name, streams) => match name.as_str() {
            crate::worker::STREAM_AUDIO => {
                assert_eq!(streams.len(), 1);
                trace!("Got audio samples {}", streams[0].data.len());
                rustradio_ui::browser_audio::enqueue(streams[0].data.iter().copied())?;
                with_waveform_sink(|sink| sink.update(streams))?;
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

#[derive(Debug)]
enum SdrOp {
    Tune(u32),
    Gain(GainSetting),
}

#[derive(Clone, Copy, Debug)]
enum GainSetting {
    Automatic,
    Manual(f64),
}

#[derive(Clone, Copy, Debug)]
enum SourceKind {
    RtlSdr,
    B200,
}

fn get_select(id: &str) -> Result<HtmlSelectElement, JsValue> {
    Ok(get_element(id)?.dyn_into()?)
}

fn selected_source() -> Result<SourceKind, JsValue> {
    match get_select(ID_SOURCE)?.value().as_str() {
        "rtlsdr" => Ok(SourceKind::RtlSdr),
        "b200" => Ok(SourceKind::B200),
        source => Err(JsValue::from_str(&format!("unknown SDR source {source:?}"))),
    }
}

async fn run_rtlsdr_source(mut sdr: rtlsdr_pure::RtlSdr) -> Result<(), JsValue> {
    let (ops_tx, ops_rx) = async_channel::bounded(10); // TODO: magic value.
    let _ = SDR_OPS.with(|slot| slot.set(ops_tx));
    get_button(ID_TUNE)?.set_disabled(false);
    get_button(ID_GAIN_APPLY)?.set_disabled(false);

    let sample_rate: u32 = SAMPLE_RATE;
    let gain = parse_gain_mode(SourceKind::RtlSdr)?;
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
    let actual_rate = sdr.set_sample_rate(sample_rate).await.map_err(js_error)?;
    info!("sample rate: {actual_rate} Hz");

    if sdr.tuner_kind().is_supported() {
        let gain_mode = rtlsdr_gain_mode(gain);
        sdr.set_tuner_gain(gain_mode).await.map_err(js_error)?;
        info!("RTLSDR tuner gain: {gain_mode:?}");
        sdr.set_center_frequency(freq).await.map_err(js_error)?;
        info!("center frequency: {freq} Hz");
    } else {
        info!("center frequency: skipped for unsupported tuner");
    }
    sdr.reset_buffer().await.map_err(js_error)?;

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
        match ops_rx.try_recv() {
            Ok(SdrOp::Tune(freq)) => {
                info!("Setting frequency to {freq}");
                sdr.set_center_frequency(freq).await.map_err(js_error)?;
            }
            Ok(SdrOp::Gain(gain)) => {
                info!("Setting RTLSDR tuner gain to {gain:?}");
                sdr.set_tuner_gain(rtlsdr_gain_mode(gain))
                    .await
                    .map_err(js_error)?;
            }
            Err(_e) => {}
        }
        let bytes = sdr.read_bytes(read_len).await.map_err(js_error)?;
        /*
        deadline =
            js_performance_now() + 1_000.0f64 * ((bytes.len() / 2) as f64) / f64::from(actual_rate);
        */
        assert!(bytes.len().is_multiple_of(2));
        //log::trace!("Read {} bytes from rtlsdr", bytes.len());
        send_complex_samples(decode_rtlsdr_samples(&bytes)).await?;
    }
}

async fn run_b200_source(mut receiver: uhd_pure::b2xx::B2xxReceiver) -> Result<(), JsValue> {
    let (ops_tx, ops_rx) = async_channel::bounded(10);
    let _ = SDR_OPS.with(|slot| slot.set(ops_tx));
    get_button(ID_TUNE)?.set_disabled(false);
    get_button(ID_GAIN_APPLY)?.set_disabled(false);

    info!(
        "USRP {} serial={} name={:?}",
        receiver.product(),
        receiver.identity().serial,
        receiver.identity().name
    );
    info!("sample rate: {} Hz", receiver.sample_rate_hz());
    info!("center frequency: {} Hz", receiver.center_frequency_hz());
    info!("Running the USRP B200 loop");

    loop {
        match ops_rx.try_recv() {
            Ok(SdrOp::Tune(freq)) => {
                info!("Setting frequency to {freq}");
                receiver
                    .set_center_frequency(f64::from(freq))
                    .await
                    .map_err(js_error)?;
            }
            Ok(SdrOp::Gain(gain)) => {
                info!("Setting USRP receive gain to {gain:?}");
                receiver
                    .set_gain(uhd_gain_mode(gain))
                    .await
                    .map_err(js_error)?;
            }
            Err(_e) => {}
        }

        let packet = match receiver.receive().await {
            Ok(packet) => packet,
            Err(uhd_pure::Error::ReceiveOverflow { expected, actual }) => {
                warn!("USRP receive overflow: expected CHDR sequence {expected}, got {actual}");
                continue;
            }
            Err(uhd_pure::Error::DeviceReceiveOverflow { sequence }) => {
                warn!("USRP receive FIFO overflow at CHDR sequence {sequence}; stream restarted");
                continue;
            }
            Err(error) => return Err(js_error(error)),
        };
        let samples = packet
            .samples
            .into_iter()
            .map(|sample| rustradio::Complex::new(sample.re, sample.im))
            .collect();
        send_complex_samples(samples).await?;
    }
}

async fn open_b200_source() -> Result<Option<uhd_pure::b2xx::B2xxReceiver>, JsValue> {
    let info = uhd_pure::b2xx::request_device()
        .await
        .map_err(js_error)?
        .ok_or_else(|| JsValue::from_str("USRP chooser was cancelled"))?;
    if !info.firmware_loaded {
        info!("USRP B200 is in its FX3 bootloader; downloading firmware");
        let firmware = fetch_b200_image(B200_FIRMWARE_IMAGE).await?;
        let device = info.open().await.map_err(js_error)?;
        info!("Loading USRP B200 FX3 firmware");
        device.load_firmware(&firmware).await.map_err(js_error)?;
        drop(device);
        // Give the old WebUSB device time to disappear before inviting the
        // user to select its newly enumerated, firmware-backed identity.
        Delay::new(B200_REENUMERATION_DELAY).await;
        info!("FX3 firmware started; click Start again and select the re-enumerated USRP B200");
        return Ok(None);
    }
    let frequency_hz = parse_frequency_hz()?;
    let gain = parse_gain_mode(SourceKind::B200)?;
    let device = info.open().await.map_err(js_error)?;

    match device.check_firmware_compatibility().await {
        Ok(_) => {}
        Err(error @ uhd_pure::Error::FirmwareCompatibility { .. }) => {
            warn!("{error}; resetting the USRP B200 to its FX3 bootloader");
            device.reset_fx3().await.map_err(js_error)?;
            drop(device);
            Delay::new(B200_REENUMERATION_DELAY).await;
            info!(
                "Click Start again, select the B200 bootloader, and the required firmware will be loaded"
            );
            return Ok(None);
        }
        Err(error) => return Err(js_error(error)),
    }

    info!("Downloading the USRP B200 FPGA image");
    let fpga = fetch_b200_image(B200_FPGA_IMAGE).await?;
    match device.load_fpga(&fpga, false).await.map_err(js_error)? {
        uhd_pure::b2xx::LoadOutcome::AlreadyLoaded => {
            info!("USRP B200 FPGA image is already loaded");
        }
        uhd_pure::b2xx::LoadOutcome::Loaded => info!("Loaded USRP B200 FPGA image"),
    }

    info!("Initializing the USRP B200 radio");
    let receiver = uhd_pure::b2xx::B2xxReceiver::open(
        device,
        uhd_pure::b2xx::RxConfig {
            center_frequency_hz: frequency_hz,
            sample_rate_hz: f64::from(SAMPLE_RATE),
            gain: uhd_gain_mode(gain),
        },
    )
    .await
    .map_err(js_error)?;
    info!("USRP B200 radio initialized and receive stream started");
    Ok(Some(receiver))
}

async fn fetch_b200_image(filename: &str) -> Result<Vec<u8>, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no browser window"))?;
    let response = JsFuture::from(window.fetch_with_str(filename))
        .await?
        .dyn_into::<Response>()?;
    if !response.ok() {
        return Err(JsValue::from_str(&format!(
            "could not download {filename}: HTTP {}; copy it next to the application WASM",
            response.status()
        )));
    }
    let buffer = JsFuture::from(response.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

async fn send_complex_samples(samples: Vec<rustradio::Complex>) -> Result<(), JsValue> {
    send_message(MainToWorker::Complexes(
        crate::worker::RCV_SOURCE_ID.into(),
        vec![TaggedVec {
            data: samples,
            tags: vec![],
        }],
    ))
    .await
    .map_err(|_| JsValue::from_str("failed to send SDR samples to worker"))
}

fn decode_rtlsdr_samples(bytes: &[u8]) -> Vec<rustradio::Complex> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|sample| {
            rustradio::Complex::new(
                (f32::from(sample[0]) - 127.0) * 0.008,
                (f32::from(sample[1]) - 127.0) * 0.008,
            )
        })
        .collect()
}

fn handle_tune() -> Result<(), JsValue> {
    let freq = get_input(ID_FREQUENCY)?
        .value()
        .parse::<f64>()
        .map_err(|e| JsValue::from_str(&format!("{e:?}")))?
        * 1e6;
    spawn(async move {
        SDR_OPS
            .with(|slot| slot.get().unwrap().clone())
            .send(SdrOp::Tune(freq as u32))
            .await
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    });
    Ok(())
}

fn parse_gain_mode(source: SourceKind) -> Result<GainSetting, JsValue> {
    let input = get_input(ID_GAIN)?;
    let gain = input.value();
    let gain = gain.trim();
    if gain.eq_ignore_ascii_case("auto") {
        input.set_custom_validity("");
        return Ok(GainSetting::Automatic);
    }

    let gain_db = gain.parse::<f64>().map_err(|e| {
        let msg = format!("Gain must be auto or a number in dB: {e}");
        input.set_custom_validity(&msg);
        let _ = input.report_validity();
        JsValue::from_str(&msg)
    })?;
    if !gain_db.is_finite() {
        let msg = "Gain must be a finite number";
        input.set_custom_validity(msg);
        let _ = input.report_validity();
        return Err(JsValue::from_str(msg));
    }

    let (minimum, maximum) = match source {
        SourceKind::RtlSdr => (-10.0, 50.0),
        SourceKind::B200 => (0.0, 76.0),
    };
    if !(minimum..=maximum).contains(&gain_db) {
        let msg = format!("Gain must be auto or {minimum:.1}..{maximum:.1} dB");
        input.set_custom_validity(&msg);
        let _ = input.report_validity();
        return Err(JsValue::from_str(&msg));
    }

    input.set_custom_validity("");
    Ok(GainSetting::Manual(gain_db))
}

fn rtlsdr_gain_mode(gain: GainSetting) -> rtlsdr_pure::GainMode {
    match gain {
        GainSetting::Automatic => rtlsdr_pure::GainMode::Auto,
        GainSetting::Manual(gain_db) => {
            rtlsdr_pure::GainMode::ManualTenthsDb((gain_db * 10.0).round() as i32)
        }
    }
}

fn uhd_gain_mode(gain: GainSetting) -> uhd_pure::b2xx::RxGain {
    match gain {
        GainSetting::Automatic => uhd_pure::b2xx::RxGain::Automatic,
        GainSetting::Manual(gain_db) => uhd_pure::b2xx::RxGain::Manual(gain_db),
    }
}

fn parse_frequency_hz() -> Result<f64, JsValue> {
    get_input(ID_FREQUENCY)?
        .value()
        .parse::<f64>()
        .map(|frequency_mhz| frequency_mhz * 1e6)
        .map_err(|e| JsValue::from_str(&format!("{e:?}")))
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn audio_volume() -> Result<f32, JsValue> {
    Ok(get_input(ID_VOLUME)?
        .value()
        .parse::<f32>()
        .unwrap_or(0.25)
        .clamp(0.0, 1.0))
}

fn handle_gain() -> Result<(), JsValue> {
    let gain = parse_gain_mode(selected_source()?)?;
    spawn(async move {
        SDR_OPS
            .with(|slot| slot.get().unwrap().clone())
            .send(SdrOp::Gain(gain))
            .await
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    });
    Ok(())
}

fn handle_enter_button(
    event: Event,
    button_id: &str,
    action: fn() -> Result<(), JsValue>,
) -> Result<(), JsValue> {
    let key = js_sys::Reflect::get(event.as_ref(), &JsValue::from_str("key"))?.as_string();
    if key.as_deref() != Some("Enter") {
        return Ok(());
    }

    event.prevent_default();
    if get_button(button_id)?.disabled() {
        return Ok(());
    }
    action()
}

fn handle_start() -> Result<(), JsValue> {
    let source = selected_source()?;
    get_button(ID_START)?.set_disabled(true);
    get_select(ID_SOURCE)?.set_disabled(true);
    // Creating and resuming AudioContext here keeps it inside the browser's
    // user-gesture window. Waiting for the first asynchronously produced audio
    // chunk can leave the context suspended by autoplay policy.
    rustradio_ui::browser_audio::set_volume(audio_volume()?);
    rustradio_ui::browser_audio::reset()?;
    spawn_local(async move {
        match source {
            SourceKind::RtlSdr => {
                let sdr = match rtlsdr_pure::open_first().await {
                    Err(e) => {
                        warn!("Failed to open RTLSDR: {e}");
                        enable_start_controls();
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
                    warn!("RTL-SDR source failed: {e:?}");
                }
            }
            SourceKind::B200 => match open_b200_source().await {
                Ok(Some(receiver)) => {
                    if let Err(e) = run_b200_source(receiver).await {
                        warn!("USRP B200 source failed: {e:?}");
                    }
                }
                Ok(None) => enable_start_controls(),
                Err(e) => {
                    warn!("Failed to open USRP B200: {e:?}");
                    enable_start_controls();
                }
            },
        }
    });
    GRAPH_STARTED.with(|started| {
        if started.set(()).is_ok() {
            send_message_sync(MainToWorker::Start(AppEmpty {}))?;
        }
        Ok::<(), JsValue>(())
    })?;
    Ok(())
}

fn enable_start_controls() {
    if let Ok(button) = get_button(ID_START) {
        button.set_disabled(false);
    }
    if let Ok(source) = get_select(ID_SOURCE) {
        source.set_disabled(false);
    }
}

pub(crate) async fn setup() -> Result<(), JsValue> {
    // Start button.
    {
        let handler = Closure::<dyn FnMut() -> Result<(), JsValue>>::new(handle_start);
        let btn = get_button(ID_START)?;
        btn.add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    // Tune button.
    {
        let handler = Closure::<dyn FnMut() -> Result<(), JsValue>>::new(handle_tune);
        let btn = get_button(ID_TUNE)?;
        btn.add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    // Tune input.
    {
        let handler = Closure::<dyn FnMut(Event) -> Result<(), JsValue>>::new(move |event| {
            handle_enter_button(event, ID_TUNE, handle_tune)
        });
        let input = get_input(ID_FREQUENCY)?;
        input.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    // Gain button.
    {
        let handler = Closure::<dyn FnMut() -> Result<(), JsValue>>::new(handle_gain);
        let btn = get_button(ID_GAIN_APPLY)?;
        btn.add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    // Gain input.
    {
        let handler = Closure::<dyn FnMut(Event) -> Result<(), JsValue>>::new(move |event| {
            handle_enter_button(event, ID_GAIN_APPLY, handle_gain)
        });
        let input = get_input(ID_GAIN)?;
        input.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    // Volume.
    {
        let input = get_input(ID_VOLUME)?;
        let handler = Closure::<dyn FnMut(Event) -> Result<(), JsValue>>::new(move |_event| {
            rustradio_ui::browser_audio::set_volume(audio_volume()?);
            Ok(())
        });
        input.add_event_listener_with_callback("input", handler.as_ref().unchecked_ref())?;
        handler.forget();
        rustradio_ui::browser_audio::set_volume(audio_volume()?);
    }

    // Waterfall.
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

    // Audio waveform.
    {
        let wave = time_sink::TimeSink::mount_by_id(
            ID_WAVEFORM,
            time_sink::TimeSinkOptions {
                title: "Audio".into(),
                subtitle: "Audio waveform".into(),
                sample_rate: crate::worker::AUDIO_SAMPLE_RATE as f64,
                max_points: 3 * crate::worker::AUDIO_SAMPLE_RATE,
                fixed_range: Some((-1.0, 1.0)),
                ..Default::default()
            },
        )?;
        let _ = WAVEFORM_SINK.with(|slot| slot.set(wave));
    }
    rustradio_ui::mainthread::start_worker::<AppEmpty, MyWorkerToMain, _, _>(worker_msg);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtl_samples_keep_existing_scaling() {
        assert_eq!(
            decode_rtlsdr_samples(&[0, 10, 20, 10, 0]),
            vec![
                rustradio::Complex::new(-1.016, -0.93600005),
                rustradio::Complex::new(-0.85600007, -0.93600005),
            ]
        );
    }
}
