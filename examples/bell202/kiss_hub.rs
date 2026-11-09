//! Packet-aware TCP fan-in/fan-out around a single modem graph generation.
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use rustradio::graph::CancellationToken;

use super::KissIo;

const QUEUE: usize = 64;
const MAX_FRAME: usize = 10_000;
const POLL: Duration = Duration::from_millis(20);
type Frame = Arc<[u8]>;

/// Frame only the wire representation; escaping and command interpretation
/// remain the responsibility of the graph's existing KISS codecs.
#[derive(Default)]
struct Framer(Option<Vec<u8>>);
impl Framer {
    fn push(&mut self, byte: u8) -> Option<Frame> {
        if byte == 0xc0 {
            let old = self.0.replace(Vec::new());
            return old.filter(|v| !v.is_empty()).map(|v| {
                let mut frame = Vec::with_capacity(v.len() + 2);
                frame.push(0xc0);
                frame.extend(v);
                frame.push(0xc0);
                frame.into()
            });
        }
        if let Some(bytes) = &mut self.0 {
            bytes.push(byte);
            if bytes.len() > MAX_FRAME {
                self.0 = None;
            }
        }
        None
    }
}

struct Queues {
    connected: bool,
    tx: VecDeque<Frame>,
    rx: VecDeque<Frame>,
    warned: Option<Instant>,
}
#[derive(Default)]
struct State {
    clients: BTreeMap<u64, Queues>,
    last: u64,
    draining: bool,
    closed: bool,
}
/// Queues belong to one graph generation. Disconnected senders keep their
/// accepted TX frames until drained; received frames are only for live clients.
#[derive(Default)]
struct Hub {
    state: Mutex<State>,
    ready: Condvar,
}
impl Hub {
    fn add(&self, id: u64) {
        self.state.lock().unwrap().clients.insert(
            id,
            Queues {
                connected: true,
                tx: VecDeque::new(),
                rx: VecDeque::new(),
                warned: None,
            },
        );
    }
    fn disconnect(&self, id: u64) {
        let mut state = self.state.lock().unwrap();
        if let Some(client) = state.clients.get_mut(&id) {
            client.connected = false;
            client.rx.clear();
        }
        // Once drainage begins, clients can only join the next generation.
        if !state.clients.values().any(|c| c.connected) {
            state.draining = true;
        }
        self.ready.notify_all();
    }
    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        self.ready.notify_all();
    }
    fn broadcast(&self, frame: Frame) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        for (id, client) in &mut state.clients {
            if !client.connected {
                continue;
            }
            if client.rx.len() < QUEUE {
                client.rx.push_back(frame.clone());
            } else if client
                .warned
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(5))
            {
                log::warn!("KISS client {id} is slow; dropping received packets");
                client.warned = Some(Instant::now());
            }
        }
        Ok(())
    }
    fn io(self: &Arc<Self>) -> KissIo {
        (
            Box::new(HubWriter {
                hub: self.clone(),
                framer: Framer::default(),
            }),
            Box::new(HubReader {
                hub: self.clone(),
                current: None,
            }),
        )
    }
}

struct HubReader {
    hub: Arc<Hub>,
    current: Option<(Frame, usize)>,
}
impl Read for HubReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.current.is_none() {
            let mut state = self.hub.state.lock().unwrap();
            loop {
                if state.closed {
                    return Ok(0);
                }
                state.clients.retain(|_, c| c.connected || !c.tx.is_empty());
                // Rotate at packet boundaries, preserving each sender's FIFO.
                let id = state
                    .clients
                    .range((
                        std::ops::Bound::Excluded(state.last),
                        std::ops::Bound::Unbounded,
                    ))
                    .chain(state.clients.range(..=state.last))
                    .find(|(_, c)| !c.tx.is_empty())
                    .map(|(&id, _)| id);
                if let Some(id) = id {
                    self.current = Some((
                        state.clients.get_mut(&id).unwrap().tx.pop_front().unwrap(),
                        0,
                    ));
                    state.last = id;
                    break;
                }
                if state.draining {
                    return Ok(0);
                }
                let (next, timeout) = self
                    .hub
                    .ready
                    .wait_timeout(state, Duration::from_millis(100))
                    .unwrap();
                state = next;
                if timeout.timed_out() {
                    return Err(io::ErrorKind::TimedOut.into());
                }
            }
        }
        if self.hub.state.lock().unwrap().closed {
            self.current = None;
            return Ok(0);
        }
        let (frame, offset) = self.current.as_mut().unwrap();
        let n = output.len().min(frame.len() - *offset);
        output[..n].copy_from_slice(&frame[*offset..*offset + n]);
        *offset += n;
        if *offset == frame.len() {
            self.current = None;
        }
        Ok(n)
    }
}
struct HubWriter {
    hub: Arc<Hub>,
    framer: Framer,
}
impl Write for HubWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for &byte in bytes {
            if let Some(frame) = self.framer.push(byte) {
                self.hub.broadcast(frame)?;
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Network state is owned by the accept loop; the graph sees only packet queues.
/// Nonblocking writes retain a partial frame until it is completely delivered.
struct Client {
    id: u64,
    stream: TcpStream,
    framer: Framer,
    input: VecDeque<u8>,
    output: Option<(Frame, usize)>,
}
impl Client {
    fn new(id: u64, stream: TcpStream, hub: &Hub) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        hub.add(id);
        Ok(Self {
            id,
            stream,
            framer: Framer::default(),
            input: VecDeque::new(),
            output: None,
        })
    }
    fn poll(&mut self, hub: &Hub) -> io::Result<bool> {
        let room = |hub: &Hub, id| {
            hub.state
                .lock()
                .unwrap()
                .clients
                .get(&id)
                .is_some_and(|c| c.tx.len() < QUEUE)
        };
        if self.input.is_empty() && room(hub, self.id) {
            let mut bytes = [0; 4096];
            match self.stream.read(&mut bytes) {
                Ok(0) => return Ok(false),
                Ok(n) => self.input.extend(&bytes[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        let mut state = hub.state.lock().unwrap();
        let client = state.clients.get_mut(&self.id).unwrap();
        let mut accepted = false;
        while client.tx.len() < QUEUE {
            let Some(byte) = self.input.pop_front() else {
                break;
            };
            if let Some(frame) = self.framer.push(byte) {
                client.tx.push_back(frame);
                accepted = true;
            }
        }
        drop(state);
        if accepted {
            hub.ready.notify_one();
        }
        if self.output.is_none() {
            self.output = hub
                .state
                .lock()
                .unwrap()
                .clients
                .get_mut(&self.id)
                .and_then(|c| c.rx.pop_front())
                .map(|f| (f, 0));
        }
        write_frame(&mut self.stream, &mut self.output)?;
        Ok(true)
    }
}
// Keep an in-flight frame across short writes and WouldBlock. Dropping queued
// frames must never discard the unsent suffix of a frame already on the wire.
fn write_frame(writer: &mut impl Write, output: &mut Option<(Frame, usize)>) -> io::Result<()> {
    if let Some((frame, offset)) = output {
        match writer.write(&frame[*offset..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => *offset += n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
        if *offset == frame.len() {
            *output = None;
        }
    }
    Ok(())
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

/// Run at most one graph generation. Accepting and socket I/O remain responsive
/// while the graph runs or drains; new arrivals during drainage wait in pending.
pub(super) fn serve(
    listener: TcpListener,
    stop: &CancellationToken,
    session: impl Fn(KissIo) -> Result<()> + Sync,
    cancel_graph: impl Fn(),
) -> Result<()> {
    listener.set_nonblocking(true)?;
    std::thread::scope(|scope| -> Result<()> {
        let mut hub: Option<Arc<Hub>> = None;
        let mut graph: Option<std::thread::ScopedJoinHandle<'_, Result<()>>> = None;
        let mut clients = Vec::<Client>::new();
        let mut pending = VecDeque::<TcpStream>::new();
        let mut next_id = 0;
        let session = &session;
        let result = (|| -> Result<()> {
            loop {
                if graph.as_ref().is_some_and(|g| g.is_finished()) {
                    let result = graph.take().unwrap().join();
                    hub.take().unwrap().close();
                    clients.clear();
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => log::warn!("KISS modem graph failed: {e:#}"),
                        Err(e) => log::error!("KISS modem graph panicked: {e:?}"),
                    }
                }
                if stop.is_canceled() {
                    break;
                }
                for _ in 0..16 {
                    match listener.accept() {
                        Ok((stream, address)) => {
                            log::info!("KISS client connected: {address}");
                            pending.push_back(stream);
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::Interrupted
                                    | io::ErrorKind::ConnectionAborted
                                    | io::ErrorKind::ConnectionReset
                            ) =>
                        {
                            continue;
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                if hub
                    .as_ref()
                    .is_none_or(|h| !h.state.lock().unwrap().draining)
                {
                    while let Some(stream) = pending.pop_front() {
                        // A peer may leave while waiting for an old graph to drain.
                        stream.set_nonblocking(true)?;
                        match stream.peek(&mut [0]) {
                            Ok(0) => continue,
                            Err(e)
                                if !matches!(
                                    e.kind(),
                                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                                ) =>
                            {
                                continue;
                            }
                            _ => {}
                        }
                        let shared = hub.get_or_insert_with(|| Arc::new(Hub::default()));
                        match Client::new(next_id, stream, shared) {
                            Ok(client) => clients.push(client),
                            Err(e) => {
                                log::warn!("Failed to set up KISS client: {e}");
                                continue;
                            }
                        }
                        next_id += 1;
                        if graph.is_none() {
                            let io = shared.io();
                            graph = Some(scope.spawn(move || session(io)));
                        }
                    }
                }
                if let Some(shared) = &hub {
                    clients.retain_mut(|client| match client.poll(shared) {
                        Ok(true) => true,
                        result => {
                            if let Err(e) = result {
                                log::debug!("KISS client {} disconnected: {e}", client.id);
                            }
                            shared.disconnect(client.id);
                            false
                        }
                    });
                }
                std::thread::sleep(POLL);
            }
            Ok(())
        })();
        // Force-close the bridge on cancellation or listener failure so any
        // background ReaderSource exits as well. The caller cancels its graph.
        cancel_graph();
        if let Some(shared) = hub {
            shared.close();
        }
        clients.clear();
        for stream in pending {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(graph) = graph {
            match graph.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => log::warn!("KISS modem graph stopped: {e:#}"),
                Err(e) => log::error!("KISS modem graph panicked: {e:?}"),
            }
        }
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Receiver};

    fn frame(value: u8) -> Frame {
        vec![0xc0, 0, value, 0xc0].into()
    }
    fn receive<T>(rx: &Receiver<T>) -> T {
        rx.recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for test worker")
    }
    fn read_frame(client: &mut TcpStream) -> Vec<u8> {
        let mut bytes = vec![0; 4];
        client.read_exact(&mut bytes).unwrap();
        bytes
    }
    fn connect(address: std::net::SocketAddr) -> TcpStream {
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
    }

    #[test]
    fn partial_socket_writes_preserve_frame_boundaries() -> Result<()> {
        struct SlowWriter {
            steps: VecDeque<io::Result<usize>>,
            bytes: Vec<u8>,
        }
        impl Write for SlowWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                let n = self.steps.pop_front().unwrap()?.min(bytes.len());
                self.bytes.extend_from_slice(&bytes[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut writer = SlowWriter {
            steps: [Ok(1), Err(io::ErrorKind::WouldBlock.into()), Ok(1), Ok(2)].into(),
            bytes: Vec::new(),
        };
        let mut output = Some((frame(7), 0));
        for _ in 0..4 {
            write_frame(&mut writer, &mut output)?;
        }
        assert!(output.is_none());
        assert_eq!(writer.bytes, frame(7).to_vec());
        Ok(())
    }

    #[test]
    fn full_tx_queue_backpressures_only_its_client() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let mut sender = connect(listener.local_addr()?);
        let (stream, _) = listener.accept()?;
        let hub = Hub::default();
        let mut client = Client::new(1, stream, &hub)?;
        hub.add(2);
        for n in 0..QUEUE + 1 {
            sender.write_all(&frame(n as u8))?;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while hub.state.lock().unwrap().clients[&1].tx.len() != QUEUE {
            assert!(Instant::now() < deadline);
            assert!(client.poll(&hub)?);
            std::thread::yield_now();
        }
        for _ in 0..3 {
            assert!(client.poll(&hub)?);
        }
        assert_eq!(hub.state.lock().unwrap().clients[&1].tx.len(), QUEUE);
        // Another sender's queue remains available despite client 1's backlog.
        hub.state
            .lock()
            .unwrap()
            .clients
            .get_mut(&2)
            .unwrap()
            .tx
            .push_back(frame(99));
        hub.state
            .lock()
            .unwrap()
            .clients
            .get_mut(&1)
            .unwrap()
            .tx
            .pop_front();
        while hub.state.lock().unwrap().clients[&1].tx.len() != QUEUE {
            assert!(Instant::now() < deadline);
            assert!(client.poll(&hub)?);
            std::thread::yield_now();
        }
        assert_eq!(
            &**hub.state.lock().unwrap().clients[&1].tx.back().unwrap(),
            &*frame(QUEUE as u8)
        );
        Ok(())
    }

    #[test]
    fn framing_is_incremental_and_resynchronizes_after_oversized_packets() {
        let mut a = Framer::default();
        let mut b = Framer::default();
        for byte in [42, 0xc0, 0xc0, 0, 0xdb] {
            assert!(a.push(byte).is_none());
        }
        let second: Vec<_> = [0xc0, 0, 2, 0xc0]
            .into_iter()
            .filter_map(|v| b.push(v))
            .collect();
        assert_eq!(&*second[0], &*frame(2));
        assert!(a.push(0xdc).is_none());
        assert_eq!(&*a.push(0xc0).unwrap(), &[0xc0, 0, 0xdb, 0xdc, 0xc0]);
        for _ in 0..=MAX_FRAME {
            assert!(a.push(1).is_none());
        }
        assert!(a.push(0xc0).is_none());
        assert!(a.push(0).is_none());
        assert_eq!(&*a.push(0xc0).unwrap(), &[0xc0, 0, 0xc0]);
    }

    #[test]
    fn fan_in_is_round_robin_and_drains_disconnected_clients() -> Result<()> {
        let hub = Arc::new(Hub::default());
        hub.add(1);
        hub.add(2);
        {
            let mut state = hub.state.lock().unwrap();
            state
                .clients
                .get_mut(&1)
                .unwrap()
                .tx
                .extend([frame(1), frame(3)]);
            state
                .clients
                .get_mut(&2)
                .unwrap()
                .tx
                .extend([frame(2), frame(4)]);
        }
        hub.disconnect(1);
        assert!(!hub.state.lock().unwrap().draining);
        hub.disconnect(2);
        let (_, mut reader) = hub.io();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        let expected: Vec<_> = (1..=4).flat_map(|n| frame(n).to_vec()).collect();
        assert_eq!(bytes, expected);
        Ok(())
    }

    #[test]
    fn slow_clients_drop_whole_new_packets_without_affecting_fast_clients() -> Result<()> {
        let hub = Arc::new(Hub::default());
        hub.add(1);
        hub.add(2);
        let (mut writer, _reader) = hub.io();
        for n in 0..QUEUE + 3 {
            // Graph WriterSink calls can split a frame at any byte boundary.
            writer.write_all(&frame(n as u8)[..2])?;
            assert!(hub.state.lock().unwrap().clients[&2].rx.is_empty());
            writer.write_all(&frame(n as u8)[2..])?;
            let delivered = hub
                .state
                .lock()
                .unwrap()
                .clients
                .get_mut(&2)
                .unwrap()
                .rx
                .pop_front()
                .unwrap();
            assert_eq!(&*delivered, &*frame(n as u8));
        }
        let state = hub.state.lock().unwrap();
        assert_eq!(state.clients[&1].rx.len(), QUEUE);
        for (n, received) in state.clients[&1].rx.iter().enumerate() {
            assert_eq!(&**received, &*frame(n as u8));
        }
        assert!(state.clients[&1].warned.is_some());
        Ok(())
    }

    #[test]
    fn concurrent_clients_share_graph_and_restart_only_after_drain() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let stop = CancellationToken::new();
        let server_stop = stop.clone();
        let starts = Arc::new(AtomicUsize::new(0));
        let graph_starts = starts.clone();
        let (observed_tx, observed_rx) = mpsc::channel();
        let (draining_tx, draining_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let (done_tx, done_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let result = serve(
                listener,
                &server_stop,
                |(mut writer, mut reader)| {
                    let generation = graph_starts.fetch_add(1, Ordering::SeqCst);
                    let mut framer = Framer::default();
                    let mut buffer = [0; 1024];
                    let mut packets = Vec::new();
                    loop {
                        match reader.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(n) => {
                                for &byte in &buffer[..n] {
                                    if let Some(packet) = framer.push(byte) {
                                        observed_tx.send((generation, packet.to_vec()))?;
                                        packets.push(packet);
                                        // Simulate radio reception after both senders
                                        // have submitted a frame, proving both attached.
                                        if generation != 0 || packets.len() >= 2 {
                                            writer.write_all(&frame(99))?;
                                        }
                                    }
                                }
                            }
                            Err(e) if e.kind() == io::ErrorKind::TimedOut => continue,
                            Err(e) => return Err(e.into()),
                        }
                    }
                    if generation == 0 {
                        draining_tx.send(())?;
                        receive(&release_rx.lock().unwrap());
                    }
                    Ok(())
                },
                || {},
            );
            done_tx.send(result).unwrap();
        });
        let mut first = connect(address);
        let mut second = connect(address);
        first.write_all(&frame(1)[..2])?;
        second.write_all(&frame(2))?;
        first.write_all(&frame(1)[2..])?;
        let mut submitted = [receive(&observed_rx), receive(&observed_rx)];
        submitted.sort_by_key(|(_, f)| f[2]);
        assert_eq!(submitted, [(0, frame(1).to_vec()), (0, frame(2).to_vec())]);
        assert_eq!(read_frame(&mut first), frame(99).to_vec());
        assert_eq!(read_frame(&mut second), frame(99).to_vec());
        drop(first);
        second.write_all(&frame(3))?;
        assert_eq!(receive(&observed_rx), (0, frame(3).to_vec()));
        assert_eq!(read_frame(&mut second), frame(99).to_vec());
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        drop(second);
        receive(&draining_rx);
        let mut third = connect(address);
        third.write_all(&frame(4))?;
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        release_tx.send(())?;
        assert_eq!(receive(&observed_rx), (1, frame(4).to_vec()));
        assert_eq!(read_frame(&mut third), frame(99).to_vec());
        stop.cancel();
        receive(&done_rx)?;
        let mut byte = [0];
        assert_eq!(third.read(&mut byte)?, 0);
        thread.join().unwrap();
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[test]
    fn setup_failure_closes_clients_and_keeps_listening() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let stop = CancellationToken::new();
        let server_stop = stop.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let result = serve(
                listener,
                &server_stop,
                |_| {
                    started_tx.send(())?;
                    anyhow::bail!("simulated device failure")
                },
                || {},
            );
            done_tx.send(result).unwrap();
        });
        for _ in 0..2 {
            let mut client = connect(address);
            receive(&started_rx);
            assert_eq!(client.read(&mut [0])?, 0);
        }
        stop.cancel();
        receive(&done_rx)?;
        thread.join().unwrap();
        Ok(())
    }

    #[test]
    fn idle_listener_does_not_start_graph_and_stops_on_cancel() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let stop = CancellationToken::new();
        stop.cancel();
        serve(listener, &stop, |_| panic!("unexpected graph"), || {})
    }
}
