//! Finite native download through the shared gRPC/WebSocket listener.
//! Run: cargo run --features unstable --example iq_stream
#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> rustradio::Result<()> {
    use rustradio::blocks::{IqStreamSink, IqStreamSource, VectorSink, VectorSource};
    use rustradio::graph::{Graph, GraphRunner};
    use rustradio::iq_stream::{IqServer, StreamOptions};
    use rustradio::stream::{Tag, TagValue};

    let server = IqServer::new();
    let data: Vec<f32> = (0..48000).map(|n| (n as f32 * 0.01).sin()).collect();
    let (source, input) = VectorSource::builder(data)
        .tags(&[Tag::new(123, "marker", TagValue::U64(1))])
        .build()?;
    let sink = IqStreamSink::builder(input, &server, "audio", 48000.0)
        .blocking(true)
        .build()?;
    let mut sender = Graph::new();
    sender.add(Box::new(source));
    sender.add(Box::new(sink));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(async move {
        server
            .serve(listener, async {
                let _ = stopped.await;
            })
            .await
    });

    // Connect before starting the finite graph. Both transports use the same
    // resource ID. A browser would use ws://ADDRESS/iq/v1/stream instead.
    let (source, input, status) = IqStreamSource::<f32>::connect(
        format!("http://{address}"),
        "audio",
        StreamOptions::default(),
    )
    .await?;
    println!("Sample rate: {} Hz", status.description().sample_rate_hz);
    let sink = VectorSink::new(input, 48000);
    let result = sink.hook();
    let mut receiver = Graph::new();
    receiver.add(Box::new(source));
    receiver.add(Box::new(sink));
    let sending = tokio::task::spawn_blocking(move || sender.run());
    tokio::task::spawn_blocking(move || receiver.run())
        .await
        .map_err(|e| rustradio::Error::msg(e.to_string()))??;
    sending
        .await
        .map_err(|e| rustradio::Error::msg(e.to_string()))??;
    println!(
        "Received {} samples; {:?}",
        result.data().samples().len(),
        status.status()
    );
    let _ = stop.send(());
    serving
        .await
        .map_err(|e| rustradio::Error::msg(e.to_string()))??;
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
