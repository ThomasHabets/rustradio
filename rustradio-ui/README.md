# Rustradio UI library, running as WASM in the browser.

<https://github.com/ThomasHabets/rustradio>

## Status

This code is in the process of being migrated / cleaned up from
<https://github.com/ThomasHabets/ruwasm>.

This code is a work in progress, and no promises of being backwards
compatible.

## Examples

- [`rtlsdr-fm`](examples/rtlsdr-fm): WebUSB FM receiver with audio and waterfall.
- [`iq-waterfall`](examples/iq-waterfall): experimental WebSocket I/Q client
  with host, port, and source controls and a waterfall display (`unstable`).

## Time sink triggering

`TimeSink` runs continuously by default. Use its Trigger and Trigger level
controls, or configure it after mounting:

```rust,no_run
use rustradio_ui::mainthread::time_sink::{TimeSink, TimeSinkTrigger, TriggerEdge};

# fn configure(sink: &TimeSink) -> rustradio::Result<()> {
sink.set_trigger(Some(TimeSinkTrigger {
    level: 0.0,
    edge: TriggerEdge::Rising,
}))?;
# Ok(())
# }
```

The first input triggers all displayed series. A rising edge crosses from below
its level to that level or above; falling reverses that comparison. The capture
starts at the crossing sample, fills `TimeSinkOptions::max_points` samples, and
holds until another crossing starts a new capture. Crossings during capture are
ignored. No trace is drawn before the first crossing.

Trigger updates must contain equally sized, already aligned series with a fixed
series count. `clear()` rearms detection and allows a new count, keeping the
trigger settings. Pause stops drawing while acquisition continues.
`set_trigger(None)` returns to continuous display.
