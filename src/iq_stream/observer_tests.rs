use super::*;
use crate::block::Block;
use crate::stream::{StreamWait, Tag, TagValue, new_stream};

fn options() -> StreamOptions {
    StreamOptions {
        loss_policy: proto::LossPolicy::AllowGaps,
        ..Default::default()
    }
}
fn open<T: IqSample>(
    server: &IqServer,
    name: &str,
    options: &StreamOptions,
) -> (Connection, proto::Started) {
    let Some(proto::client_message::Body::Open(open)) = options.open::<T>(name).unwrap().body
    else {
        unreachable!()
    };
    server.open(open).unwrap()
}
fn credit(connection: &Connection, limit: u64) {
    connection
        .control(proto::ClientMessage {
            body: Some(proto::client_message::Body::FlowControl(
                proto::FlowControl { send_limit: limit },
            )),
        })
        .unwrap();
}
async fn next(connection: &Connection) -> proto::server_message::Body {
    tokio::time::timeout(std::time::Duration::from_secs(2), connection.next())
        .await
        .unwrap()
        .unwrap()
        .body
        .unwrap()
}

#[tokio::test]
async fn observer_producer_boundary_tags_offsets_and_outstanding_eof() -> crate::Result<()> {
    let server = IqServer::new();
    let (tx, rx) = new_stream::<f32>()?;
    let id = rx.name();
    let rx = rx.with_name(format!("observer-boundary-{id}"));
    let mut window = tx.write_buf()?;
    window.fill_from_slice(&[10., 20.]);
    window.produce(2, &[]);
    server.register_all_streams();
    let (connection, started) = open::<f32>(&server, &rx.name(), &options());
    let desc = started.description.unwrap();
    assert_eq!(desc.sample_rate_hz, 0.0);
    assert_eq!(desc.source_id, rx.name());
    assert_eq!(desc.source_sample_offset, Some(2));
    assert_eq!(rx.refcount(), 2);
    credit(&connection, 8);
    let (primary, _) = rx.read_buf()?;
    let mut window = tx.write_buf()?;
    window.fill_from_slice(&[30., 40., 50.]);
    drop(tx);
    assert!(!connection.resource.lock().eof);
    window.produce(
        3,
        vec![
            Tag::new(2, "later", TagValue::Bool(true)),
            Tag::new(0, "first", TagValue::Bool(false)),
        ],
    );
    assert!(connection.resource.lock().eof);
    assert_eq!(primary.slice(), &[10., 20.]);
    drop(primary);
    let proto::server_message::Body::Frame(frame) = next(&connection).await else {
        panic!("frame")
    };
    let Some(proto::frame::Body::Chunk(chunk)) = frame.body else {
        panic!("chunk")
    };
    assert_eq!(chunk.sample_count, 3);
    assert_eq!(
        chunk.samples,
        [30f32, 40., 50.]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(chunk.tags.last().unwrap().key, "later");
    assert_eq!(chunk.tags.last().unwrap().sample_index, 2);
    assert!(matches!(
        next(&connection).await,
        proto::server_message::Body::End(proto::End { next_sample: 3, .. })
    ));
    let (primary, _) = rx.read_buf()?;
    assert_eq!(primary.slice(), &[10., 20., 30., 40., 50.]);
    primary.consume(5);
    assert!(rx.eof());
    Ok(())
}

#[tokio::test]
async fn observer_overflow_final_gap_and_reconnect() -> crate::Result<()> {
    let server = IqServer::new();
    server.register_all_streams();
    let (tx, rx) = new_stream::<u8>()?;
    let mut opts = options();
    opts.limits = proto::Limits {
        max_frame_bytes: 256,
        max_in_flight_frames: 1,
    };
    let (connection, _) = open::<u8>(&server, &rx.name(), &opts);
    credit(&connection, 1);
    let mut window = tx.write_buf()?;
    window.slice()[..1000].fill(7);
    window.produce(1000, &[]);
    let proto::server_message::Body::Frame(frame) = next(&connection).await else {
        panic!("frame")
    };
    let Some(proto::frame::Body::Chunk(chunk)) = frame.body else {
        panic!("chunk")
    };
    let retained = chunk.sample_count as u64;
    assert!(retained < 1000);
    credit(&connection, 2);
    let proto::server_message::Body::Frame(frame) = next(&connection).await else {
        panic!("gap frame")
    };
    let Some(proto::frame::Body::Gap(gap)) = frame.body else {
        panic!("gap")
    };
    assert_eq!(
        (gap.first_sample, gap.sample_count),
        (retained, 1000 - retained)
    );
    drop(connection);
    let mut window = tx.write_buf()?;
    window.slice()[..11].fill(0);
    window.produce(11, &[]);
    let (connection, started) = open::<u8>(&server, &rx.name(), &opts);
    assert_eq!(
        started.description.unwrap().source_sample_offset,
        Some(1011)
    );
    drop(tx);
    credit(&connection, 1);
    assert!(matches!(
        next(&connection).await,
        proto::server_message::Body::End(_)
    ));
    assert_eq!(rx.read_buf()?.0.len(), 1011);
    Ok(())
}

#[tokio::test]
async fn observer_failure_does_not_fail_primary_or_reconnect() -> crate::Result<()> {
    let server = IqServer::new();
    server.register_all_streams();
    let (tx, rx) = new_stream::<u8>()?;
    let (connection, _) = open::<u8>(&server, &rx.name(), &options());
    let mut window = tx.write_buf()?;
    window.fill_from_slice(&[1]);
    window.produce(
        1,
        vec![Tag::new(0, super::super::GAP_SAMPLES, TagValue::U64(1))],
    );
    assert!(connection.next().await.is_err());
    assert_eq!(rx.read_buf()?.0.slice(), [1]);
    drop(connection);
    let (connection, _) = open::<u8>(&server, &rx.name(), &options());
    let mut window = tx.write_buf()?;
    window.fill_from_slice(&[0]);
    window.produce(1, &[]);
    assert!(
        connection
            .resource
            .lock()
            .active
            .as_ref()
            .unwrap()
            .failure
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn multiple_servers_and_dropped_window() -> crate::Result<()> {
    let a = IqServer::new();
    a.register_all_streams();
    let b = IqServer::new();
    b.register_all_streams();
    let (tx, rx) = new_stream::<crate::Complex>()?;
    let (ca, _) = open::<crate::Complex>(&a, &rx.name(), &options());
    let (cb, _) = open::<crate::Complex>(&b, &rx.name(), &options());
    let window = tx.write_buf()?;
    drop(tx);
    assert!(!ca.resource.lock().eof);
    drop(window);
    credit(&ca, 8);
    credit(&cb, 8);
    assert!(matches!(
        next(&ca).await,
        proto::server_message::Body::End(_)
    ));
    assert!(matches!(
        next(&cb).await,
        proto::server_message::Body::End(_)
    ));
    assert!(rx.eof());
    Ok(())
}

#[tokio::test]
async fn observer_multiframe_tags_are_rebased_once() -> crate::Result<()> {
    let server = IqServer::new();
    server.register_all_streams();
    let (tx, rx) = new_stream::<f32>()?;
    let mut opts = options();
    opts.limits = proto::Limits {
        max_frame_bytes: 256,
        max_in_flight_frames: 8,
    };
    let (connection, _) = open::<f32>(&server, &rx.name(), &opts);
    credit(&connection, 8);
    let mut window = tx.write_buf()?;
    window.slice()[..300].fill(1.);
    window.produce(
        300,
        vec![
            Tag::new(150, "middle", TagValue::U64(1)),
            Tag::new(299, "last", TagValue::U64(2)),
        ],
    );
    drop(tx);
    let mut cursor = 0;
    let mut ordinary = vec![];
    loop {
        match next(&connection).await {
            proto::server_message::Body::Frame(frame) => {
                let Some(proto::frame::Body::Chunk(chunk)) = frame.body else {
                    panic!("unexpected gap")
                };
                assert_eq!(chunk.first_sample, cursor);
                cursor += u64::from(chunk.sample_count);
                ordinary.extend(
                    chunk
                        .tags
                        .into_iter()
                        .filter(|tag| tag.key == "middle" || tag.key == "last")
                        .map(|tag| (tag.key, tag.sample_index)),
                );
            }
            proto::server_message::Body::End(end) => {
                assert_eq!(end.next_sample, 300);
                break;
            }
            _ => panic!("unexpected message"),
        }
    }
    assert_eq!(cursor, 300);
    assert_eq!(ordinary, vec![("middle".into(), 150), ("last".into(), 299)]);
    Ok(())
}

#[test]
fn catalog_exposed_names_resolve_despite_manual_collisions() -> crate::Result<()> {
    let server = IqServer::new();
    server.register_all_streams();
    let (tx, rx) = new_stream::<u8>()?;
    let id = rx.name();
    let label = format!("collision-{id}");
    let rx = rx.with_name(&label);
    let (_manual_tx, manual_rx) = new_stream::<u8>()?;
    let _sink = super::super::IqStreamSink::builder(manual_rx, &server, &label, 1.).build()?;
    let info = server
        .streams()
        .into_iter()
        .find(|s| s.name.starts_with(&format!("{label}-")))
        .unwrap();
    assert_ne!(info.name, label);
    let (connection, _) = open::<u8>(&server, &info.name, &options());
    drop(connection);
    let Some(proto::client_message::Body::Open(request)) = options().open::<u8>(&id)?.body else {
        unreachable!()
    };
    assert_eq!(
        server.open(request).err().unwrap().code(),
        tonic::Code::NotFound
    );
    let (_bad_tx, bad_rx) = new_stream::<u8>()?;
    let _named_sink = super::super::IqStreamSink::builder(bad_rx, &server, &id, 1.).build()?;
    drop((tx, rx));
    Ok(())
}

async fn numeric_transports<T: IqSample + PartialEq>(values: &[T]) -> crate::Result<()> {
    let server = IqServer::new();
    server.register_all_streams();
    let (tx, rx) = new_stream::<T>()?;
    let name = rx.name();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        server
            .serve(listener, async {
                let _ = stopped.await;
            })
            .await
    });
    let (mut source, out, handle) =
        IqStreamSource::<T>::connect_async(format!("http://{address}"), &name, options()).await?;
    let mut window = tx.write_buf()?;
    window.fill_from_slice(values);
    window.produce(values.len(), &[]);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            source.work().unwrap();
            if out.read_buf().unwrap().0.len() == values.len() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(out.read_buf()?.0.slice(), values);
    drop((source, out, handle));
    let mut ws = super::tests::WsClient::connect(address).await?;
    // Use a fresh stream so the WebSocket handshake is independent of gRPC cancellation.
    let (ws_tx, ws_rx) = new_stream::<T>()?;
    ws.send(options().open::<T>(ws_rx.name())?).await?;
    let Some(proto::server_message::Body::Started(started)) =
        ws.receive().await?.unwrap().message.body
    else {
        panic!("Started")
    };
    assert_eq!(started.description.unwrap().sample_rate_hz, 0.);
    ws.send(proto::ClientMessage {
        body: Some(proto::client_message::Body::FlowControl(
            proto::FlowControl { send_limit: 8 },
        )),
    })
    .await?;
    let mut window = ws_tx.write_buf()?;
    window.fill_from_slice(values);
    window.produce(values.len(), &[]);
    drop(ws_tx);
    let mut encoded = vec![];
    for value in values {
        value.serialize_into(&mut encoded);
    }
    let mut complete = false;
    while let Some(message) = ws.receive().await? {
        match message.message.body.unwrap() {
            proto::server_message::Body::Frame(frame) => {
                let Some(proto::frame::Body::Chunk(chunk)) = frame.body else {
                    panic!("chunk")
                };
                assert_eq!(chunk.samples, encoded);
            }
            proto::server_message::Body::End(end) => {
                ws.send(proto::ClientMessage {
                    body: Some(proto::client_message::Body::Complete(proto::Complete {
                        next_sequence: end.next_sequence,
                        next_sample: end.next_sample,
                        completion_mode: proto::CompletionMode::Accepted as i32,
                    })),
                })
                .await?;
            }
            proto::server_message::Body::Complete(_) => complete = true,
            other => panic!("unexpected message: {other:?}"),
        }
    }
    assert!(complete);
    assert_eq!(rx.read_buf()?.0.slice(), values);
    assert_eq!(ws_rx.read_buf()?.0.slice(), values);
    drop(tx);
    let _ = stop.send(());
    task.await.unwrap()?;
    Ok(())
}

#[tokio::test]
async fn automatic_numeric_types_over_both_transports() -> crate::Result<()> {
    numeric_transports(&[0u8, 128, 255]).await?;
    numeric_transports(&[1f32, -2., 3.5]).await?;
    numeric_transports(&[crate::Complex::new(1., 2.), crate::Complex::new(-3., 4.)]).await?;
    Ok(())
}

#[tokio::test]
async fn observation_does_not_change_concurrent_primary_consumption() -> crate::Result<()> {
    let server = IqServer::new();
    server.register_all_streams();
    let (tx, rx) = new_stream::<u8>()?;
    let (connection, _) = open::<u8>(&server, &rx.name(), &options());
    credit(&connection, 8);
    let producer = std::thread::spawn(move || {
        for value in 0..200u8 {
            let mut window = tx.write_buf().unwrap();
            window.slice()[..128].fill(value);
            window.produce(128, &[]);
        }
    });
    let consumer = std::thread::spawn(move || {
        let mut received = vec![];
        loop {
            let (buffer, _) = rx.read_buf().unwrap();
            let n = buffer.len();
            received.extend_from_slice(buffer.slice());
            buffer.consume(n);
            if rx.eof() {
                break;
            }
            if n == 0 {
                std::thread::yield_now();
            }
        }
        received
    });
    let mut cursor = 0;
    let mut frames = 0;
    loop {
        match next(&connection).await {
            proto::server_message::Body::Frame(frame) => {
                assert_eq!(frame.sequence, frames);
                frames += 1;
                let (first, count) = match frame.body.unwrap() {
                    proto::frame::Body::Chunk(c) => (c.first_sample, u64::from(c.sample_count)),
                    proto::frame::Body::Gap(g) => (g.first_sample, g.sample_count),
                };
                assert_eq!(first, cursor);
                cursor += count;
                credit(&connection, frames + 8);
            }
            proto::server_message::Body::End(end) => {
                assert_eq!(end.next_sample, cursor);
                break;
            }
            _ => panic!("unexpected message"),
        }
    }
    producer.join().unwrap();
    let received = consumer.join().unwrap();
    assert_eq!(cursor, 25600);
    assert_eq!(
        received,
        (0..200u8).flat_map(|v| [v; 128]).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn renamed_streams_download_only_by_current_name() -> crate::Result<()> {
    let server = IqServer::new();
    server.register_all_streams();
    let (_tx, rx) = new_stream::<u8>()?;
    let original = rx.name();
    let (connection, started) = open::<u8>(&server, &original, &options());
    assert_eq!(started.description.unwrap().source_id, original);
    let rx = rx.with_name(format!("renamed-{original}"));
    assert!(server.sample_pool(&original).is_some());
    let request = |name: &str| {
        let Some(proto::client_message::Body::Open(request)) =
            options().open::<u8>(name).unwrap().body
        else {
            unreachable!()
        };
        request
    };
    assert_eq!(
        server.open(request(&original)).err().unwrap().code(),
        tonic::Code::NotFound
    );
    assert_eq!(
        server.open(request(&rx.name())).err().unwrap().code(),
        tonic::Code::AlreadyExists
    );
    drop(connection);
    let (_connection, started) = open::<u8>(&server, &rx.name(), &options());
    assert_eq!(started.description.unwrap().source_id, rx.name());
    assert!(server.sample_pool(&rx.name()).is_some());
    Ok(())
}

#[test]
fn automatic_catalog_lifetime_rates_and_supported_types() -> crate::Result<()> {
    let server = IqServer::new();
    let (tx, rx) = new_stream::<f32>()?;
    let original = rx.name();
    assert!(server.streams().is_empty());
    server.register_all_streams();
    let rx = rx
        .with_sample_rate(48_000.)?
        .with_name(format!("catalog-{original}"));
    let entry = server
        .streams()
        .into_iter()
        .find(|s| s.name == rx.name())
        .unwrap();
    assert_eq!(entry.sample_rate_hz, Some(48_000.));
    assert_eq!(entry.encoding, Some(f32::encoding()));
    assert!(!entry.ended);
    drop(tx);
    assert!(
        server
            .streams()
            .into_iter()
            .find(|s| s.name == rx.name())
            .unwrap()
            .ended
    );
    let name = rx.name();
    drop(rx);
    assert!(server.streams().iter().all(|s| s.name != name));
    let (_tx, custom) = new_stream::<u64>()?;
    let (_tx, messages) = crate::stream::new_nocopy_stream::<String>();
    assert!(
        server
            .streams()
            .iter()
            .all(|s| s.name != custom.name() && s.name != messages.name())
    );
    Ok(())
}

#[test]
fn stale_subscription_cannot_duplicate_a_reconnected_session() -> crate::Result<()> {
    let server = IqServer::new();
    server.register_all_streams();
    let (tx, rx) = new_stream::<u8>()?;
    let (old, _) = open::<u8>(&server, &rx.name(), &options());
    let (_, control) = crate::stream_registry::lookup::<u8>(tx.id()).unwrap();
    let stale: Arc<dyn Observer<u8>> = Arc::new(super::super::observer::Tap {
        resource: old.resource.clone(),
        generation: old.generation,
        _metadata: crate::stream_registry::lookup::<u8>(tx.id()).unwrap().0,
    });
    drop(old);
    // Reproduce the interval between releasing the resource lock and removing
    // an old hook during disconnect, without depending on thread scheduling.
    control.with_state(&mut |observers, _, _| observers.0.push(Arc::downgrade(&stale)));
    let (connection, _) = open::<u8>(&server, &rx.name(), &options());
    let mut window = tx.write_buf()?;
    window.fill_from_slice(&[1, 2, 3]);
    window.produce(3, &[]);
    let state = connection.resource.lock();
    let session = state.active.as_ref().unwrap();
    assert_eq!(session.cursor, 3);
    assert_eq!(session.queue.len(), 1);
    Ok(())
}

#[tokio::test]
async fn observer_shutdown_and_failed_open_leave_primary_usable() -> crate::Result<()> {
    let server = IqServer::new();
    server.register_all_streams();
    let (tx, rx) = new_stream::<u8>()?;
    let Some(proto::client_message::Body::Open(request)) =
        StreamOptions::default().open::<u8>(rx.name())?.body
    else {
        unreachable!()
    };
    assert_eq!(
        server.open(request).err().unwrap().code(),
        tonic::Code::FailedPrecondition
    );
    let (connection, _) = open::<u8>(&server, &rx.name(), &options());
    server.shutdown();
    assert_eq!(
        connection.next().await.err().unwrap().code(),
        tonic::Code::Cancelled
    );
    let Some(proto::client_message::Body::Open(request)) = options().open::<u8>(rx.name())?.body
    else {
        unreachable!()
    };
    assert_eq!(
        server.open(request).err().unwrap().code(),
        tonic::Code::Unavailable
    );
    let mut window = tx.write_buf()?;
    window.fill_from_slice(&[1, 2, 3]);
    window.produce(3, &[]);
    drop(tx);
    let (primary, _) = rx.read_buf()?;
    assert_eq!(primary.slice(), [1, 2, 3]);
    primary.consume(3);
    assert!(rx.eof());
    Ok(())
}

#[test]
fn observer_aliases_do_not_steal_other_process_stream_names() -> crate::Result<()> {
    let server = IqServer::new();
    server.register_all_streams();
    let (_tx, rx) = new_stream::<u8>()?;
    let label = format!("collision-{}", rx.name());
    let rx = rx.with_name(&label);
    let (_tx, other) = new_stream::<u8>()?;
    let alias = format!("{label}-{}", rx.id());
    let other = other.with_name(&alias);
    let (_tx, manual) = new_stream::<u8>()?;
    let _sink = super::super::IqStreamSink::builder(manual, &server, &label, 1.).build()?;
    let entries = server.streams();
    assert_eq!(
        entries.iter().filter(|entry| entry.name == alias).count(),
        1
    );
    let (connection, _) = open::<u8>(&server, &alias, &options());
    assert!(Arc::ptr_eq(
        &connection.resource,
        &server
            .observed
            .lock()
            .unwrap()
            .get(&other.id())
            .unwrap()
            .resource
    ));
    assert!(
        entries
            .iter()
            .any(|entry| entry.name == format!("{alias}-2"))
    );
    Ok(())
}
