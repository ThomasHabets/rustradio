use std::cell::RefCell;
use std::future::{Future, poll_fn};
use std::task::Poll;

use async_channel::{Receiver, Sender};
use rustradio::block::{Block, BlockRet};
use rustradio::blocks::{Fft, NCMap, StreamChunks};
use rustradio::graph::GraphRunner;
use rustradio::iq_stream::{GAP_SAMPLES, StreamOptions, proto};
use rustradio::stream::{NCReadStream, Tag};
use rustradio::{Complex, Float};
use rustradio_ui::worker::{IqStreamSource, send_message};
use rustradio_ui::{AppEmpty, TaggedVec};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;

use crate::{AppMessage, Connection, MainToWorker, WorkerToMain};

pub(crate) const SPECTRUM: &str = "spectrum";
const FFT_SIZE: usize = 2048;
const ROWS_PER_SECOND: f32 = 30.0;
thread_local! {
    static STOP: RefCell<Option<Sender<()>>> = const { RefCell::new(None) };
}

// Allow Stop during both connection establishment and graph execution. Dropping
// the losing future drops its source/socket guard and cancels the IQ session.
async fn until_stop<T>(
    future: impl Future<Output = rustradio::Result<T>>,
    stop: &Receiver<()>,
) -> rustradio::Result<Option<T>> {
    let mut future = std::pin::pin!(future);
    let mut stopped = std::pin::pin!(stop.recv());
    poll_fn(|cx| {
        if stopped.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Ok(None));
        }
        future.as_mut().poll(cx).map(|result| result.map(Some))
    })
    .await
}

// A slow UI drops display rows, never stream samples. Keeping this channel
// bounded avoids spawning a pending main-thread send task for every FFT frame.
#[derive(rustradio_macros::Block)]
struct DisplaySink {
    #[rustradio(in)]
    src: NCReadStream<Vec<Float>>,
    frames: Sender<TaggedVec<Float>>,
}
impl Block for DisplaySink {
    fn work(&mut self) -> rustradio::Result<BlockRet<'_>> {
        while let Some((data, tags)) = self.src.pop() {
            let _ = self.frames.try_send(TaggedVec { data, tags });
        }
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}

fn window_chunk(
    mut samples: Vec<Complex>,
    tags: Vec<Tag>,
    window: &[f32],
) -> Vec<(Vec<Complex>, Vec<Tag>)> {
    // A compressed stream can jump across missing samples. Do not draw an FFT
    // assembled from both sides of a gap; its next complete chunk is contiguous.
    if tags.iter().any(|tag| tag.key() == GAP_SAMPLES) {
        return vec![];
    }
    for (sample, weight) in samples.iter_mut().zip(window) {
        *sample *= *weight;
    }
    vec![(samples, tags)]
}

async fn run_graph(settings: Connection, stop: Receiver<()>) -> rustradio::Result<String> {
    let (poke, wake) = async_channel::bounded(1);
    let options = StreamOptions {
        loss_policy: if settings.allow_gaps {
            proto::LossPolicy::AllowGaps
        } else {
            proto::LossPolicy::Lossless
        },
        ..Default::default()
    };
    let Some((source, samples, handle)) = until_stop(
        IqStreamSource::<Complex>::connect(&settings.url, &settings.source, options, poke),
        &stop,
    )
    .await?
    else {
        return Ok("Disconnected".into());
    };
    let sample_rate = handle.description().sample_rate_hz as f32;
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(rustradio::Error::msg(
            "Sample rate is outside the waterfall's range",
        ));
    }
    send_message(WorkerToMain::ApplicationSpecific(AppMessage::Connected {
        sample_rate,
        source: settings.source,
    }))
    .await?;
    let mut graph = rustradio::wasm::wasm_graph::WasmGraph::new();
    graph.add(Box::new(source));
    let (chunks, chunks_out) = StreamChunks::new(samples, FFT_SIZE);
    graph.add(Box::new(chunks));
    let window = rustradio::window::WindowType::Hamming
        .make_window(FFT_SIZE)
        .0;
    // Limit FFT and UI work to roughly 30 rows/s even for a high-rate stream.
    let keep_every = (sample_rate / FFT_SIZE as f32 / ROWS_PER_SECOND)
        .ceil()
        .max(1.0) as usize;
    let mut count = 0;
    let (select, fft_in) = NCMap::new(chunks_out, "waterfall windows", move |samples, tags| {
        count = (count + 1) % keep_every;
        if count == 0 {
            window_chunk(samples, tags, &window)
        } else {
            vec![]
        }
    });
    graph.add(Box::new(select));
    let (fft, fft_out) = Fft::from_fft_size(fft_in, FFT_SIZE)?;
    graph.add(Box::new(fft));
    let (power, power_out) = NCMap::new(fft_out, "FFT power", |bins: Vec<Complex>, tags| {
        vec![(
            bins.iter()
                .map(|bin| 10.0 * (bin.norm_sqr() / FFT_SIZE as f32).max(1.0e-20).log10())
                .collect(),
            tags,
        )]
    });
    graph.add(Box::new(power));
    let (frames, rows) = async_channel::bounded(2);
    let (display_done, display_finished) = async_channel::bounded(1);
    graph.add(Box::new(DisplaySink {
        src: power_out,
        frames,
    }));
    // This task exits when graph teardown drops the last frame sender.
    spawn_local(async move {
        while let Ok(row) = rows.recv().await {
            if send_message(WorkerToMain::Floats(SPECTRUM.into(), vec![row]))
                .await
                .is_err()
            {
                break;
            }
        }
        let _ = display_done.send(()).await;
    });
    let completed = until_stop(graph.run_async(wake), &stop).await;
    // Finish posting this session's rows before End enables another connection.
    // Otherwise an old row could arrive after the UI clears its new waterfall.
    drop(graph);
    let _ = display_finished.recv().await;
    let completed = completed?;
    let lost = handle.lost_samples();
    Ok(if completed.is_some() {
        format!("Stream completed · {lost} missing samples")
    } else {
        format!("Disconnected · {lost} missing samples")
    })
}

pub(crate) async fn setup() -> Result<(), JsValue> {
    rustradio_ui::worker::setup::<AppMessage, AppMessage, _>(|messages| {
        spawn_local(async move {
            let _ = send_message(WorkerToMain::Ready(AppEmpty {})).await;
            while let Ok(message) = messages.recv().await {
                match message {
                    MainToWorker::Start(settings) => {
                        if STOP.with(|slot| slot.borrow().is_some()) {
                            continue;
                        }
                        let (tx, rx) = async_channel::bounded(1);
                        STOP.with(|slot| *slot.borrow_mut() = Some(tx));
                        spawn_local(async move {
                            let result = run_graph(settings, rx).await;
                            STOP.with(|slot| slot.borrow_mut().take());
                            let text = result.unwrap_or_else(|e| format!("Stream failed: {e}"));
                            let _ = send_message(WorkerToMain::End(text)).await;
                        });
                    }
                    MainToWorker::ApplicationSpecific(AppMessage::Stop) => {
                        STOP.with(|slot| {
                            if let Some(tx) = slot.borrow().as_ref() {
                                let _ = tx.try_send(());
                            }
                        });
                    }
                    _ => {}
                }
            }
        });
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gap_windows_are_discarded() {
        let tags = vec![Tag::new(
            1,
            GAP_SAMPLES,
            rustradio::stream::TagValue::U64(12),
        )];
        assert!(window_chunk(vec![Complex::new(1.0, 0.0); 2], tags, &[1.0; 2]).is_empty());
    }
    #[test]
    fn slow_display_does_not_block_graph() -> rustradio::Result<()> {
        let (tx, input) = rustradio::stream::new_nocopy_stream();
        let (frames, rows) = async_channel::bounded(1);
        let mut sink = DisplaySink { src: input, frames };
        tx.push(vec![1.0], &[]);
        tx.push(vec![2.0], &[]);
        assert!(matches!(sink.work()?, BlockRet::WaitForStream(_, 1)));
        assert_eq!(rows.try_recv().unwrap().data, vec![1.0]);
        assert!(rows.try_recv().is_err());
        Ok(())
    }
}
