//! Serve graph observation points without a Tee or an IQ sink.
//! Run: cargo run --features unstable --example iq_observer
//! An optional first argument selects the listen address (default 127.0.0.1:50051).
#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use rustradio::blocks::{AddConst, NullSink, SignalSourceComplex};
    use rustradio::graph::{Graph, GraphRunner};
    use rustradio::iq_stream::IqServer;

    let server = IqServer::new();
    server.register_all_streams();
    let mut graph = Graph::new();
    let (block, prev) = SignalSourceComplex::new(48_000., 1_000., 1.)?;
    graph.add(Box::new(block));
    let prev = prev.with_name("tone").with_sample_rate(48_000.)?;
    let prev = rustradio::blockchain![
        graph,
        prev,
        AddConst::new(prev, rustradio::Complex::new(0.1, 0.))?,
    ];
    graph.add(Box::new(NullSink::new(
        prev.with_name("offset").with_sample_rate(48_000.)?,
    )));
    let cancel = graph.cancel_token();
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:50051".into());
    let listener = tokio::net::TcpListener::bind(address).await?;
    let address = listener.local_addr()?;
    let graph_task = tokio::task::spawn_blocking(move || graph.run());
    eprintln!("Catalog: http://{address}/iq/v1/streams");
    let result = server
        .serve(listener, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    cancel.cancel();
    graph_task.await??;
    result?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
