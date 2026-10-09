---
name: rustradio-ui-app
description: Build or modify a WASM application using rustradio-ui, including worker graphs, browser controls, plots, shared-memory bootstrap, and packaging. Use for applications and examples built on this crate; use the new-block skill for standalone processing blocks.
---

# Write a RustRadio UI application

Build the browser application around the existing `rustradio-ui` components.
Every application must support **light and dark mode**. Prefer
**`rustradio::blockchain!`** for graph construction to minimize repetitive code.
Every application must verify at startup that its **HTML and WASM come from the
same application version**.

## Read the closest working example

Resolve these paths from the repository root; the links are relative to this
skill. Read the relevant example before choosing APIs or copying build settings.

- [iq-waterfall](../../../rustradio-ui/examples/iq-waterfall): the smaller reference
  for application messages, worker startup, connecting/stopping/reconnecting,
  bounded display delivery, and a waterfall. Read its `src/lib.rs`,
  `src/mainthread.rs`, `src/worker.rs`, build script, and `.cargo/config.toml`.
- [rtlsdr-fm](../../../rustradio-ui/examples/rtlsdr-fm): the reference for WebUSB,
  audio, multiple displays, graph branches, `blockchain!`, and CSS theme variables.
  Use its hardware-specific code only when the application needs that hardware.
- [UI crate](../../../rustradio-ui/src): verify current `ApplicationSpecific`,
  message, worker, source, and sink APIs. Inspect
  [shared assets](../../../rustradio-ui/assets) before replacing bootstrap or
  component styles.
- [../ruwasm](../../../../ruwasm), if available: a secondary reference for larger
  graph composition, file inputs, and browser smoke testing. Prefer the current
  examples when APIs differ. Its older WebSocket bridge and `DATA_STREAM.md` are
  not the IqStream protocol. Do not depend on this sibling repository at runtime.

For a new application inside this repository, follow the examples' standalone
Cargo workspace layout. Use local crate paths or patches consistently when
working on unpublished APIs. For an external application, use the dependencies
appropriate to that project. Read versions and features from current manifests;
do not copy all of an example's dependencies or SIMD flags automatically.

## Split UI and DSP work

Use `src/lib.rs` for application types and the exported async `start()` entrypoint,
`src/mainthread.rs` for browser controls and displays, and `src/worker.rs` for the
DSP graph. An existing application's equivalent structure is fine.

- Install the panic hook in `start()`. The examples distinguish main thread from
  worker using `web_sys::window().is_none()`; initialize the matching side.
- Define typed startup parameters and application messages implementing
  `rustradio_ui::ApplicationSpecific`. Its associated types describe Start, End,
  Ready, and custom messages. Keep settings and status in these messages rather
  than coupling worker code to DOM elements.
- Initialize worker communication through `worker::setup` and main-thread
  `start_worker`. Use the crate's `send_message` APIs for normal communication;
  do not recreate the channel bootstrap or send before it is initialized.
- Keep DOM and canvas handles on the main thread. Run the graph through
  `WasmGraph::run_async(wake)` in the worker. Use `spawn_local` for browser futures;
  keep JS handles out of graph block fields that must satisfy `Send`.
- Wake the graph when asynchronous input or external readiness changes. Follow
  the source's wake contract rather than repeatedly polling with `Again`.

## Compose graphs with `blockchain!`

Use the macro for sequences of constructors returning `(block, output)`,
including an already-created source tuple. It adds each block and returns the
last output stream. Put `?` on fallible constructors inside the macro.

```rust
use rustradio::blockchain;
use rustradio::blocks::{Fft, StreamChunks};

// `graph` is a WasmGraph; `source` and `samples` came from a complex source.
let bins = blockchain![
    graph,
    prev,
    (source, samples),
    StreamChunks::new(prev, fft_size),
    Fft::from_fft_size(prev, fft_size)?,
];
```

Use explicit `graph.add(Box::new(...))` for terminal sinks, blocks with multiple
outputs, or control handles that the application needs to retain. For a branch,
add a `Tee` explicitly and use `blockchain!` for each linear branch, or follow the
FM example's nested macro pattern. Do not force constructors returning three or
more values into the macro. Prefer named streams over a large manually assembled
sequence of temporary blocks.

Reuse existing processing blocks and display components. If a new processing
block is actually needed, apply [new-block](../new-block/SKILL.md) for its derive,
ports, backpressure, tags, EOF, and tests; app-local derives must not use the main
crate's `crate` option.

## Bound traffic and handle the session lifecycle

- Keep Start disabled until Ready. Prevent overlapping starts, show connecting
  and running status, and make Stop work while opening a source as well as while
  running its graph. Report errors and restore controls on completion or failure.
- Stop and tear down the previous session before starting another. Cancel its
  socket/source, close its channels, and finish or discard queued display updates
  before clearing displays for the next session. Do not let old rows arrive in
  the new session.
- Bound input and display queues. For live displays, limit visualization work
  and drop stale display frames when rendering is slow. Do not spawn an awaiting
  send task per incoming frame: that turns a bounded channel into an unbounded
  collection of tasks. Keep display dropping separate from stream loss policy.
- Use `StreamChunks` when a component needs vectors or FFT windows. Preserve tags
  through conversions; handle gaps explicitly. An FFT window spanning a gap must
  be discarded or reset rather than treating separated samples as contiguous.
- Mount sinks in dedicated DOM elements and update their handles from worker
  messages. `TimeSink::update` accepts multiple tagged float series, with stable
  series ordering; split complex samples into real/imaginary series first.
  Displays do not automatically synchronize remote streams. Use `StreamAlign`
  upstream when inputs need alignment and share a sample rate/position reference.
- For IqStream applications, enable `unstable` in both crates, use the negotiated
  sample rate and encoding, and match the native sink's blocking/loss policy.
  Keep the WebSocket client in the worker, as in iq-waterfall.

## Require light and dark mode

- Set `color-scheme: light dark`. Style application surfaces, text, borders,
  controls, muted/help text, and error states using theme variables overridden by
  `prefers-color-scheme`, or paired values through `light-dark()` where supported.
- Reuse `assets/rustradio.css` before application CSS. Check component styles as
  well as the page background: plot axes, tick labels, traces, status text, logs,
  inputs, disabled controls, and keyboard focus must remain readable in both modes.
- CSS does not recolor pixels already drawn on a canvas. Reuse the sinks' theme
  handling; custom canvases must choose appropriate colors and redraw when the
  browser color preference changes. Verify an existing canvas while switching
  themes, including while the application is paused or idle.
- Respect the system preference by default. A manual theme picker is optional;
  if provided, keep DOM styles and canvas colors consistent with its selection.
  Do not require a picker merely to satisfy support for both modes.

## Build and serve shared WASM memory

- Follow the smaller example's nightly toolchain and `rust-src` setup, `wasm`
  feature, and shared-memory linker configuration and thread-local storage exports. Add SIMD or other
  browser target features only when the selected graph needs them.
- Dependencies must not enable `async-channel/std` for the WASM target. Where
  directly needed, use `async-channel` with `default-features = false`; check the
  resolved feature graph rather than relying on the application's manifest alone.
- Reuse `assets/bootstrap.js`. The compiled module and shared memory must be
  passed through the existing main/worker handshake. Keep the JS package name,
  generated file names, memory limits, and worker stack settings consistent.
- Package the generated JS/WASM, application HTML/JS/CSS, shared bootstrap, and
  shared component CSS. Follow the example's build script; put wasm-pack's profile
  option before Cargo's forwarded feature arguments. Ignore generated output.
- Serve the page and its assets in a secure context with
  `Cross-Origin-Opener-Policy: same-origin` and
  `Cross-Origin-Embedder-Policy: require-corp`. Localhost HTTP is suitable for
  development; ordinary remote HTTP cannot enable shared WASM memory merely by
  adding headers. Use HTTPS and WSS for remote hosting/streaming, with trusted TLS
  certificates. The examples' development servers bind to loopback.
- WebSocket connections do not need ordinary CORS permission headers. Separately
  hosted scripts/WASM or other fetched assets must satisfy the page's cross-origin
  isolation rules. A deployment CSP must permit the chosen connection endpoint.

## Require matching HTML and WASM versions

All applications must check the HTML version against the compiled WASM version
on the main thread before starting the worker or initializing application
controls. This catches stale cached assets and mixed deployments.

- Embed the application's Git version in the packaged HTML, for example
  `<html data-git-version="GIT_VERSION_NOT_SET">`, replacing the placeholder
  during packaging. Compile the same version into the WASM through `build.rs`
  and `cargo:rustc-env=GIT_VERSION=...`, then read it with `env!("GIT_VERSION")`.
  Use the application's version, rather than the `rustradio-ui` crate version.
- Generate both values from the same source state, for example with
  `git describe --tags --dirty --always`. Ensure Git version changes refresh the
  compiled value even when Cargo reuses previous build output.
- Read `data-git-version` from the HTML in the WASM startup code and compare it
  with the compiled version. If the value is missing, still a placeholder, or
  differs, show a visible error and stop initialization. For a mismatch, include
  both versions and suggest reloading or clearing cached assets.
- Follow [ruwasm's HTML](../../../../ruwasm/web/index.html),
  [packaging script](../../../../ruwasm/build-local.sh),
  [build script](../../../../ruwasm/build.rs), and the version check in
  [main-thread setup](../../../../ruwasm/src/mainthread.rs) as a concrete example.
  Implement the check locally; do not require the sibling repository at runtime.

## Validate the application

Run checks from the application directory so its toolchain and `.cargo` settings
apply. Substitute its actual features instead of enabling every feature.

```sh
cargo fmt -- --check
./build-local.sh
```

For an application following iq-waterfall, also use the applicable checks:

```sh
cargo test --features unstable --lib
cargo check --target wasm32-unknown-unknown --features unstable --lib
cargo clippy --target wasm32-unknown-unknown --features unstable --lib --no-deps -- -D warnings
cargo tree --target wasm32-unknown-unknown --features unstable \
    --edges normal,build --invert async-channel --depth 0 --format '{f}'
```

The channel feature output must exclude `std`. Pure DSP tests can run natively;
DOM, JS socket, and bootstrap behavior require a browser. Avoid treating a plain
Cargo check as proof that the shared-memory wasm-pack package starts correctly.

Serve the packaged app with the required headers and check it in a real browser:

- `crossOriginIsolated` and shared memory are available; worker reaches Ready;
  no bootstrap errors, panics, or unexpected console errors occur.
- Matching HTML/WASM versions allow startup. A deliberately mismatched version
  or a missing HTML version shows an error and prevents worker startup.
- Start, streaming, Stop, failure, and reconnect work as applicable. Use a
  deterministic signal to check plot content and sample-rate/frequency axes.
- Light and dark mode work at load and during theme changes, with readable
  controls and canvas content. Check idle/paused rendering and a narrow viewport.
- Slow display delivery keeps queues bounded and does not prevent Stop.

Use the sibling ruwasm smoke test as inspiration when adding browser automation,
not as a required dependency. Test hardware behavior when hardware is available;
otherwise report what was checked and which paths remain unverified. Document
build/serve commands, enabled experimental features, and any deployment headers
in the application's README. Do not publish or deploy as part of app creation
unless the user requests it.
