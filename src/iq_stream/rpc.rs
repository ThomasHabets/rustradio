//! Small native RPC adapter using the same budget-checking decoder as WebSocket.
use super::{Decoded, MAX_CONTROL, MAX_ENVELOPE, proto};
use prost::Message;
use std::marker::PhantomData;
use tonic::codegen::*;
use tonic::{Request, Response, Status};

pub(super) const RPC_PATH: &str = "/rustradio.iq.v1.IqStreaming/Stream";
pub(super) trait Envelope: Message {
    fn is_frame(&self) -> bool;
    fn recycle(self, _server: Option<&super::IqServer>, _pool: &mut Option<super::sink::SamplePool>)
    where
        Self: Sized,
    {
    }
}
impl Envelope for proto::ClientMessage {
    fn is_frame(&self) -> bool {
        matches!(self.body, Some(proto::client_message::Body::Frame(_)))
    }
}
impl Envelope for proto::ServerMessage {
    fn is_frame(&self) -> bool {
        matches!(self.body, Some(proto::server_message::Body::Frame(_)))
    }
    fn recycle(self, server: Option<&super::IqServer>, pool: &mut Option<super::sink::SamplePool>) {
        match self.body {
            Some(proto::server_message::Body::Started(started)) => {
                *pool = server.and_then(|server| {
                    started
                        .description
                        .as_ref()
                        .and_then(|desc| server.sample_pool(&desc.source_id))
                });
            }
            Some(proto::server_message::Body::Frame(proto::Frame {
                body: Some(proto::frame::Body::Chunk(chunk)),
                ..
            })) => {
                if let Some(pool) = pool {
                    pool.put(chunk.samples);
                }
            }
            _ => {}
        }
    }
}

// Use the same pre-allocation budget checks for HTTP/2 gRPC and WebSocket.
// The ordinary prost codec decodes before we can inspect raw Frame byte sizes.
pub(super) struct CheckedCodec<E, D> {
    client_decode: bool,
    server: Option<super::IqServer>,
    marker: PhantomData<(E, D)>,
}
impl<E, D> CheckedCodec<E, D> {
    pub fn new(client_decode: bool) -> Self {
        Self {
            client_decode,
            server: None,
            marker: PhantomData,
        }
    }
}
pub(super) struct Encoder<T> {
    server: Option<super::IqServer>,
    // Started selects the named sink's pool. Once encoding has copied the bytes
    // into tonic's output buffer, its sample allocation can return to that sink.
    pool: Option<super::sink::SamplePool>,
    marker: PhantomData<T>,
}
pub(super) struct Decoder<T> {
    client: bool,
    marker: PhantomData<T>,
}
impl<E: Envelope + Send + 'static, D: Message + Default + Send + 'static> tonic::codec::Codec
    for CheckedCodec<E, D>
{
    type Encode = E;
    type Decode = Decoded<D>;
    type Encoder = Encoder<E>;
    type Decoder = Decoder<D>;
    fn encoder(&mut self) -> Self::Encoder {
        Encoder {
            server: self.server.clone(),
            pool: None,
            marker: PhantomData,
        }
    }
    fn decoder(&mut self) -> Self::Decoder {
        Decoder {
            client: self.client_decode,
            marker: PhantomData,
        }
    }
}
impl<T: Envelope> tonic::codec::Encoder for Encoder<T> {
    type Item = T;
    type Error = Status;
    fn encode(&mut self, item: T, dst: &mut tonic::codec::EncodeBuf<'_>) -> Result<(), Status> {
        if item.encoded_len()
            > if item.is_frame() {
                MAX_ENVELOPE
            } else {
                MAX_CONTROL
            }
        {
            return Err(Status::resource_exhausted("IQ envelope limit"));
        }
        item.encode(dst)
            .map_err(|e| Status::internal(e.to_string()))?;
        item.recycle(self.server.as_ref(), &mut self.pool);
        Ok(())
    }
}
impl<T: Message + Default> tonic::codec::Decoder for Decoder<T> {
    type Item = Decoded<T>;
    type Error = Status;
    fn decode(
        &mut self,
        src: &mut tonic::codec::DecodeBuf<'_>,
    ) -> Result<Option<Self::Item>, Status> {
        use prost::bytes::Buf;
        if src.remaining() > MAX_ENVELOPE {
            return Err(Status::resource_exhausted("IQ envelope limit"));
        }
        let bytes = src.copy_to_bytes(src.remaining());
        super::wire::decode(&bytes, self.client)
            .map(Some)
            .map_err(|e| Status::invalid_argument(e.to_string()))
    }
}
#[derive(Clone)]
pub(super) struct RpcServer(pub super::IqServer);
impl<B> Service<http::Request<B>> for RpcServer
where
    B: Body + Send + 'static,
    B::Error: Into<StdError> + Send + 'static,
{
    type Response = http::Response<tonic::body::Body>;
    type Error = std::convert::Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        struct StreamService(super::IqServer);
        impl tonic::server::StreamingService<Decoded<proto::ClientMessage>> for StreamService {
            type Response = proto::ServerMessage;
            type ResponseStream =
                tokio_stream::wrappers::ReceiverStream<Result<proto::ServerMessage, Status>>;
            type Future = BoxFuture<Response<Self::ResponseStream>, Status>;
            fn call(
                &mut self,
                request: Request<tonic::Streaming<Decoded<proto::ClientMessage>>>,
            ) -> Self::Future {
                let server = self.0.clone();
                let cancel = request
                    .extensions()
                    .get::<tokio::sync::watch::Receiver<bool>>()
                    .cloned();
                Box::pin(async move {
                    let mut input = request.into_inner();
                    let (input_tx, input_rx) = tokio::sync::mpsc::channel(1);
                    let (output_tx, output_rx) = tokio::sync::mpsc::channel(1);
                    tokio::spawn(async move {
                        let pump = tokio::spawn(async move {
                            loop {
                                let message = match input.message().await {
                                    Ok(Some(message)) => Ok(message),
                                    Ok(None) => break,
                                    Err(e) => Err(e),
                                };
                                let failed = message.is_err();
                                if input_tx.send(message).await.is_err() || failed {
                                    break;
                                }
                            }
                        });
                        let cancelled = async {
                            if let Some(mut rx) = cancel {
                                if !*rx.borrow() {
                                    let _ = rx.changed().await;
                                }
                            } else {
                                std::future::pending::<()>().await;
                            }
                        };
                        let result = tokio::select! {
                            result = server.session(input_rx, output_tx.clone()) => result,
                            _ = output_tx.closed() => Err(Status::cancelled("client disconnected")),
                            _ = cancelled => Err(Status::cancelled("server shutdown")),
                        };
                        pump.abort();
                        if let Err(status) = result {
                            super::native::failure(&output_tx, status).await;
                        }
                    });
                    Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                        output_rx,
                    )))
                })
            }
        }
        let server = self.0.clone();
        if req.uri().path() != RPC_PATH {
            return Box::pin(async {
                Ok(http::Response::builder()
                    .status(200)
                    .header("grpc-status", "12")
                    .header("content-type", "application/grpc")
                    .body(tonic::body::Body::empty())
                    .expect("constant response"))
            });
        }
        Box::pin(async move {
            let mut codec = CheckedCodec::<proto::ServerMessage, proto::ClientMessage>::new(true);
            codec.server = Some(server.clone());
            let mut grpc = tonic::server::Grpc::new(codec)
                .max_decoding_message_size(MAX_ENVELOPE)
                .max_encoding_message_size(MAX_ENVELOPE);
            let response = grpc.streaming(StreamService(server), req).await;
            Ok(response)
        })
    }
}
impl tonic::server::NamedService for RpcServer {
    const NAME: &'static str = "rustradio.iq.v1.IqStreaming";
}
