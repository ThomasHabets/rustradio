//! Paced complex tone for the WASM I/Q waterfall example.
//! Run: cargo run --features unstable --example iq_waterfall
#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> rustradio::Result<()> {
    use rustradio::blocks::{IqStreamSink, ReaderSource};
    use rustradio::graph::{Graph, GraphRunner};
    use rustradio::iq_stream::IqServer;
    use std::io::Read;

    struct Tone {
        phase: f32,
    }
    impl Read for Tone {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let samples = output.len() / 8;
            for bytes in output[..samples * 8].as_chunks_mut::<8>().0 {
                bytes[..4].copy_from_slice(&(0.5 * self.phase.cos()).to_le_bytes());
                bytes[4..].copy_from_slice(&(0.5 * self.phase.sin()).to_le_bytes());
                self.phase = (self.phase + std::f32::consts::TAU / 8.0) % std::f32::consts::TAU;
            }
            // Pace the reader so the demonstration behaves like a live source.
            std::thread::sleep(std::time::Duration::from_secs_f64(samples as f64 / 48000.0));
            Ok(samples * 8)
        }
    }
    let server = IqServer::new();
    let (source, input) = ReaderSource::<rustradio::Complex>::new(Tone { phase: 0.0 })?;
    let sink = IqStreamSink::builder(input, &server, "iq", 48000.0)
        .blocking(true)
        .build()?;
    let mut graph = Graph::new();
    graph.add(Box::new(source));
    graph.add(Box::new(sink));
    let cancel = graph.cancel_token();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:50051").await?;
    println!("I/Q waterfall: 127.0.0.1:50051, source iq, blocking, 48000 samples/s");
    let running = tokio::task::spawn_blocking(move || graph.run());
    server
        .serve(listener, async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        })
        .await?;
    running
        .await
        .map_err(|e| rustradio::Error::msg(e.to_string()))??;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
