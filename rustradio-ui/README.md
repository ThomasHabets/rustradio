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
starts at the crossing sample by default, fills `TimeSinkOptions::max_points`
samples, and
holds while waiting for another crossing and while the replacement capture fills.
The new waveform replaces it when the full window is ready. The first capture is
drawn progressively; no trace is drawn before its crossing. Crossings during
capture are ignored.

Trigger updates must contain equally sized, already aligned series with a fixed
series count. `clear()` rearms detection and allows a new count, keeping the
trigger settings. Pause stops drawing while acquisition continues.
`set_trigger(None)` returns to continuous display.

The Trigger delay control accepts a pretrigger duration such as `1ms`, `500us`,
or `0.01s`. The API equivalent is
`sink.set_trigger_delay(std::time::Duration::from_millis(1))?`. The duration is
rounded to the nearest sample at the current sample rate and reserves that many
samples at the beginning of the capture. The total window length stays fixed;
the delay must leave room for at least the crossing sample. Detection arms once
the history is full. Changing the delay clears and rearms the display; `clear()`
retains it. Sample-rate changes recompute the history length and reject a delay
that no longer fits the window.

The X axis uses milliseconds when the displayed span is shorter than one second;
longer spans use seconds. This changes tick labels and the unit label together.
