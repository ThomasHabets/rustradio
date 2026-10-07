//! Experimental WebSocket download source, independent of the legacy UI bridge.
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rustradio::block::{Block, BlockEOF, BlockRet};
use rustradio::iq_stream::{
    self, IqSample, SourceHandle, SourceStatus, SourceTransport, StreamOptions, proto,
};
use rustradio::stream::ReadStream;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{CloseEvent, Event, MessageEvent, WebSocket};

fn error(message: impl Into<String>) -> rustradio::Error {
    rustradio::Error::msg(message.into())
}
fn js_error(value: wasm_bindgen::JsValue) -> rustradio::Error {
    error(format!("WebSocket: {value:?}"))
}
fn send_binary(ws: &WebSocket, bytes: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
    // Rust byte slices view WASM memory, which is shared in worker-based UIs.
    // WebSocket.send rejects shared ArrayBufferViews, so copy this small control
    // envelope into a regular JS-owned ArrayBuffer before passing it to the API.
    ws.send_with_js_u8_array(&js_sys::Uint8Array::from(bytes))
}
// Callbacks exist before Started, when the common transport cannot yet be built.
// Install it here after negotiation; before then, messages go to startup_rx.
type TransportSlot = Rc<RefCell<Option<Arc<SourceTransport>>>>;
// Keep JS closures alive for as long as their socket handlers can run. Detach
// them before freeing the closures, including cancellation during connection.
struct Socket {
    ws: WebSocket,
    _open: Closure<dyn FnMut(Event)>,
    _message: Closure<dyn FnMut(MessageEvent)>,
    _error: Closure<dyn FnMut(Event)>,
    _close: Closure<dyn FnMut(CloseEvent)>,
}
impl Drop for Socket {
    fn drop(&mut self) {
        self.ws.set_onopen(None);
        self.ws.set_onmessage(None);
        self.ws.set_onerror(None);
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}
fn fail(
    slot: &TransportSlot,
    startup: &async_channel::Sender<Result<iq_stream::Decoded<proto::ServerMessage>, String>>,
    poke: &async_channel::Sender<()>,
    message: String,
) {
    if let Some(transport) = slot.borrow().as_ref() {
        transport.fail(message);
    } else {
        let _ = startup.try_send(Err(message));
    }
    let _ = poke.try_send(());
}

/// Browser worker client for a native IqStreamSink, using binary protobuf.
///
/// The negotiated sample rate is available from the returned SourceHandle.
/// ALLOW_GAPS must be explicitly selected for nonblocking sinks; gap tags mark
/// discontinuities and stateful DSP must react to them. Pass the sender paired
/// with the receiver given to WasmGraph::run_async so network arrivals wake it.
#[derive(rustradio_macros::Block)]
#[rustradio(noeof)]
pub struct IqStreamSource<T: IqSample> {
    inner: iq_stream::IqStreamSource<T>,
}
impl<T: IqSample> IqStreamSource<T> {
    /// Connect to ws://host:port/iq/v1/stream (or wss:// behind TLS).
    /// The socket lives in worker-local tasks; this graph block remains Send.
    pub async fn connect(
        url: &str,
        source: impl Into<String>,
        options: StreamOptions,
        poke: async_channel::Sender<()>,
    ) -> rustradio::Result<(Self, ReadStream<T>, SourceHandle)> {
        let open = iq_stream::encode_client(&options.open::<T>(source)?)?;
        let ws = WebSocket::new_with_str(url, "rustradio.iq.v1").map_err(js_error)?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
        let (startup_tx, startup_rx) = async_channel::bounded(1);
        let slot: TransportSlot = Rc::new(RefCell::new(None));
        let on_open = Closure::<dyn FnMut(Event)>::new({
            let (ws, slot, startup, poke) =
                (ws.clone(), slot.clone(), startup_tx.clone(), poke.clone());
            move |_| {
                if ws.protocol() != "rustradio.iq.v1" {
                    fail(
                        &slot,
                        &startup,
                        &poke,
                        "server did not negotiate IQ WebSocket subprotocol".into(),
                    );
                    let _ = ws.close();
                } else if let Err(e) = send_binary(&ws, &open) {
                    fail(&slot, &startup, &poke, format!("Open failed: {e:?}"));
                }
            }
        });
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new({
            let (ws, slot, startup, poke) =
                (ws.clone(), slot.clone(), startup_tx.clone(), poke.clone());
            move |event: MessageEvent| {
                let result = (|| {
                    let buffer = event
                        .data()
                        .dyn_into::<js_sys::ArrayBuffer>()
                        .map_err(|_| "WebSocket messages must be binary".to_owned())?;
                    // Check browser-owned bytes before copying into WASM memory;
                    // the shared decoder then checks frame and tag budgets.
                    if buffer.byte_length() > 2 * 1024 * 1024 {
                        return Err("IQ envelope byte limit".to_owned());
                    }
                    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                    let message = iq_stream::decode_server(&bytes).map_err(|e| e.to_string())?;
                    if let Some(transport) = slot.borrow().as_ref() {
                        transport.accept(message).map_err(|e| e.to_string())?;
                    } else {
                        startup
                            .try_send(Ok(message))
                            .map_err(|_| "server sent messages before initial credit".to_owned())?;
                    }
                    Ok::<_, String>(())
                })();
                if let Err(e) = result {
                    fail(&slot, &startup, &poke, e);
                    let _ = ws.close_with_code(1002);
                }
                let _ = poke.try_send(());
            }
        });
        let on_error = Closure::<dyn FnMut(Event)>::new({
            let (slot, startup, poke) = (slot.clone(), startup_tx.clone(), poke.clone());
            move |_| fail(&slot, &startup, &poke, "WebSocket connection failed".into())
        });
        let on_close = Closure::<dyn FnMut(CloseEvent)>::new({
            let (slot, startup, poke) = (slot.clone(), startup_tx.clone(), poke.clone());
            move |event: CloseEvent| {
                if let Some(transport) = slot.borrow().as_ref() {
                    if transport.handle().status() != SourceStatus::Cancelled {
                        if event.code() == 1000 && event.was_clean() {
                            if let Err(e) = transport.finish() {
                                transport.fail(e.to_string());
                            }
                        } else {
                            transport.fail(format!("WebSocket closed with code {}", event.code()));
                        }
                    }
                } else {
                    let _ = startup.try_send(Err("WebSocket closed before Started".into()));
                }
                let _ = poke.try_send(());
            }
        });
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        let socket = Socket {
            ws,
            _open: on_open,
            _message: on_message,
            _error: on_error,
            _close: on_close,
        };
        let first = startup_rx
            .recv()
            .await
            .map_err(|e| error(e.to_string()))?
            .map_err(error)?;
        let started = match first.message.body {
            Some(proto::server_message::Body::Started(started)) => started,
            Some(proto::server_message::Body::Failure(failure)) => {
                return Err(error(failure.message));
            }
            _ => return Err(error("first server message must be Started")),
        };
        let (inner, out, handle, transport) =
            iq_stream::IqStreamSource::from_started(started, &options)?;
        let transport = Arc::new(transport);
        *slot.borrow_mut() = Some(transport.clone());
        // JS handles remain in this worker-local task. The graph owns only the
        // common source and bounded channels, so its Block still satisfies Send.
        wasm_bindgen_futures::spawn_local(async move {
            while let Some(message) = transport.next_control().await {
                let cancelled =
                    matches!(message.body, Some(proto::client_message::Body::Cancel(_)));
                let result = iq_stream::encode_client(&message).and_then(|bytes| {
                    if socket.ws.buffered_amount() as usize + bytes.len() > 64 * 1024 {
                        return Err(error("IQ WebSocket control buffer exceeded"));
                    }
                    send_binary(&socket.ws, &bytes).map_err(js_error)
                });
                if let Err(e) = result {
                    transport.fail(e.to_string());
                    let _ = poke.try_send(());
                    break;
                }
                if cancelled {
                    break;
                }
            }
            drop(socket);
        });
        Ok((Self { inner }, out, handle))
    }
}
impl<T: IqSample> BlockEOF for IqStreamSource<T> {
    fn eof(&mut self) -> bool {
        self.inner.eof()
    }
}
impl<T: IqSample> Block for IqStreamSource<T> {
    fn work(&mut self) -> rustradio::Result<BlockRet<'_>> {
        self.inner.work()
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use prost::Message;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_test::*;
    wasm_bindgen_test_configure!(run_in_browser);

    // Exercise actual browser callbacks and worker-local task ownership without
    // requiring a separate server process for this unit test. Native tests cover
    // the real WebSocket server and the protobuf wire messages independently.
    #[wasm_bindgen(inline_js = "
        let original, socket, sent;
        export function install_fake(started) {
            original = globalThis.WebSocket;
            const initial = Uint8Array.from(started).buffer;
            sent = [];
            globalThis.WebSocket = class {
                constructor(url, protocol) {
                    socket = this; this.protocol = protocol; this.bufferedAmount = 0;
                    queueMicrotask(() => {
                        this.onopen?.(new Event('open'));
                        this.onmessage?.(new MessageEvent('message', {data:initial}));
                    });
                }
                send(bytes) {
                    if (typeof SharedArrayBuffer !== 'undefined' && bytes.buffer instanceof SharedArrayBuffer) {
                        throw new TypeError('WebSocket.send rejects shared buffers');
                    }
                    sent.push(Uint8Array.from(bytes));
                }
                close() {}
            };
        }
        export function restore() { globalThis.WebSocket = original; }
        export function sent_count() { return sent.length; }
        export function sent_message(index) { return sent[index]; }
        export function receive(bytes) {
            socket.onmessage?.(new MessageEvent('message', {data:Uint8Array.from(bytes).buffer}));
        }
        export function finish() {
            socket.onclose?.(new CloseEvent('close', {code:1000, wasClean:true}));
        }
    ")]
    extern "C" {
        fn install_fake(started: &[u8]);
        fn restore();
        fn sent_count() -> u32;
        fn sent_message(index: u32) -> js_sys::Uint8Array;
        fn receive(bytes: &[u8]);
        fn finish();
    }
    async fn wait_sent(count: u32) {
        for _ in 0..100 {
            if sent_count() >= count {
                return;
            }
            wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::UNDEFINED))
                .await
                .unwrap();
        }
        panic!("control task did not send");
    }
    fn send(body: proto::server_message::Body) {
        receive(&proto::ServerMessage { body: Some(body) }.encode_to_vec());
    }
    #[wasm_bindgen_test(async)]
    async fn browser_roundtrip_wakes_graph_preserves_tags_and_waits_for_close() {
        fn assert_send<T: Send>() {}
        assert_send::<IqStreamSource<f32>>();
        let options = StreamOptions::default();
        let started = proto::Started {
            description: Some(proto::StreamDescription {
                encoding: Some(f32::encoding()),
                sample_rate_hz: 48000.0,
                tag_kinds: vec![1, 3, 4, 5, 6],
                ..Default::default()
            }),
            limits: Some(options.limits.clone()),
            loss_policy: options.loss_policy as i32,
            completion_mode: proto::CompletionMode::Accepted as i32,
            initial_credit: None,
        };
        install_fake(
            &proto::ServerMessage {
                body: Some(proto::server_message::Body::Started(started)),
            }
            .encode_to_vec(),
        );
        let (poke, wake) = async_channel::bounded(1);
        let (mut source, output, handle) =
            IqStreamSource::<f32>::connect("ws://example/iq/v1/stream", "test", options, poke)
                .await
                .unwrap();
        wait_sent(2).await; // Open and initial grant.
        let granted = iq_stream::decode_client(&sent_message(1).to_vec()).unwrap();
        assert!(matches!(
            granted.message.body,
            Some(proto::client_message::Body::FlowControl(_))
        ));
        while wake.try_recv().is_ok() {}
        send(proto::server_message::Body::Frame(proto::Frame {
            sequence: 0,
            body: Some(proto::frame::Body::Chunk(proto::SampleChunk {
                first_sample: 0,
                sample_count: 1,
                samples: 0.5f32.to_le_bytes().to_vec(),
                tags: vec![proto::Tag {
                    sample_index: 0,
                    key: "counter".into(),
                    value: Some(proto::TagValue {
                        kind: Some(proto::tag_value::Kind::Uint64Value(u64::MAX)),
                    }),
                    source_id: None,
                }],
            })),
        }));
        assert!(wake.try_recv().is_ok());
        source.work().unwrap();
        let (data, tags) = output.read_buf().unwrap();
        assert_eq!(data.slice(), &[0.5]);
        assert_eq!(tags[0].val(), &rustradio::stream::TagValue::U64(u64::MAX));
        data.consume(1);
        send(proto::server_message::Body::End(proto::End {
            next_sequence: 1,
            next_sample: 1,
            tags: vec![],
        }));
        source.work().unwrap();
        wait_sent(3).await;
        // A credit update can precede Complete; let the task send both.
        let mut last = iq_stream::decode_client(&sent_message(sent_count() - 1).to_vec())
            .unwrap()
            .message;
        if !matches!(last.body, Some(proto::client_message::Body::Complete(_))) {
            wait_sent(sent_count() + 1).await;
            last = iq_stream::decode_client(&sent_message(sent_count() - 1).to_vec())
                .unwrap()
                .message;
        }
        let complete = match last.body.unwrap() {
            proto::client_message::Body::Complete(c) => c,
            _ => panic!("missing Complete"),
        };
        send(proto::server_message::Body::Complete(complete));
        assert_eq!(handle.status(), SourceStatus::Completing);
        finish();
        assert_eq!(handle.status(), SourceStatus::Complete);
        source.work().unwrap();
        restore();
    }
}
