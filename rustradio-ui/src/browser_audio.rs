//! Browser audio playing.
use std::cell::Cell;
use std::cell::RefCell;

use log::{error, info};
use wasm_bindgen::prelude::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::js_sys;
use web_sys::{AudioContext, AudioContextState, GainNode};

// Maybe some of these should jut be default values, settable by the user.
const AUDIO_SAMPLE_RATE: f32 = 44_100.0;
const AUDIO_START_LATENCY_SECONDS: f64 = 0.08;
const AUDIO_TARGET_LATENCY_SECONDS: f64 = 0.10;
const AUDIO_MAX_LATENCY_SECONDS: f64 = 1.0;
const AUDIO_MAX_PLAYBACK_RATE: f32 = 1.02;

struct AudioPlayback {
    context: AudioContext,
    gain: GainNode,
    next_time: f64,
    chunks_queued: u64,
}

thread_local! {
    static AUDIO_PLAYBACK: RefCell<Option<AudioPlayback>> = const { RefCell::new(None) };
    static VOLUME: Cell<f32> = const { Cell::new(0.0) };
}

/// Set playback volume.
pub fn set_volume(v: f32) {
    VOLUME.set(v);
    AUDIO_PLAYBACK.with(|slot| {
        if let Some(audio) = slot.borrow().as_ref() {
            audio.gain.gain().set_value(v);
        }
    });
}

/// Lazily create the Web Audio graph and keep its gain node in sync.
fn ensure_audio_playback() -> Result<(), JsValue> {
    let volume = VOLUME.get();
    AUDIO_PLAYBACK.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(audio) = slot.as_mut() {
            audio.gain.gain().set_value(volume);
            request_resume(&audio.context)?;
            return Ok(());
        }

        let context = AudioContext::new()?;
        let gain = context.create_gain()?;
        gain.gain().set_value(volume);
        gain.connect_with_audio_node(context.destination().unchecked_ref())?;
        info!("Web Audio context created with state {:?}", context.state());
        request_resume(&context)?;
        let next_time = context.current_time() + AUDIO_START_LATENCY_SECONDS;
        *slot = Some(AudioPlayback {
            context,
            gain,
            next_time,
            chunks_queued: 0,
        });
        Ok(())
    })
}

fn request_resume(context: &AudioContext) -> Result<(), JsValue> {
    if context.state() == AudioContextState::Running {
        return Ok(());
    }
    let context = context.clone();
    let resume = context.resume()?;
    spawn_local(async move {
        match JsFuture::from(resume).await {
            Ok(_) => info!("Web Audio context resume completed with state {:?}", context.state()),
            Err(error) => error!("Web Audio context resume failed: {error:?}"),
        }
    });
    Ok(())
}

/// Restart sample scheduling slightly in the future to avoid immediate underruns.
pub fn reset() -> Result<(), JsValue> {
    ensure_audio_playback()?;
    AUDIO_PLAYBACK.with(|slot| {
        if let Some(audio) = slot.borrow_mut().as_mut() {
            audio.next_time = audio.context.current_time() + AUDIO_START_LATENCY_SECONDS;
        }
    });
    Ok(())
}

/// Return the bounded playback rate to use for the current queued latency.
fn audio_playback_rate(queued_seconds: f64) -> f32 {
    let excess = queued_seconds - AUDIO_TARGET_LATENCY_SECONDS;
    if excess <= 0.0 {
        return 1.0;
    }

    let correction_range = AUDIO_MAX_LATENCY_SECONDS - AUDIO_TARGET_LATENCY_SECONDS;
    let correction = (excess / correction_range).clamp(0.0, 1.0) as f32;
    1.0 + (AUDIO_MAX_PLAYBACK_RATE - 1.0) * correction
}

/// Queue one demodulated audio chunk for browser playback.
pub fn enqueue(samples: impl IntoIterator<Item = f32>) -> Result<(), JsValue> {
    let samples: Vec<_> = samples.into_iter().map(|s| s.clamp(-1.0, 1.0)).collect();
    if samples.is_empty() {
        return Ok(());
    }

    ensure_audio_playback()?;

    AUDIO_PLAYBACK.with(|slot| {
        let mut slot = slot.borrow_mut();
        let audio = slot
            .as_mut()
            .ok_or_else(|| JsValue::from_str("audio playback is not initialized"))?;
        if audio.chunks_queued == 0 {
            info!(
                "Web Audio scheduling first buffer: {} samples, context state {:?}",
                samples.len(),
                audio.context.state()
            );
        }
        let now = audio.context.current_time();
        let start_time = audio.next_time.max(now + AUDIO_START_LATENCY_SECONDS);
        let queued_seconds = (start_time - now).max(0.0);
        let playback_rate = audio_playback_rate(queued_seconds);
        let max_end_time = now + AUDIO_MAX_LATENCY_SECONDS;
        let available_seconds = (max_end_time - start_time).max(0.0);
        let max_samples =
            (available_seconds * f64::from(AUDIO_SAMPLE_RATE) * f64::from(playback_rate)).floor()
                as usize;
        let sample_offset = samples.len().saturating_sub(max_samples);
        if sample_offset == samples.len() {
            info!(
                "Main: dropping {} audio samples; queued audio is {:.0}ms",
                samples.len(),
                queued_seconds * 1000.0
            );
            return Ok(());
        }
        if sample_offset > 0 {
            info!(
                "Main: dropping {} audio samples to keep queued audio below {:.0}ms",
                sample_offset,
                AUDIO_MAX_LATENCY_SECONDS * 1000.0
            );
        }
        let samples = &samples[sample_offset..];
        let len =
            u32::try_from(samples.len()).map_err(|_| JsValue::from_str("audio chunk too large"))?;
        let buffer = audio.context.create_buffer(1, len, AUDIO_SAMPLE_RATE)?;
        // AudioBuffer.copyToChannel rejects views backed by shared Wasm memory.
        let channel = js_sys::Float32Array::new_with_length(len);
        channel.copy_from(samples);
        buffer.copy_to_channel_with_f32_array(&channel, 0)?;

        let source = audio.context.create_buffer_source()?;
        source.set_buffer(Some(&buffer));
        source.playback_rate().set_value(playback_rate);
        source.connect_with_audio_node(audio.gain.unchecked_ref())?;
        source.start_with_when(start_time)?;
        audio.chunks_queued += 1;
        audio.next_time = start_time
            + samples.len() as f64 / f64::from(AUDIO_SAMPLE_RATE) / f64::from(playback_rate);
        Ok(())
    })
}
