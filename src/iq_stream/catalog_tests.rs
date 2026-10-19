use super::*;
use crate::stream::{new_nocopy_stream, new_stream};

#[test]
fn catalog_lists_only_this_servers_registered_sinks() -> crate::Result<()> {
    let server = IqServer::new();
    let other = IqServer::new();
    let (_ordinary_tx, _ordinary_rx) = new_stream::<u8>()?;
    assert!(server.streams().is_empty());
    let (_tx, rx) = new_stream::<u8>()?;
    let sink = super::super::IqStreamSink::builder(rx, &server, "bytes", 48_000.).build()?;
    assert!(other.streams().is_empty());
    let entry = server.streams().remove(0);
    assert_eq!(entry.name, "bytes");
    assert_eq!(entry.sample_rate_hz, Some(48_000.));
    assert_eq!(entry.encoding, Some(u8::encoding()));
    assert!(!entry.messages);
    assert!(!entry.ended);
    let message = StreamOptions::default().open::<u8>(&entry.name)?;
    let Some(proto::client_message::Body::Open(open)) = message.body else {
        unreachable!();
    };
    let (connection, started) = server.open(open).unwrap();
    assert_eq!(started.description.unwrap().encoding, entry.encoding);
    drop(connection);
    drop(sink);
    assert!(server.streams()[0].ended);
    Ok(())
}

#[test]
fn catalog_encodings_match_open_capabilities_and_sort_by_name() -> crate::Result<()> {
    let server = IqServer::new();
    let (_real_tx, real) = new_stream::<f32>()?;
    let (_complex_tx, complex) = new_stream::<crate::Complex>()?;
    let (_bytes_tx, bytes) = new_stream::<u8>()?;
    let (_strings_tx, strings) = new_nocopy_stream::<String>();
    let _real_sink = super::super::IqStreamSink::builder(real, &server, "real", 48_000.).build()?;
    let _complex_sink =
        super::super::IqStreamSink::builder(complex, &server, "complex", 96_000.).build()?;
    let _bytes_sink = super::super::IqStreamSink::builder(bytes, &server, "bytes", 1.).build()?;
    let _strings_sink =
        super::super::IqStreamStringSink::builder(strings, &server, "strings", 2.).build()?;
    let catalog = server.catalog();
    assert_eq!(
        catalog
            .streams
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["bytes", "complex", "real", "strings"]
    );
    for (name, encoding, messages) in [
        ("real", f32::encoding(), false),
        ("complex", crate::Complex::encoding(), false),
        ("bytes", u8::encoding(), false),
        ("strings", super::super::string_encoding(), true),
    ] {
        let entry = catalog.streams.iter().find(|s| s.name == name).unwrap();
        assert_eq!(entry.encoding, Some(encoding.clone()));
        assert_eq!(entry.messages, messages);
        let json = serde_json::to_value(entry)?;
        assert_eq!(json["encoding"], serde_json::to_value(encoding)?);
        assert!(json.get("sample_type").is_none());
        assert!(json.get("id").is_none());
    }
    Ok(())
}

#[tokio::test]
async fn catalog_over_http_and_grpc() -> crate::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server = IqServer::new();
    let (_tx, rx) = new_stream::<f32>()?;
    let name = String::from("catalog-f32");
    let _sink = super::super::IqStreamSink::builder(rx, &server, &name, 48_000.).build()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(async move {
        server
            .serve(listener, async {
                let _ = stopped.await;
            })
            .await
    });
    let mut http = tokio::net::TcpStream::connect(addr).await.unwrap();
    http.write_all(b"GET /iq/v1/streams HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost:8080\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    http.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    let headers = response
        .split_once("\r\n\r\n")
        .unwrap()
        .0
        .to_ascii_lowercase();
    assert!(
        headers
            .lines()
            .any(|line| line == "access-control-allow-origin: *")
    );
    let body: serde_json::Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1)?;
    let entry = body
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .unwrap();
    assert_eq!(entry["encoding"], serde_json::to_value(f32::encoding())?);
    assert!(entry.get("sample_type").is_none());
    assert!(entry.get("id").is_none());
    assert!(entry.get("observable").is_none());
    assert!(entry.get("named").is_none());
    assert_eq!(entry["sample_rate_hz"], 48_000.);
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready().await.unwrap();
    let catalog =
        grpc
            .unary(
                tonic::Request::new(proto::ListStreamsRequest {}),
                axum::http::uri::PathAndQuery::from_static(super::super::rpc::LIST_PATH),
                super::super::rpc::CatalogCodec::<
                    proto::ListStreamsRequest,
                    proto::ListStreamsResponse,
                >::default(),
            )
            .await
            .unwrap()
            .into_inner();
    assert!(catalog.streams.iter().any(|s| s.name == name
        && s.sample_rate_hz == Some(48_000.)
        && s.encoding == Some(f32::encoding())));
    assert_eq!(catalog.streams.len(), 1);
    drop(grpc);
    stop.send(()).unwrap();
    serving.await.unwrap()?;
    Ok(())
}
