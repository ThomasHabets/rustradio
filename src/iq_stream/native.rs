use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{
    State, WebSocketUpgrade,
    ws::{CloseFrame, Message},
};
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::sync::mpsc;
use tonic::{Code, Status};

use super::sink::{Resource, Session};
use super::{
    Decoded, IqSample, IqStreamSource, SourceHandle, StreamOptions, advance, decode_client, err,
    limits_valid, proto,
};

/// Registry and shared HTTP/1 WebSocket + HTTP/2 gRPC server.
///
/// Register sinks before starting their graphs. Run `serve` on the application's
/// Tokio runtime; run a synchronous Graph using `spawn_blocking`. Applications
/// can instead mount `router` in their own HTTP server, including TLS termination.
#[derive(Clone, Default)]
pub struct IqServer {
    resources: Arc<Mutex<HashMap<String, Arc<Resource>>>>,
}
impl IqServer {
    /// Empty named-stream registry.
    pub fn new() -> Self {
        Self::default()
    }
    pub(super) fn register(&self, name: String, resource: Arc<Resource>) -> crate::Result<()> {
        let mut resources = self.resources.lock().unwrap_or_else(|p| p.into_inner());
        if resources.contains_key(&name) {
            return Err(err("duplicate IQ resource name"));
        }
        resources.insert(name, resource);
        Ok(())
    }
    /// Abort all sessions and fail registered graph sinks. Applications mounting
    /// `router` themselves call this alongside their HTTP server shutdown.
    pub fn shutdown(&self) {
        for resource in self
            .resources
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
        {
            let mut state = resource.lock();
            state.stopped = true;
            state.eof = true;
            state.active = None;
            drop(state);
            resource.notify.notify_one();
        }
    }
    pub(super) fn sample_pool(&self, name: &str) -> Option<super::sink::SamplePool> {
        self.resources
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(name)
            .map(|r| r.pool.clone())
    }
    /// HTTP router suitable for mounting in an application-owned server.
    pub fn router(&self) -> axum::Router {
        let grpc = super::rpc::RpcServer(self.clone());
        tonic::service::Routes::new(grpc).into_axum_router()
            .route("/iq/v1/stream", axum::routing::get({
                let server = self.clone();
                move |ws: WebSocketUpgrade, cancel: Option<axum::Extension<tokio::sync::watch::Receiver<bool>>>| {
                    websocket(State(server.clone()), ws, cancel)
                }
            }))
    }
    /// Serve both transports on one listener until shutdown. Shutdown cancels
    /// sessions; it does not imply successful stream completion.
    pub async fn serve(
        &self,
        listener: tokio::net::TcpListener,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> crate::Result<()> {
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let server = self.clone();
        let shutdown = async move {
            shutdown.await;
            server.shutdown();
            let _ = cancel_tx.send(true);
        };
        // Stop idle handshakes as well as sessions already attached to resources.
        let router = self.router().layer(axum::Extension(cancel_rx));
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|e| err(e.to_string()))
    }
    fn open(&self, open: proto::Open) -> Result<(Connection, proto::Started), Status> {
        let limits = open
            .limits
            .ok_or_else(|| Status::invalid_argument("missing limits"))?;
        limits_valid(&limits).map_err(invalid)?;
        if !matches!(open.loss_policy, 1 | 2) {
            return Err(Status::invalid_argument("unspecified loss policy"));
        }
        if open.protocol_version != 1 {
            return Err(Status::invalid_argument("unsupported protocol version"));
        }
        if open.completion_mode != proto::CompletionMode::Accepted as i32 {
            return Err(Status::unimplemented(
                "only ACCEPTED completion is supported",
            ));
        }
        let download = match open.operation {
            Some(proto::open::Operation::Download(download)) => download,
            Some(proto::open::Operation::Upload(_)) => {
                return Err(Status::unimplemented("uploads are not supported"));
            }
            None => return Err(Status::invalid_argument("missing operation")),
        };
        if download.accepted_encodings.is_empty()
            || download.accepted_encodings.iter().any(|e| {
                !(1..=10).contains(&e.component_type)
                    || !matches!(e.layout, 1 | 2)
                    || if matches!(e.component_type, 3 | 4) {
                        e.byte_order != 3
                    } else {
                        !matches!(e.byte_order, 1 | 2)
                    }
            })
        {
            return Err(Status::invalid_argument("invalid encoding capabilities"));
        }
        if download.source.is_empty() {
            return Err(Status::invalid_argument("empty source ID"));
        }
        let resource = self
            .resources
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&download.source)
            .cloned()
            .ok_or_else(|| Status::not_found("unknown source"))?;
        let mut state = resource.lock();
        if state.active.is_some() {
            return Err(Status::already_exists("source has an active client"));
        }
        if state.eof {
            return Err(Status::failed_precondition("source has ended"));
        }
        let policy = if resource.blocking {
            proto::LossPolicy::Lossless
        } else {
            proto::LossPolicy::AllowGaps
        };
        if open.loss_policy != policy as i32 {
            return Err(Status::failed_precondition(
                "loss policy must match sink blocking setting",
            ));
        }
        let encoding = state.description.encoding.as_ref().expect("sink encoding");
        if !download.accepted_encodings.contains(encoding)
            || state
                .description
                .tag_kinds
                .iter()
                .any(|k| !download.accepted_tag_kinds.contains(k))
        {
            return Err(Status::failed_precondition(
                "unsupported encoding or tag kinds",
            ));
        }
        let mut encodings = std::collections::HashSet::new();
        let mut kinds = std::collections::HashSet::new();
        let mut codecs = std::collections::HashSet::new();
        if download
            .accepted_encodings
            .iter()
            .any(|e| !encodings.insert((e.component_type, e.byte_order, e.layout)))
            || download
                .accepted_tag_kinds
                .iter()
                .any(|k| !(1..=16).contains(k) || !kinds.insert(k))
            || download
                .accepted_opaque_codecs
                .iter()
                .any(|k| k.is_empty() || !codecs.insert(k))
        {
            return Err(Status::invalid_argument("invalid capability sets"));
        }
        let limits = proto::Limits {
            max_frame_bytes: limits.max_frame_bytes.min(resource.limits.max_frame_bytes),
            max_in_flight_frames: limits
                .max_in_flight_frames
                .min(resource.limits.max_in_flight_frames),
        };
        state.generation = advance(state.generation, 1).map_err(invalid)?;
        let generation = state.generation;
        let mut description = state.description.clone();
        description.source_sample_offset = Some(state.position);
        state.active = Some(Session {
            generation,
            limits: limits.clone(),
            send_limit: 0,
            next_sequence: 0,
            next_sample: 0,
            cursor: 0,
            queue: Default::default(),
            lost: None,
            identity_pending: true,
            end_sent: false,
        });
        drop(state);
        let connection = Connection {
            resource,
            generation,
        };
        Ok((
            connection,
            proto::Started {
                description: Some(description),
                limits: Some(limits),
                loss_policy: policy as i32,
                completion_mode: proto::CompletionMode::Accepted as i32,
                initial_credit: None,
            },
        ))
    }
    pub(super) async fn session(
        &self,
        mut input: mpsc::Receiver<Result<Decoded<proto::ClientMessage>, Status>>,
        output: mpsc::Sender<Result<proto::ServerMessage, Status>>,
    ) -> Result<(), Status> {
        let first = input
            .recv()
            .await
            .ok_or_else(|| Status::data_loss("missing Open"))??;
        let open = match first.message.body {
            Some(proto::client_message::Body::Open(open)) => open,
            _ => return Err(Status::invalid_argument("first message must be Open")),
        };
        let (connection, started) = self.open(open)?;
        output
            .send(Ok(server(proto::server_message::Body::Started(started))))
            .await
            .map_err(|_| Status::cancelled("client disconnected"))?;
        let reader = async {
            loop {
                let received = input
                    .recv()
                    .await
                    .ok_or_else(|| Status::data_loss("client closed before Complete"))??;
                if let Some(complete) = connection.control(received.message)? {
                    return Ok::<_, Status>(complete);
                }
            }
        };
        let writer = async {
            loop {
                let message = connection.next().await?;
                output
                    .send(Ok(message))
                    .await
                    .map_err(|_| Status::cancelled("client disconnected"))?;
            }
            #[allow(unreachable_code)]
            Ok::<(), Status>(())
        };
        let complete = tokio::select! {
            result = reader => result?,
            result = writer => { result?; return Err(Status::internal("writer ended prematurely")); },
            _ = output.closed() => return Err(Status::cancelled("client disconnected")),
        };
        output
            .send(Ok(server(proto::server_message::Body::Complete(complete))))
            .await
            .map_err(|_| Status::cancelled("client disconnected"))?;
        Ok(())
    }
}
fn invalid(e: crate::Error) -> Status {
    Status::invalid_argument(e.to_string())
}
pub(super) fn server(body: proto::server_message::Body) -> proto::ServerMessage {
    proto::ServerMessage { body: Some(body) }
}
pub(super) async fn failure(
    output: &mpsc::Sender<Result<proto::ServerMessage, Status>>,
    status: Status,
) {
    let mut text = status.message().to_owned();
    if text.len() > 60000 {
        let mut end = 60000;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    let status = Status::new(status.code(), text.clone());
    let message = server(proto::server_message::Body::Failure(proto::Failure {
        grpc_status_code: status.code() as u32,
        message: text,
    }));
    // A peer that stops reading must not retain a failed server task forever.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = output.send(Ok(message)).await;
        let _ = output.send(Err(status)).await;
    })
    .await;
}
struct Connection {
    resource: Arc<Resource>,
    generation: u64,
}
impl Connection {
    fn control(&self, message: proto::ClientMessage) -> Result<Option<proto::Complete>, Status> {
        let mut state = self.resource.lock();
        let session = state
            .active
            .as_mut()
            .filter(|s| s.generation == self.generation)
            .ok_or_else(|| Status::cancelled("session detached"))?;
        match message.body {
            Some(proto::client_message::Body::FlowControl(flow)) => {
                let window = u64::from(session.limits.max_in_flight_frames);
                // Credit only increases. Subtracting the window yields the
                // receiver's claimed retired count, which cannot exceed the
                // number of frames we have submitted. The initial grant is W.
                if flow.send_limit < session.send_limit
                    || flow.send_limit < window
                    || flow.send_limit - window > session.next_sequence
                {
                    return Err(Status::invalid_argument("invalid cumulative credit"));
                }
                session.send_limit = flow.send_limit;
                self.resource.notify.notify_one();
                Ok(None)
            }
            Some(proto::client_message::Body::Complete(complete)) => {
                if !session.end_sent
                    || complete.next_sequence != session.next_sequence
                    || complete.next_sample != session.next_sample
                    || complete.completion_mode != proto::CompletionMode::Accepted as i32
                {
                    return Err(Status::invalid_argument("invalid Complete"));
                }
                Ok(Some(complete))
            }
            Some(proto::client_message::Body::Cancel(_)) => {
                Err(Status::cancelled("client cancelled"))
            }
            Some(proto::client_message::Body::Failure(failure)) => {
                if !(1..=16).contains(&failure.grpc_status_code) {
                    return Err(Status::invalid_argument("invalid failure code"));
                }
                Err(Status::new(
                    Code::from_i32(failure.grpc_status_code as i32),
                    failure.message,
                ))
            }
            _ => Err(Status::invalid_argument("unexpected client message")),
        }
    }
    async fn next(&self) -> Result<proto::ServerMessage, Status> {
        loop {
            let notified = self.resource.notify.notified();
            {
                let state = &mut *self.resource.lock();
                let session = state
                    .active
                    .as_mut()
                    .filter(|s| s.generation == self.generation)
                    .ok_or_else(|| Status::cancelled("session detached"))?;
                if session.next_sequence < session.send_limit
                    && let Some(body) = session.queue.pop_front()
                {
                    let (first, count) = match &body {
                        proto::frame::Body::Chunk(chunk) => {
                            (chunk.first_sample, u64::from(chunk.sample_count))
                        }
                        proto::frame::Body::Gap(gap) => (gap.first_sample, gap.sample_count),
                    };
                    if first != session.next_sample {
                        return Err(Status::internal("sink sample cursor mismatch"));
                    }
                    let frame = proto::Frame {
                        sequence: session.next_sequence,
                        body: Some(body),
                    };
                    if frame.encoded_len() > session.limits.max_frame_bytes as usize {
                        return Err(Status::resource_exhausted("frame exceeds negotiated limit"));
                    }
                    // Record submission before the socket/channel send can
                    // yield; a fast receiver may return credit immediately.
                    session.next_sequence = advance(session.next_sequence, 1).map_err(invalid)?;
                    session.next_sample = advance(session.next_sample, count).map_err(invalid)?;
                    return Ok(server(proto::server_message::Body::Frame(frame)));
                }
                // End consumes no credit, but must follow every retained chunk
                // and accumulated gap. Wait for initial credit even for an empty
                // stream so Started is installed at the receiver before End.
                if state.eof
                    && session.send_limit > 0
                    && !session.end_sent
                    && session.queue.is_empty()
                    && session.lost.is_none()
                {
                    session.end_sent = true;
                    return Ok(server(proto::server_message::Body::End(proto::End {
                        next_sequence: session.next_sequence,
                        next_sample: session.next_sample,
                        tags: vec![],
                    })));
                }
            }
            notified.await;
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        let mut state = self.resource.lock();
        // A disconnected task may finish after another client has connected.
        // Only release the session owned by this connection's generation.
        if state
            .active
            .as_ref()
            .is_some_and(|s| s.generation == self.generation)
        {
            state.active = None;
        }
        drop(state);
        self.resource.notify.notify_one();
    }
}
async fn websocket(
    State(server): State<IqServer>,
    ws: WebSocketUpgrade,
    cancel: Option<axum::Extension<tokio::sync::watch::Receiver<bool>>>,
) -> axum::response::Response {
    if !ws.requested_protocols().any(|p| p == "rustradio.iq.v1") {
        return axum::response::IntoResponse::into_response((
            axum::http::StatusCode::BAD_REQUEST,
            "required subprotocol: rustradio.iq.v1",
        ));
    }
    ws.protocols(["rustradio.iq.v1"])
        .max_message_size(super::MAX_ENVELOPE)
        .max_frame_size(super::MAX_ENVELOPE)
        .max_write_buffer_size(super::MAX_ENVELOPE + 128 * 1024)
        .on_upgrade(move |socket| async move {
            let (mut sink, mut stream) = socket.split();
            let (input_tx, input_rx) = mpsc::channel(1);
            let (output_tx, mut output_rx) = mpsc::channel(1);
            let pump = tokio::spawn(async move {
                while let Some(message) = stream.next().await {
                    let decoded = match message {
                        Ok(Message::Binary(bytes)) => decode_client(&bytes).map_err(invalid),
                        Ok(Message::Ping(_) | Message::Pong(_)) => continue,
                        Ok(Message::Close(_)) => break,
                        Ok(_) => Err(Status::invalid_argument(
                            "WebSocket messages must be binary",
                        )),
                        Err(e) => Err(Status::data_loss(e.to_string())),
                    };
                    let failed = decoded.is_err();
                    if input_tx.send(decoded).await.is_err() || failed {
                        break;
                    }
                }
            });
            let pool_server = server.clone();
            let mut sample_pool = None;
            let session = tokio::spawn(async move {
                let mut cancel = cancel.map(|c| c.0);
                let cancelled = async {
                    if let Some(rx) = cancel.as_mut() {
                        if !*rx.borrow() {
                            let _ = rx.changed().await;
                        }
                    } else {
                        std::future::pending::<()>().await;
                    }
                };
                let result = tokio::select! {
                    result = server.session(input_rx, output_tx.clone()) => result,
                    _ = cancelled => Err(Status::cancelled("server shutdown")),
                };
                if let Err(status) = result {
                    failure(&output_tx, status).await;
                }
            });
            let mut close = 1000;
            while let Some(message) = output_rx.recv().await {
                match message {
                    Ok(message) => {
                        let bytes = message.encode_to_vec();
                        super::rpc::Envelope::recycle(
                            message,
                            Some(&pool_server),
                            &mut sample_pool,
                        );
                        if sink.send(Message::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(status) => {
                        close = if matches!(
                            status.code(),
                            Code::InvalidArgument | Code::DataLoss | Code::ResourceExhausted
                        ) {
                            1002
                        } else {
                            1011
                        };
                        break;
                    }
                }
            }
            let _ = sink
                .send(Message::Close(Some(CloseFrame {
                    code: close,
                    reason: "".into(),
                })))
                .await;
            pump.abort();
            session.abort();
        })
}
impl<T: IqSample> IqStreamSource<T> {
    /// Connect a native gRPC download source. Requires a running Tokio runtime.
    /// The returned handle contains the negotiated rate before the graph starts.
    pub async fn connect(
        uri: impl Into<String>,
        source: impl Into<String>,
        options: StreamOptions,
    ) -> crate::Result<(Self, crate::stream::ReadStream<T>, SourceHandle)> {
        let open = options.open::<T>(source)?;
        let endpoint =
            tonic::transport::Endpoint::from_shared(uri.into()).map_err(|e| err(e.to_string()))?;
        let endpoint = if endpoint.uri().scheme_str() == Some("https") {
            endpoint
                .tls_config(tonic::transport::ClientTlsConfig::new().with_native_roots())
                .map_err(|e| err(e.to_string()))?
        } else {
            endpoint
        };
        let channel = endpoint.connect().await.map_err(|e| err(e.to_string()))?;
        let (tx, rx) = mpsc::channel(1);
        tx.send(open).await.map_err(|e| err(e.to_string()))?;
        let mut grpc = tonic::client::Grpc::new(channel);
        grpc = grpc
            .max_decoding_message_size(super::MAX_ENVELOPE)
            .max_encoding_message_size(super::MAX_ENVELOPE);
        grpc.ready().await.map_err(|e| err(e.to_string()))?;
        let response = grpc
            .streaming(
                tonic::Request::new(tokio_stream::wrappers::ReceiverStream::new(rx)),
                axum::http::uri::PathAndQuery::from_static(super::rpc::RPC_PATH),
                super::rpc::CheckedCodec::<proto::ClientMessage, proto::ServerMessage>::new(false),
            )
            .await
            .map_err(|e| err(e.to_string()))?;
        let mut input = response.into_inner();
        let first = input
            .message()
            .await
            .map_err(|e| err(e.to_string()))?
            .ok_or_else(|| err("missing Started"))?;
        let started = match first.message.body {
            Some(proto::server_message::Body::Started(started)) => started,
            Some(proto::server_message::Body::Failure(f)) => return Err(err(f.message)),
            _ => return Err(err("first server message must be Started")),
        };
        let (block, out, handle, transport) = Self::from_started(started, &options)?;
        let transport = Arc::new(transport);
        tokio::spawn(async move {
            let writer_transport = transport.clone();
            let writer = async move {
                while let Some(message) = writer_transport.next_control().await {
                    let stop = matches!(message.body, Some(proto::client_message::Body::Cancel(_)));
                    tx.send(message).await.map_err(|e| err(e.to_string()))?;
                    if stop {
                        return Err(err("IQ source cancelled"));
                    }
                }
                std::future::pending::<crate::Result<()>>().await
            };
            let reader = async {
                while let Some(message) = input.message().await.map_err(|e| err(e.to_string()))? {
                    transport.accept(message)?;
                }
                transport.finish()
            };
            let result = tokio::select! { result = reader => result, result = writer => result };
            if let Err(error) = result {
                transport.fail(error.to_string());
            }
        });
        Ok((block, out, handle))
    }
}

#[cfg(test)]
mod tests {
    use super::super::IqStreamSink;
    use super::*;
    use crate::block::{Block, BlockRet};
    use crate::stream::{Tag, TagValue, new_stream};

    fn open<T: IqSample>(options: &StreamOptions, name: &str) -> proto::Open {
        match options.open::<T>(name).unwrap().body.unwrap() {
            proto::client_message::Body::Open(open) => open,
            _ => unreachable!(),
        }
    }
    fn credit(connection: &Connection, limit: u64) -> Result<(), Status> {
        connection.control(proto::ClientMessage {
            body: Some(proto::client_message::Body::FlowControl(
                proto::FlowControl { send_limit: limit },
            )),
        })?;
        Ok(())
    }
    #[tokio::test]
    async fn blocking_sink_preserves_input_when_queue_is_full() -> crate::Result<()> {
        let server = IqServer::new();
        let (write, read) = new_stream::<f32>();
        let options = StreamOptions {
            limits: proto::Limits {
                max_frame_bytes: 256,
                max_in_flight_frames: 1,
            },
            ..Default::default()
        };
        let mut sink = IqStreamSink::builder(read, &server, "test", 48000.0)
            .limits(options.limits.clone())
            .build()?;
        let mut input = write.write_buf()?;
        input.slice()[..100].fill(1.0);
        input.produce(100, &[]);
        assert!(matches!(sink.work()?, BlockRet::Pending)); // No client.
        let (connection, _) = server.open(open::<f32>(&options, "test")).unwrap();
        credit(&connection, 1).unwrap();
        assert!(matches!(sink.work()?, BlockRet::Again));
        let remaining = connection.resource.lock().position;
        assert!(remaining < 100);
        assert!(matches!(sink.work()?, BlockRet::Pending));
        assert_eq!(connection.resource.lock().position, remaining);
        let first = connection.next().await.unwrap();
        assert!(matches!(
            first.body,
            Some(proto::server_message::Body::Frame(_))
        ));
        assert!(matches!(sink.work()?, BlockRet::Again));
        assert!(server.open(open::<f32>(&options, "test")).is_err());
        assert!(credit(&connection, 3).is_err()); // Only one frame submitted.
        Ok(())
    }
    #[tokio::test]
    async fn nonblocking_sink_reports_exact_final_gap() -> crate::Result<()> {
        let server = IqServer::new();
        let (write, read) = new_stream::<f32>();
        let options = StreamOptions {
            limits: proto::Limits {
                max_frame_bytes: 256,
                max_in_flight_frames: 1,
            },
            loss_policy: proto::LossPolicy::AllowGaps,
        };
        let mut sink = IqStreamSink::builder(read, &server, "test", 48000.0)
            .blocking(false)
            .limits(options.limits.clone())
            .build()?;
        let (connection, _) = server.open(open::<f32>(&options, "test")).unwrap();
        credit(&connection, 1).unwrap();
        let mut input = write.write_buf()?;
        input.slice()[..100].fill(1.0);
        input.produce(100, &[]);
        drop(write);
        sink.work()?;
        sink.work()?;
        let message = connection.next().await.unwrap();
        let retained = match message.body.unwrap() {
            proto::server_message::Body::Frame(frame) => match frame.body.unwrap() {
                proto::frame::Body::Chunk(chunk) => chunk.sample_count as u64,
                _ => panic!("expected chunk"),
            },
            _ => panic!("expected frame"),
        };
        sink.work()?;
        credit(&connection, 2).unwrap();
        let gap = match connection.next().await.unwrap().body.unwrap() {
            proto::server_message::Body::Frame(frame) => match frame.body.unwrap() {
                proto::frame::Body::Gap(gap) => gap,
                _ => panic!("expected gap"),
            },
            _ => panic!("expected frame"),
        };
        assert_eq!(gap.first_sample, retained);
        assert_eq!(gap.sample_count, 100 - retained);
        let end = connection.next().await.unwrap();
        assert!(matches!(
            end.body,
            Some(proto::server_message::Body::End(proto::End {
                next_sequence: 2,
                next_sample: 100,
                ..
            }))
        ));
        drop(connection);
        assert!(matches!(sink.work()?, BlockRet::EOF));
        Ok(())
    }
    #[tokio::test]
    async fn disconnected_nonblocking_sink_starts_at_current_position() -> crate::Result<()> {
        let server = IqServer::new();
        let (write, read) = new_stream::<f32>();
        let mut sink = IqStreamSink::builder(read, &server, "test", 48000.0)
            .blocking(false)
            .build()?;
        let mut input = write.write_buf()?;
        input.slice()[..10].fill(1.0);
        input.produce(10, &[]);
        sink.work()?;
        let options = StreamOptions {
            loss_policy: proto::LossPolicy::AllowGaps,
            ..Default::default()
        };
        let (connection, started) = server.open(open::<f32>(&options, "test")).unwrap();
        assert_eq!(started.description.unwrap().source_sample_offset, Some(10));
        drop(connection);
        let (connection, started) = server.open(open::<f32>(&options, "test")).unwrap();
        assert_eq!(started.description.unwrap().source_sample_offset, Some(10));
        let mut input = write.write_buf()?;
        input.slice()[0] = 1.0;
        input.produce(1, &[]);
        sink.work()?;
        credit(&connection, 8).unwrap();
        let proto::server_message::Body::Frame(frame) =
            connection.next().await.unwrap().body.unwrap()
        else {
            panic!("expected frame");
        };
        let proto::frame::Body::Chunk(chunk) = frame.body.unwrap() else {
            panic!("expected chunk");
        };
        let position = chunk
            .tags
            .iter()
            .find(|tag| tag.key == super::super::ABSOLUTE_SAMPLE_INDEX)
            .unwrap();
        assert_eq!(position.sample_index, 0);
        assert_eq!(
            position.value.as_ref().unwrap().kind,
            Some(proto::tag_value::Kind::Uint64Value(10))
        );
        Ok(())
    }
    #[tokio::test]
    async fn sink_identification_is_once_per_connection_and_fits_frame() -> crate::Result<()> {
        let server = IqServer::new();
        let (write, read) = new_stream::<f32>();
        let options = StreamOptions {
            limits: proto::Limits {
                max_frame_bytes: 256,
                max_in_flight_frames: 1,
            },
            ..Default::default()
        };
        let mut sink = IqStreamSink::builder(read, &server, "test", 48000.0)
            .limits(options.limits.clone())
            .build()?;
        // Reconnect after two chunks. Identification is independent of the
        // graph's absolute sample position and appears once in each session.
        for connection_index in 0..2 {
            let (connection, _) = server.open(open::<f32>(&options, "test")).unwrap();
            for chunk_index in 0..2 {
                credit(&connection, chunk_index + 1).unwrap();
                let mut input = write.write_buf()?;
                input.slice()[..2].fill(1.0);
                input.produce(2, &[Tag::new(0, "input", TagValue::Bool(true))]);
                assert!(matches!(sink.work()?, BlockRet::Again));
                let message = connection.next().await.unwrap();
                let proto::server_message::Body::Frame(frame) = message.body.unwrap() else {
                    panic!("expected frame");
                };
                assert!(frame.encoded_len() <= options.limits.max_frame_bytes as usize);
                let proto::frame::Body::Chunk(chunk) = frame.body.unwrap() else {
                    panic!("expected chunk");
                };
                assert_eq!(chunk.first_sample, chunk_index * 2);
                assert_eq!(chunk.sample_count, 2);
                let mut expected = if chunk_index == 0 {
                    super::super::sink::identity_tags(0, connection_index * 4, env!("GIT_VERSION"))
                } else {
                    vec![]
                };
                expected.push(super::super::tag_to_wire(
                    &Tag::new(0, "input", TagValue::Bool(true)),
                    chunk.first_sample,
                )?);
                assert_eq!(chunk.tags, expected);
            }
            drop(connection);
        }
        Ok(())
    }
    #[tokio::test]
    async fn identification_cannot_exceed_negotiated_frame_limit() -> crate::Result<()> {
        let server = IqServer::new();
        let (write, read) = new_stream::<f32>();
        let options = StreamOptions {
            limits: proto::Limits {
                max_frame_bytes: 64,
                max_in_flight_frames: 1,
            },
            ..Default::default()
        };
        let mut sink = IqStreamSink::builder(read, &server, "test", 48000.0).build()?;
        let (connection, _) = server.open(open::<f32>(&options, "test")).unwrap();
        let mut input = write.write_buf()?;
        input.slice()[0] = 1.0;
        input.produce(1, &[]);
        assert!(sink.work().is_err());
        assert_eq!(connection.resource.lock().position, 0);
        Ok(())
    }
    struct WsClient(tokio::net::TcpStream);
    impl WsClient {
        async fn connect(address: std::net::SocketAddr) -> crate::Result<Self> {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut stream = tokio::net::TcpStream::connect(address).await?;
            let request = format!(
                "GET /iq/v1/stream HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: rustradio.iq.v1\r\n\r\n"
            );
            stream.write_all(request.as_bytes()).await?;
            let mut response = Vec::new();
            while !response.ends_with(b"\r\n\r\n") {
                response.push(stream.read_u8().await?);
                if response.len() > 4096 {
                    return Err(err("oversized test upgrade response"));
                }
            }
            assert!(response.starts_with(b"HTTP/1.1 101"));
            Ok(Self(stream))
        }
        async fn send(&mut self, message: proto::ClientMessage) -> crate::Result<()> {
            use tokio::io::AsyncWriteExt;
            let bytes = message.encode_to_vec();
            let mask = [1, 2, 3, 4];
            let mut frame = vec![0x82];
            if bytes.len() < 126 {
                frame.push(0x80 | bytes.len() as u8);
            } else if bytes.len() <= u16::MAX as usize {
                frame.push(0xfe);
                frame.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            } else {
                frame.push(0xff);
                frame.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
            }
            frame.extend_from_slice(&mask);
            frame.extend(bytes.into_iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
            self.0.write_all(&frame).await?;
            Ok(())
        }
        async fn receive(&mut self) -> crate::Result<Option<Decoded<proto::ServerMessage>>> {
            use tokio::io::AsyncReadExt;
            let op = self.0.read_u8().await?;
            let size = self.0.read_u8().await?;
            assert_eq!(op & 0x80, 0x80);
            assert_eq!(size & 0x80, 0);
            let size = match size & 127 {
                126 => self.0.read_u16().await? as u64,
                127 => self.0.read_u64().await?,
                n => n as u64,
            };
            if size > super::super::MAX_ENVELOPE as u64 {
                return Err(err("oversized test WebSocket message"));
            }
            let mut bytes = vec![0; size as usize];
            self.0.read_exact(&mut bytes).await?;
            if op & 15 == 8 {
                assert_eq!(&bytes[..2], &1000u16.to_be_bytes());
                return Ok(None);
            }
            assert_eq!(op & 15, 2);
            Ok(Some(super::super::decode_server(&bytes)?))
        }
    }
    #[tokio::test]
    async fn grpc_roundtrip_and_multiple_named_streams() -> crate::Result<()> {
        let server = IqServer::new();
        let (a_write, a_read) = new_stream::<f32>();
        let (b_write, b_read) = new_stream::<crate::Complex>();
        let mut a = IqStreamSink::builder(a_read, &server, "real", 48000.0).build()?;
        let mut b = IqStreamSink::builder(b_read, &server, "iq", 96000.0).build()?;
        let (c_write, c_read) = new_stream::<f32>();
        let mut c = IqStreamSink::builder(c_read, &server, "ws", 24000.0).build()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| err(e.to_string()))?;
        let address = listener.local_addr().map_err(|e| err(e.to_string()))?;
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let task_server = server.clone();
        let server_task = tokio::spawn(async move {
            task_server
                .serve(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
        });
        let (mut source_a, out_a, handle_a) = IqStreamSource::<f32>::connect(
            format!("http://{address}"),
            "real",
            StreamOptions::default(),
        )
        .await?;
        let (mut source_b, out_b, handle_b) = IqStreamSource::<crate::Complex>::connect(
            format!("http://{address}"),
            "iq",
            StreamOptions::default(),
        )
        .await?;
        let mut ws = WsClient::connect(address).await?;
        ws.send(StreamOptions::default().open::<f32>("ws")?).await?;
        assert!(matches!(
            ws.receive().await?.unwrap().message.body,
            Some(proto::server_message::Body::Started(_))
        ));
        ws.send(proto::ClientMessage {
            body: Some(proto::client_message::Body::FlowControl(
                proto::FlowControl { send_limit: 8 },
            )),
        })
        .await?;
        let mut buffer = c_write.write_buf()?;
        buffer.slice()[..2].copy_from_slice(&[4.0, 5.0]);
        buffer.produce(2, &[]);
        drop(c_write);
        let ws_task = tokio::spawn(async move {
            let mut frames = 0;
            let mut final_complete = false;
            while let Some(message) = ws.receive().await? {
                match message.message.body {
                    Some(proto::server_message::Body::Frame(frame)) => {
                        assert_eq!(frame.sequence, 0);
                        let chunk = match frame.body.unwrap() {
                            proto::frame::Body::Chunk(chunk) => chunk,
                            _ => panic!("unexpected gap"),
                        };
                        assert_eq!(chunk.sample_count, 2);
                        assert_eq!(chunk.first_sample, 0);
                        assert_eq!(
                            chunk.samples,
                            [4.0f32.to_le_bytes(), 5.0f32.to_le_bytes()].concat()
                        );
                        frames += 1;
                    }
                    Some(proto::server_message::Body::End(end)) => {
                        assert_eq!(end.next_sample, 2);
                        ws.send(proto::ClientMessage {
                            body: Some(proto::client_message::Body::Complete(proto::Complete {
                                next_sequence: end.next_sequence,
                                next_sample: end.next_sample,
                                completion_mode: proto::CompletionMode::Accepted as i32,
                            })),
                        })
                        .await?;
                    }
                    Some(proto::server_message::Body::Complete(_)) => final_complete = true,
                    other => panic!("unexpected WebSocket response: {other:?}"),
                }
            }
            assert!(final_complete);
            assert_eq!(frames, 1);
            Ok::<_, crate::Error>(())
        });
        let tags = vec![
            Tag::new(1, "flag", TagValue::Bool(false)),
            Tag::new(1, "number", TagValue::U64(u64::MAX)),
        ];
        let mut buffer = a_write.write_buf()?;
        buffer.slice()[..3].copy_from_slice(&[1.0, 2.0, 3.0]);
        buffer.produce(3, &tags);
        drop(a_write);
        let mut buffer = b_write.write_buf()?;
        buffer.slice()[..2].copy_from_slice(&[
            crate::Complex::new(1.0, -1.0),
            crate::Complex::new(2.0, -2.0),
        ]);
        buffer.produce(2, &[]);
        drop(b_write);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                a.work()?;
                b.work()?;
                c.work()?;
                source_a.work()?;
                source_b.work()?;
                if handle_a.status() == super::super::SourceStatus::Complete
                    && handle_b.status() == super::super::SourceStatus::Complete
                    && ws_task.is_finished()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            Ok::<_, crate::Error>(())
        })
        .await
        .map_err(|_| err("roundtrip timed out"))??;
        let (buffer, received_tags) = out_a.read_buf()?;
        assert_eq!(buffer.slice(), &[1.0, 2.0, 3.0]);
        let mut expected = vec![
            Tag::new(
                0,
                "rustradio.software",
                TagValue::String("rustradio".into()),
            ),
            Tag::new(
                0,
                "rustradio.version",
                TagValue::String(env!("CARGO_PKG_VERSION").into()),
            ),
        ];
        if !env!("GIT_VERSION").is_empty() {
            expected.push(Tag::new(
                0,
                "rustradio.git_version",
                TagValue::String(env!("GIT_VERSION").into()),
            ));
        }
        expected.push(Tag::new(
            0,
            super::super::ABSOLUTE_SAMPLE_INDEX,
            TagValue::U64(0),
        ));
        expected.extend(tags);
        assert_eq!(received_tags, expected);
        buffer.consume(3);
        let (buffer, _) = out_b.read_buf()?;
        assert_eq!(
            buffer.slice(),
            &[
                crate::Complex::new(1.0, -1.0),
                crate::Complex::new(2.0, -2.0)
            ]
        );
        buffer.consume(2);
        assert_eq!(handle_b.description().sample_rate_hz, 96000.0);
        ws_task.await.map_err(|e| err(e.to_string()))??;
        let _ = shutdown_tx.send(());
        server_task.await.map_err(|e| err(e.to_string()))??;
        Ok(())
    }
}
