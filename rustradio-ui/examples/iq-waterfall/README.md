# I/Q waterfall

A small WASM client for a native `IqStreamSink`. Enter a host, port, and source
identifier, then connect to view a waterfall. It uses the experimental
`rustradio-ui::worker::IqStreamSource`, with socket and DSP work in a worker.
There is no WebUSB or audio setup.

## Build and run

Install `wasm-pack` and the nightly Rust toolchain with `rust-src`, then run:

```sh
cd rustradio-ui/examples/iq-waterfall
./build-local.sh
python3 serve.py
```

Open <http://127.0.0.1:8080>. The included server sets the COOP and COEP headers
needed by the shared-memory worker. Build output is in `web-dist/`.
`./build-local.sh release` builds an optimized version. The script enables
`unstable`; direct Cargo or wasm-pack builds must enable it too.

For a test stream, run this from the repository root:

```sh
cargo run --features unstable --example iq_waterfall
```

Use host `127.0.0.1`, port `50051`, source `iq`, and **Blocking (lossless)**.
The test server generates a complex tone at +6 kHz with a 48 ksample/s rate.
Disconnect and reconnect using the same buttons. Stop the server with Ctrl-C.

## Stream settings

- The source identifier must match the name registered by the native sink.
- This viewer accepts **complex float32** samples. The protocol supplies the
  sample rate; the waterfall axis updates automatically after connection.
  Frequencies are relative to the stream center, from −rate/2 to +rate/2.
- Choose the mode matching the sink's `.blocking(bool)`: lossless for `true`,
  allow gaps for `false`. A mismatch is reported by the server.
- TLS selects `wss://`; otherwise the URL is `ws://HOST:PORT/iq/v1/stream`.
  HTTPS pages select TLS by default and require a secure backend connection.
  TLS can be provided by a reverse proxy in front of the native listener.

The graph uses 2048-sample Hamming windows and computes at most about 30 FFT
rows per second. Display queues are bounded and may skip rows if rendering is
slow. FFT windows containing a stream gap are discarded, so a row never mixes
samples from both sides of a discontinuity. The final status reports missing
samples; it does not show gaps as elapsed-time rows. A final partial FFT window
is discarded. This example has no automatic reconnection or recording.
