use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use log::debug;
use rustradio::Float;
use rustradio::stream::Tag;
use wasm_bindgen::prelude::*;
use web_sys::{
    CanvasRenderingContext2d, Element, Event, HtmlButtonElement, HtmlCanvasElement,
    HtmlInputElement, HtmlSelectElement, MutationObserver, MutationObserverInit,
};

use crate::TaggedVec;
use crate::mainthread::CLASS_SINK;

const CLASS_TIME_SINK: &str = "rr-time-sink-section";

/// Convert browser/DOM failures into the crate-level error type exposed by the
/// time sink API.
fn dom_result<T>(result: Result<T, JsValue>, context: &str) -> rustradio::Result<T> {
    result.map_err(|err| {
        let detail = err.as_string().unwrap_or_else(|| format!("{err:?}"));
        rustradio::Error::msg(format!("{context}: {detail}"))
    })
}

const TIME_SINK_HTML: &str = r#"
<div class="rr-panel-header rr-time-sink-header">
  <div>
    <h2 class="rr-panel-title" data-role="title"></h2>
    <p class="rr-panel-kicker" data-role="subtitle"></p>
  </div>
  <div class="rr-time-sink-controls" aria-label="Time sink controls">
    <label class="rr-time-sink-control-field">
      <span>Y min</span>
      <input data-role="y-min" type="number" step="any" value="-1">
    </label>
    <label class="rr-time-sink-control-field">
      <span>Y max</span>
      <input data-role="y-max" type="number" step="any" value="1">
    </label>
    <button data-role="y-apply" type="button">Apply</button>
    <button data-role="y-zoom-in" type="button">Zoom In</button>
    <button data-role="y-zoom-out" type="button">Zoom Out</button>
    <button data-role="y-auto" type="button">Autoscale Off</button>
    <label class="rr-time-sink-control-field">
      <span>Trigger</span>
      <select data-role="trigger-mode">
        <option value="off">Free running</option>
        <option value="rising">Rising</option>
        <option value="falling">Falling</option>
      </select>
    </label>
    <label class="rr-time-sink-control-field">
      <span>Trigger level</span>
      <input data-role="trigger-level" type="number" step="any" value="0" disabled>
    </label>
    <button data-role="pause" type="button">Pause</button>
  </div>
</div>
<div class="rr-panel-body">
  <canvas class="rr-time-sink-canvas" data-role="canvas"></canvas>
</div>
"#;

const DEFAULT_MAX_GRAPH_POINTS: usize = 10_000;
const AXIS_MARGIN_LEFT: f64 = 56.0;
const AXIS_MARGIN_RIGHT: f64 = 12.0;
const AXIS_MARGIN_TOP: f64 = 12.0;
const AXIS_MARGIN_BOTTOM: f64 = 30.0;
const AXIS_TICK_COUNT: usize = 6;

/// Direction in which the first input must cross the trigger level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerEdge {
    /// Previous sample is below the level and the current sample reaches it.
    Rising,
    /// Previous sample is above the level and the current sample reaches it.
    Falling,
}

/// Level trigger on the first input. The crossing sample starts the capture;
/// `TimeSinkOptions::max_points` determines its length. No pretrigger data is kept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSinkTrigger {
    /// Finite amplitude at which to trigger.
    pub level: Float,
    /// Select rising or falling crossings.
    pub edge: TriggerEdge,
}

/// Options for the time sink.
#[derive(Debug, Clone)]
pub struct TimeSinkOptions {
    pub title: String,
    pub subtitle: String,
    pub y_label: String,
    pub sample_rate: f64,
    /// Samples retained per series, or the capture length in trigger mode.
    pub max_points: usize,

    /// Initial fixed Y range, defaulting to -1..1. Set to None to start with
    /// autoscaling enabled; the Autoscale button can toggle it at runtime.
    pub fixed_range: Option<(f32, f32)>,
}

impl Default for TimeSinkOptions {
    /// Build a generic time sink configuration for callers that only need a
    /// mount point and sample updates.
    fn default() -> Self {
        Self {
            title: "Time Sink".into(),
            subtitle: "Float stream amplitude over time".into(),
            y_label: "Amplitude".into(),
            sample_rate: 1.0,
            max_points: DEFAULT_MAX_GRAPH_POINTS,
            fixed_range: Some((-1.0, 1.0)),
        }
    }
}

/// Time sink. This is a handle to a graph element where samples are shown on a
/// normal X axis being time and Y axis being value.
///
/// The time sink can graph multiple series at the same time, e.g. two lines for
/// a complex signal.
#[derive(Clone)]
pub struct TimeSink {
    inner: Rc<RefCell<Inner>>,
}

impl TimeSink {
    /// Find a mount element by ID and replace its contents with a time sink.
    ///
    /// The root element with this browser DOM ID is where more elements will be
    /// created, one of which will be a canvas where the graph is drawn.
    pub fn mount_by_id(id: &str, options: TimeSinkOptions) -> rustradio::Result<Self> {
        let root = dom_result(
            get_element_by_id(id),
            &format!("finding time sink mount element {id}"),
        )?;
        Self::mount(&root, options)
    }

    /// Mount a self-contained time sink into an existing DOM element.
    ///
    /// The root element with this browser DOM ID is where more elements will be
    /// created, one of which will be a canvas where the graph is drawn.
    pub fn mount(root: &Element, options: TimeSinkOptions) -> rustradio::Result<Self> {
        dom_result(Self::mount_dom(root, options), "mounting time sink")
    }

    /// Mount the generated DOM and wire handlers, preserving browser-native
    /// errors until the public API boundary.
    fn mount_dom(root: &Element, options: TimeSinkOptions) -> Result<Self, JsValue> {
        root.set_inner_html(TIME_SINK_HTML);
        root.class_list().add_2(CLASS_SINK, CLASS_TIME_SINK)?;

        role::<Element>(root, "title")?.set_text_content(Some(&options.title));
        role::<Element>(root, "subtitle")?.set_text_content(Some(&options.subtitle));

        let canvas = role::<HtmlCanvasElement>(root, "canvas")?;
        let ctx = canvas
            .get_context("2d")?
            .ok_or(JsValue::from_str("no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;

        let (y_min, y_max) = options.fixed_range.unwrap_or((-1.0, 1.0));

        let inner = Rc::new(RefCell::new(Inner {
            canvas,
            ctx,
            y_min_input: role::<HtmlInputElement>(root, "y-min")?,
            y_max_input: role::<HtmlInputElement>(root, "y-max")?,
            y_apply_button: role::<HtmlButtonElement>(root, "y-apply")?,
            y_zoom_in_button: role::<HtmlButtonElement>(root, "y-zoom-in")?,
            y_zoom_out_button: role::<HtmlButtonElement>(root, "y-zoom-out")?,
            y_auto_button: role::<HtmlButtonElement>(root, "y-auto")?,
            pause_button: role::<HtmlButtonElement>(root, "pause")?,
            data: TimeData::new(options.max_points.max(1)),
            trigger_mode: role::<HtmlSelectElement>(root, "trigger-mode")?,
            trigger_level_input: role::<HtmlInputElement>(root, "trigger-level")?,
            trigger_level: 0.0,
            y_min,
            y_max,
            auto_scale: options.fixed_range.is_none(),
            paused: false,
            sample_rate: options.sample_rate,
            y_label: options.y_label,
            sync_inputs: true,
            callbacks: Vec::new(),
            theme_observer: None,
        }));

        let sink = Self { inner };
        sink.install_handlers()?;
        sink.draw()?;
        Ok(sink)
    }

    /// Add new tagged float streams to the sink and redraw unless paused.
    ///
    /// In order to graph a complex signal, first split it into two Float
    /// streams. With triggering enabled, all series must be already aligned and
    /// have equal lengths in each update. Their count must remain fixed until
    /// clear() or a trigger configuration change. Invalid updates change no state.
    /// A completed capture stays visible while waiting and while its replacement
    /// fills. The first capture is drawn progressively from its crossing sample.
    #[allow(clippy::needless_pass_by_value)]
    pub fn update(&self, streams: Vec<TaggedVec<Float>>) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        let changed = inner.data.append_streams(&streams)?;
        let result = if inner.paused || !changed {
            inner.sync_controls()
        } else {
            inner.draw()
        };
        dom_result(result, "updating time sink")
    }

    /// Set or disable level triggering. `None` selects the default free-running
    /// mode. Changing settings clears the display and rearms detection, including
    /// while paused; identical settings leave the current capture intact.
    /// Nonfinite levels are rejected without changing the configuration.
    pub fn set_trigger(&self, trigger: Option<TimeSinkTrigger>) -> rustradio::Result<()> {
        dom_result(
            self.inner.borrow_mut().configure_trigger(trigger),
            "setting time sink trigger",
        )
    }

    /// Return the current trigger, or `None` for free-running mode.
    pub fn trigger(&self) -> Option<TimeSinkTrigger> {
        self.inner.borrow().data.trigger
    }

    /// Set the sample rate used to convert sample indexes into seconds.
    ///
    /// This will affect the X axis labels.
    pub fn set_sample_rate(&self, sample_rate: f64) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.set_sample_rate(sample_rate);
        let result = if inner.paused {
            inner.sync_controls()
        } else {
            inner.draw()
        };
        dom_result(result, "setting time sink sample rate")
    }

    /// Pause or resume drawing through the API, matching the UI button state.
    pub fn set_paused(&self, paused: bool) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.paused = paused;
        inner.sync_inputs = true;
        let result = if inner.paused {
            inner.sync_controls()
        } else {
            inner.draw()
        };
        dom_result(result, "setting time sink pause state")
    }

    /// Return whether incoming updates are currently buffered without redraw.
    pub fn paused(&self) -> bool {
        self.inner.borrow().paused
    }

    /// Drop all buffered series data and redraw the empty sink.
    pub fn clear(&self) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.data.clear();
        dom_result(inner.draw(), "clearing time sink")
    }

    /// Redraw the current sink state.
    fn draw(&self) -> Result<(), JsValue> {
        self.inner.borrow_mut().draw()
    }

    /// Install all generated control callbacks for this sink instance.
    fn install_handlers(&self) -> Result<(), JsValue> {
        let inner = self.inner.clone();
        let button = inner.borrow().y_apply_button.clone();
        install_button_handler(&inner, &button, |inner| {
            let y_min = Inner::parse_y_input(&inner.y_min_input, "Y min")?;
            let y_max = Inner::parse_y_input(&inner.y_max_input, "Y max")?;
            if !y_min.is_finite() || !y_max.is_finite() || y_min >= y_max {
                return Err(JsValue::from_str("invalid Y min/max range"));
            }
            inner.y_min = y_min;
            inner.y_max = y_max;
            inner.auto_scale = false;
            inner.sync_inputs = false;
            inner.draw()
        })?;

        let inner = self.inner.clone();
        let button = inner.borrow().y_zoom_in_button.clone();
        install_button_handler(&inner, &button, |inner| inner.zoom_y(0.8))?;

        let inner = self.inner.clone();
        let button = inner.borrow().y_zoom_out_button.clone();
        install_button_handler(&inner, &button, |inner| inner.zoom_y(1.25))?;

        let inner = self.inner.clone();
        let button = inner.borrow().y_auto_button.clone();
        install_button_handler(&inner, &button, |inner| {
            inner.auto_scale = !inner.auto_scale;
            inner.sync_inputs = true;
            inner.draw()
        })?;

        let inner = self.inner.clone();
        let button = inner.borrow().pause_button.clone();
        install_button_handler(&inner, &button, |inner| {
            inner.paused = !inner.paused;
            inner.sync_inputs = true;
            if inner.paused {
                inner.sync_controls()
            } else {
                inner.draw()
            }
        })?;

        let controls: [Element; 2] = {
            let inner = self.inner.borrow();
            [
                inner.trigger_mode.clone().unchecked_into(),
                inner.trigger_level_input.clone().unchecked_into(),
            ]
        };
        for element in controls {
            let state = self.inner.clone();
            let handler = Closure::<dyn FnMut(Event)>::new(move |_| {
                let mut inner = state.borrow_mut();
                if let Err(err) = inner.apply_trigger_controls() {
                    log::error!("time sink trigger failed: {err:?}");
                    // Restore valid controls after a rejected edit.
                    inner.sync_inputs = true;
                    let _ = inner.sync_controls();
                }
            });
            element.add_event_listener_with_callback("change", handler.as_ref().unchecked_ref())?;
            self.inner.borrow_mut().callbacks.push(handler);
        }

        let inner = self.inner.clone();
        let handler = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            if let Err(err) = inner.borrow_mut().draw() {
                log::error!("time sink resize failed: {err:?}");
            }
        });
        let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
        window.add_event_listener_with_callback("resize", handler.as_ref().unchecked_ref())?;
        self.inner.borrow_mut().callbacks.push(handler);

        // Redraw stored samples even when paused or waiting for a trigger;
        // CSS alone cannot recolor the pixels already painted on a canvas.
        if let Some(media) = window.match_media("(prefers-color-scheme: dark)")? {
            let inner = Rc::downgrade(&self.inner);
            let handler = Closure::<dyn FnMut(Event)>::new(move |_| {
                if let Some(inner) = inner.upgrade()
                    && let Err(err) = inner.borrow_mut().draw()
                {
                    log::error!("time sink theme redraw failed: {err:?}");
                }
            });
            media.add_event_listener_with_callback("change", handler.as_ref().unchecked_ref())?;
            self.inner.borrow_mut().callbacks.push(handler);
        }
        // Applications may override the system preference with data-theme on
        // the document root. Observe that choice without requiring new samples.
        if let Some(root) = window.document().and_then(|doc| doc.document_element()) {
            let inner = Rc::downgrade(&self.inner);
            let callback = Closure::<dyn FnMut()>::new(move || {
                if let Some(inner) = inner.upgrade()
                    && let Err(err) = inner.borrow_mut().draw()
                {
                    log::error!("time sink theme redraw failed: {err:?}");
                }
            });
            let observer = MutationObserver::new(callback.as_ref().unchecked_ref())?;
            let options = MutationObserverInit::new();
            options.set_attributes(true);
            options.set_attribute_filter(&js_sys::Array::of1(&JsValue::from_str("data-theme")));
            observer.observe_with_options(&root, &options)?;
            self.inner.borrow_mut().theme_observer = Some(ThemeObserver {
                observer,
                _callback: callback,
            });
        }

        Ok(())
    }
}

/// Keep the observer callback alive and detach it when its owner is dropped.
struct ThemeObserver {
    observer: MutationObserver,
    _callback: Closure<dyn FnMut()>,
}
impl Drop for ThemeObserver {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}

struct GraphSeries {
    // The absolute stream pos of the first value.
    start_index: u64,
    samples: VecDeque<f32>,

    // Tag positions are in absolute stream value.
    tags: Vec<Tag>,
}

impl GraphSeries {
    /// Create one buffered plotted series with room for the first update.
    fn new(capacity: usize) -> Self {
        Self {
            start_index: 0,
            samples: VecDeque::with_capacity(capacity),
            tags: Vec::new(),
        }
    }

    /// Append one tagged stream and trim old samples beyond the retention cap.
    fn append_stream(&mut self, stream: &TaggedVec<Float>, max_points: usize) {
        self.tags.extend(stream.tags.iter().map(|t| {
            Tag::new(
                ((t.pos() as u64) + self.start_index + (self.samples.len() as u64)) as _,
                t.key(),
                t.val().clone(),
            )
        }));
        self.samples.extend(stream.data.iter().copied());
        while self.samples.len() > max_points {
            self.samples.pop_front();
            self.start_index = self.start_index.saturating_add(1);
        }
    }
}

// Buffering is independent of DOM handles so edge detection and capture ranges
// can be tested natively. `remaining` is Some only while a capture is filling;
// waiting retains the previous completed window. Acquisition uses a separate
// buffer so a new partial capture cannot erase the completed waveform.
struct TimeData {
    series: Vec<GraphSeries>,
    capture: Vec<GraphSeries>,
    max_points: usize,
    trigger: Option<TimeSinkTrigger>,
    previous: Option<Float>,
    remaining: Option<usize>,
    series_count: Option<usize>,
}
impl TimeData {
    /// Start in the existing free-running mode with bounded sample retention.
    fn new(max_points: usize) -> Self {
        Self {
            series: Vec::new(),
            capture: Vec::new(),
            max_points: max_points.max(1),
            trigger: None,
            previous: None,
            remaining: None,
            series_count: None,
        }
    }
    /// Forget captures and detector history, retaining the selected trigger.
    fn clear(&mut self) {
        self.series.clear();
        self.capture.clear();
        self.previous = None;
        self.remaining = None;
        self.series_count = None;
    }
    /// Keep the latest complete waveform visible while its replacement fills.
    /// Before the first completion, show the initial capture progressively.
    fn visible_series(&self) -> &[GraphSeries] {
        if self.trigger.is_some() && self.series.is_empty() {
            &self.capture
        } else {
            &self.series
        }
    }
    /// Reject invalid settings before touching state; report whether a reset
    /// occurred so the caller can clear the canvas even when rendering is paused.
    fn set_trigger(&mut self, trigger: Option<TimeSinkTrigger>) -> rustradio::Result<bool> {
        if trigger.is_some_and(|trigger| !trigger.level.is_finite()) {
            return Err(rustradio::Error::msg("trigger level must be finite"));
        }
        if self.trigger == trigger {
            return Ok(false);
        }
        self.trigger = trigger;
        self.clear();
        Ok(true)
    }
    /// Append samples and report whether visible data changed. Trigger updates
    /// are validated together before any detector or capture state is modified.
    fn append_streams(&mut self, streams: &[TaggedVec<Float>]) -> rustradio::Result<bool> {
        let Some(first) = streams.first() else {
            return Ok(false);
        };
        let Some(trigger) = self.trigger else {
            let mut changed = false;
            for (index, stream) in streams.iter().enumerate() {
                if self.series.len() <= index {
                    self.series
                        .push(GraphSeries::new(stream.data.len().min(self.max_points)));
                }
                self.series[index].append_stream(stream, self.max_points);
                changed |= !stream.data.is_empty();
            }
            return Ok(changed);
        };
        let count = first.data.len();
        if streams.iter().any(|stream| stream.data.len() != count)
            || self
                .series_count
                .is_some_and(|previous| previous != streams.len())
        {
            return Err(rustradio::Error::msg(
                "trigger updates require aligned, equally sized series with a fixed count",
            ));
        }
        if count == 0 {
            return Ok(false);
        }
        self.series_count = Some(streams.len());
        let mut cursor = 0;
        let mut changed = false;
        while cursor < count {
            if self.remaining.is_none() {
                let sample = first.data[cursor];
                let crossed = sample.is_finite()
                    && self.previous.is_some_and(|previous| match trigger.edge {
                        TriggerEdge::Rising => previous < trigger.level && sample >= trigger.level,
                        TriggerEdge::Falling => previous > trigger.level && sample <= trigger.level,
                    });
                self.previous = sample.is_finite().then_some(sample);
                if !crossed {
                    cursor += 1;
                    continue;
                }
                // Reuse the bounded capture buffers on subsequent triggers.
                if self.capture.is_empty() {
                    self.capture = (0..streams.len())
                        .map(|_| GraphSeries::new(self.max_points))
                        .collect();
                } else {
                    for series in &mut self.capture {
                        series.samples.clear();
                        series.tags.clear();
                    }
                }
                self.remaining = Some(self.max_points);
            }
            // Edge detection uses series zero only. Copy the same selected span
            // into every series and ignore further crossings during acquisition.
            let remaining = self.remaining.expect("capture started");
            let end = cursor + remaining.min(count - cursor);
            for (series, stream) in self.capture.iter_mut().zip(streams) {
                let offset = series.samples.len();
                series
                    .samples
                    .extend(stream.data[cursor..end].iter().copied());
                series.tags.extend(
                    stream
                        .tags
                        .iter()
                        .filter(|tag| tag.pos() >= cursor && tag.pos() < end)
                        .map(|tag| {
                            Tag::new(offset + tag.pos() - cursor, tag.key(), tag.val().clone())
                        }),
                );
            }
            let last = first.data[end - 1];
            self.previous = last.is_finite().then_some(last);
            let left = remaining - (end - cursor);
            self.remaining = (left > 0).then_some(left);
            cursor = end;
            if left == 0 {
                // Publish a whole window atomically. Later triggers in this
                // update may start filling the spare buffer, but leave this
                // completed capture (including its tags) available to draw.
                std::mem::swap(&mut self.series, &mut self.capture);
                changed = true;
            } else if self.series.is_empty() {
                changed = true;
            }
        }
        Ok(changed)
    }
}

struct Inner {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    y_min_input: HtmlInputElement,
    y_max_input: HtmlInputElement,
    y_apply_button: HtmlButtonElement,
    y_zoom_in_button: HtmlButtonElement,
    y_zoom_out_button: HtmlButtonElement,
    y_auto_button: HtmlButtonElement,
    pause_button: HtmlButtonElement,
    data: TimeData,
    trigger_mode: HtmlSelectElement,
    trigger_level_input: HtmlInputElement,
    trigger_level: Float,
    y_min: f32,
    y_max: f32,

    // Controls.
    auto_scale: bool,
    paused: bool,
    sample_rate: f64,
    y_label: String,
    sync_inputs: bool,
    callbacks: Vec<Closure<dyn FnMut(Event)>>,
    theme_observer: Option<ThemeObserver>,
}

impl Inner {
    /// Validate and apply configuration through the same path for UI and API.
    fn configure_trigger(&mut self, trigger: Option<TimeSinkTrigger>) -> Result<(), JsValue> {
        let changed = self
            .data
            .set_trigger(trigger)
            .map_err(|err| JsValue::from_str(&err.to_string()))?;
        if let Some(trigger) = trigger {
            self.trigger_level = trigger.level;
        }
        self.sync_inputs = true;
        if changed {
            self.draw()
        } else {
            self.sync_controls()
        }
    }

    /// Read an edited edge/level pair and apply it atomically.
    fn apply_trigger_controls(&mut self) -> Result<(), JsValue> {
        let trigger = match self.trigger_mode.value().as_str() {
            "off" => None,
            mode => Some(TimeSinkTrigger {
                level: Self::parse_y_input(&self.trigger_level_input, "Trigger level")?,
                edge: match mode {
                    "rising" => TriggerEdge::Rising,
                    "falling" => TriggerEdge::Falling,
                    _ => return Err(JsValue::from_str("invalid trigger mode")),
                },
            }),
        };
        self.configure_trigger(trigger)
    }

    /// Store a positive finite sample rate, ignoring invalid values.
    fn set_sample_rate(&mut self, sample_rate: f64) {
        if sample_rate.is_finite() && sample_rate > 0.0 {
            self.sample_rate = sample_rate;
        }
    }

    /// Draw the full canvas, including axes, controls, autoscale, and traces.
    fn draw(&mut self) -> Result<(), JsValue> {
        let (width, height) = resize_canvas_to_display_size(&self.canvas)?;
        let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
        let page_theme = window
            .document()
            .and_then(|doc| doc.document_element())
            .and_then(|root| root.get_attribute("data-theme"));
        let is_dark = match page_theme.as_deref() {
            Some("light") => false,
            Some("dark") => true,
            _ => window
                .match_media("(prefers-color-scheme: dark)")?
                .is_some_and(|m| m.matches()),
        };
        let bg = if is_dark { "#0b0b0b" } else { "#ffffff" };
        let axis = if is_dark { "#666" } else { "#888" };
        let text = if is_dark { "#ddd" } else { "#222" };

        self.ctx.set_fill_style_str(bg);
        self.ctx.fill_rect(0.0, 0.0, width, height);
        self.ctx.set_stroke_style_str(axis);
        self.ctx
            .stroke_rect(0.5, 0.5, (width - 1.0).max(0.0), (height - 1.0).max(0.0));

        if self.auto_scale {
            if let Some((min, max)) = self.data_range() {
                debug!("Data range: {min} {max}");
                self.y_min = min;
                self.y_max = max;
            }
            self.sync_inputs = true;
        }
        self.sync_controls()?;

        let mut y_min = self.y_min;
        let mut y_max = self.y_max;
        if !y_min.is_finite() || !y_max.is_finite() || y_min >= y_max {
            y_min = -1.0;
            y_max = 1.0;
        }
        if (y_max - y_min).abs() < f32::EPSILON {
            y_min -= 1.0;
            y_max += 1.0;
        }

        let Some((x_min, x_max)) = self.time_range() else {
            self.ctx.set_fill_style_str(text);
            self.ctx.set_font("12px sans-serif");
            // Axis labels change these canvas settings during a capture.
            self.ctx.set_text_align("left");
            self.ctx.set_text_baseline("alphabetic");
            self.ctx.fill_text(
                if self.data.trigger.is_some() {
                    "Waiting for trigger..."
                } else {
                    "Waiting for float data..."
                },
                12.0,
                20.0,
            )?;
            return Ok(());
        };

        let plot_left = AXIS_MARGIN_LEFT.min((width - 1.0).max(0.0));
        let plot_top = AXIS_MARGIN_TOP.min((height - 1.0).max(0.0));
        let plot_width = (width - AXIS_MARGIN_LEFT - AXIS_MARGIN_RIGHT).max(1.0);
        let plot_height = (height - AXIS_MARGIN_TOP - AXIS_MARGIN_BOTTOM).max(1.0);

        draw_axes(
            &self.ctx,
            axis,
            text,
            plot_left,
            plot_top,
            plot_width,
            plot_height,
            x_min,
            x_max,
            f64::from(y_min),
            f64::from(y_max),
            &self.y_label,
        )?;

        let x_range = (x_max - x_min).max(1e-9);
        let y_range = f64::from(y_max - y_min);
        let colors = ["#2b8cbe", "#31a354", "#756bb1", "#e6550d"];
        let samples_per_bucket = (x_range * self.sample_rate / plot_width.max(1.0))
            .ceil()
            .max(1.0) as u64;

        for (idx, series) in self.data.visible_series().iter().enumerate() {
            draw_series(
                &self.ctx,
                series,
                colors[idx % colors.len()],
                plot_left,
                plot_top,
                plot_width,
                plot_height,
                x_min,
                x_range,
                f64::from(y_min),
                y_range,
                self.sample_rate,
                samples_per_bucket,
            );
        }
        Ok(())
    }

    /// Compute the visible sample range used by autoscale.
    fn data_range(&self) -> Option<(f32, f32)> {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for series in self.data.visible_series() {
            for &sample in &series.samples {
                if sample < min {
                    min = sample;
                }
                if sample > max {
                    max = sample;
                }
            }
        }
        if min.is_finite() && max.is_finite() {
            Some((min, max))
        } else {
            None
        }
    }

    /// Compute the earliest and latest buffered sample time in seconds.
    fn time_range(&self) -> Option<(f64, f64)> {
        let sample_rate = if self.sample_rate.is_finite() && self.sample_rate > 0.0 {
            self.sample_rate
        } else {
            1.0
        };
        if self.data.trigger.is_some() && !self.data.visible_series().is_empty() {
            // A partial capture uses the full window so its X scale stays fixed.
            return Some((0.0, (self.data.max_points - 1).max(1) as f64 / sample_rate));
        }
        let mut min_idx: Option<u64> = None;
        let mut max_idx: Option<u64> = None;
        for series in self.data.visible_series() {
            let len = series.samples.len() as u64;
            if len == 0 {
                continue;
            }
            let series_min = series.start_index;
            let series_max = series.start_index + len - 1;
            min_idx = Some(min_idx.map_or(series_min, |v| v.min(series_min)));
            max_idx = Some(max_idx.map_or(series_max, |v| v.max(series_max)));
        }
        match (min_idx, max_idx) {
            (Some(min_idx), Some(max_idx)) => {
                let min_t = min_idx as f64 / sample_rate;
                let max_t = max_idx as f64 / sample_rate;
                Some((min_t, max_t))
            }
            _ => None,
        }
    }

    /// Apply a multiplicative zoom to the Y axis around its current center.
    fn zoom_y(&mut self, factor: f32) -> Result<(), JsValue> {
        let (mut y_min, mut y_max) = if self.auto_scale {
            self.data_range().unwrap_or((self.y_min, self.y_max))
        } else {
            (self.y_min, self.y_max)
        };
        if !y_min.is_finite() || !y_max.is_finite() || y_min >= y_max {
            y_min = -1.0;
            y_max = 1.0;
        }
        let center = f32::midpoint(y_min, y_max);
        let half = ((y_max - y_min) / 2.0 * factor).max(1e-6);
        self.y_min = center - half;
        self.y_max = center + half;
        self.auto_scale = false;
        self.sync_inputs = true;
        self.draw()
    }

    /// Mirror pause, axes, autoscale, and trigger state into generated controls.
    fn sync_controls(&mut self) -> Result<(), JsValue> {
        if !self.sync_inputs {
            return Ok(());
        }
        self.sync_inputs = false;
        self.trigger_mode.set_value(match self.data.trigger {
            None => "off",
            Some(TimeSinkTrigger {
                edge: TriggerEdge::Rising,
                ..
            }) => "rising",
            Some(TimeSinkTrigger {
                edge: TriggerEdge::Falling,
                ..
            }) => "falling",
        });
        self.trigger_level_input
            .set_value(&self.trigger_level.to_string());
        self.trigger_level_input
            .set_disabled(self.data.trigger.is_none());

        self.y_min_input.set_value(&format!("{}", self.y_min));
        self.y_max_input.set_value(&format!("{}", self.y_max));

        self.pause_button
            .set_text_content(Some(if self.paused { "Resume" } else { "Pause" }));
        self.pause_button
            .set_attribute("aria-pressed", if self.paused { "true" } else { "false" })?;

        self.y_auto_button
            .set_text_content(Some(if self.auto_scale {
                "Autoscale On"
            } else {
                "Autoscale Off"
            }));
        self.y_auto_button.set_attribute(
            "aria-pressed",
            if self.auto_scale { "true" } else { "false" },
        )?;

        Ok(())
    }

    /// Parse one numeric Y-axis input with a caller-friendly error label.
    fn parse_y_input(input: &HtmlInputElement, label: &str) -> Result<f32, JsValue> {
        input
            .value()
            .parse::<f32>()
            .map_err(|e| JsValue::from_str(&format!("parsing {label}: {e}")))
    }
}

/// Register one button callback and keep the closure alive with the sink.
fn install_button_handler(
    inner: &Rc<RefCell<Inner>>,
    button: &HtmlButtonElement,
    mut handler: impl FnMut(&mut Inner) -> Result<(), JsValue> + 'static,
) -> Result<(), JsValue> {
    let state = inner.clone();
    let closure = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
        if let Err(err) = handler(&mut state.borrow_mut()) {
            log::error!("time sink control failed: {err:?}");
        }
    });
    button.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref())?;
    inner.borrow_mut().callbacks.push(closure);
    Ok(())
}

/// Look up a mount element in the current browser document.
fn get_element_by_id(id: &str) -> Result<Element, JsValue> {
    let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    document
        .get_element_by_id(id)
        .ok_or(JsValue::from_str(&format!(
            "can't find element with id {id}"
        )))
}

/// Find a generated child element by its component-local data role.
fn role<T: JsCast>(root: &Element, role: &str) -> Result<T, JsValue> {
    root.query_selector(&format!("[data-role=\"{role}\"]"))?
        .ok_or(JsValue::from_str(&format!(
            "missing time sink element role {role}"
        )))?
        .dyn_into::<T>()
        .map_err(|_| JsValue::from_str(&format!("time sink role {role} has wrong element type")))
}

/// Match the backing canvas resolution to CSS size and device pixel ratio.
fn resize_canvas_to_display_size(canvas: &HtmlCanvasElement) -> Result<(f64, f64), JsValue> {
    let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
    let dpr = window.device_pixel_ratio();
    let display_width = f64::from(canvas.client_width());
    let display_height = f64::from(canvas.client_height());
    if display_width > 0.0 && display_height > 0.0 {
        let width = (display_width * dpr).round().max(1.0) as u32;
        let height = (display_height * dpr).round().max(1.0) as u32;
        if canvas.width() != width || canvas.height() != height {
            canvas.set_width(width);
            canvas.set_height(height);
        }
    }
    Ok((f64::from(canvas.width()), f64::from(canvas.height())))
}

#[allow(clippy::too_many_arguments)]
/// Draw one series as connected samples or bucketed aggregate points.
fn draw_series(
    ctx: &CanvasRenderingContext2d,
    series: &GraphSeries,
    color: &str,
    plot_left: f64,
    plot_top: f64,
    plot_width: f64,
    plot_height: f64,
    x_min: f64,
    x_range: f64,
    y_min: f64,
    y_range: f64,
    sample_rate: f64,
    samples_per_bucket: u64,
) {
    if series.samples.is_empty() {
        return;
    }

    ctx.set_stroke_style_str(color);
    ctx.set_line_width(1.0);

    if samples_per_bucket <= 1 {
        ctx.begin_path();
        let mut started = false;
        for (i, sample) in series.samples.iter().enumerate() {
            let sample_idx = series.start_index + i as u64;
            let x = graph_x(
                sample_idx,
                sample_rate,
                plot_left,
                plot_width,
                x_min,
                x_range,
            );
            let y = graph_y(f64::from(*sample), plot_top, plot_height, y_min, y_range);
            if started {
                ctx.line_to(x, y);
            } else {
                ctx.move_to(x, y);
                started = true;
            }
        }
        ctx.stroke();
        return;
    }

    let mut points = Vec::new();
    let mut bucket: Option<u64> = None;
    let mut first_idx = 0;
    let mut last_idx = 0;
    let mut bucket_min = f32::INFINITY;
    let mut bucket_max = f32::NEG_INFINITY;

    for (i, sample) in series.samples.iter().enumerate() {
        let sample_idx = series.start_index + i as u64;
        let sample_bucket = sample_idx / samples_per_bucket;
        if bucket.is_some_and(|v| v != sample_bucket) {
            push_bucket_point(
                &mut points,
                first_idx,
                last_idx,
                bucket_min,
                bucket_max,
                sample_rate,
                plot_left,
                plot_width,
                x_min,
                x_range,
                plot_top,
                plot_height,
                y_min,
                y_range,
            );
            bucket_min = f32::INFINITY;
            bucket_max = f32::NEG_INFINITY;
            first_idx = sample_idx;
        } else if bucket.is_none() {
            first_idx = sample_idx;
        }
        bucket = Some(sample_bucket);
        last_idx = sample_idx;
        bucket_min = bucket_min.min(*sample);
        bucket_max = bucket_max.max(*sample);
    }

    if bucket.is_some() {
        push_bucket_point(
            &mut points,
            first_idx,
            last_idx,
            bucket_min,
            bucket_max,
            sample_rate,
            plot_left,
            plot_width,
            x_min,
            x_range,
            plot_top,
            plot_height,
            y_min,
            y_range,
        );
    }

    ctx.begin_path();
    for (idx, &(x, y)) in points.iter().enumerate() {
        if idx == 0 {
            ctx.move_to(x, y);
        } else {
            ctx.line_to(x, y);
        }
    }
    ctx.stroke();
}

#[allow(clippy::too_many_arguments)]
/// Add one downsampled point representing the vertical center of a bucket.
fn push_bucket_point(
    points: &mut Vec<(f64, f64)>,
    first_idx: u64,
    last_idx: u64,
    bucket_min: f32,
    bucket_max: f32,
    sample_rate: f64,
    plot_left: f64,
    plot_width: f64,
    x_min: f64,
    x_range: f64,
    plot_top: f64,
    plot_height: f64,
    y_min: f64,
    y_range: f64,
) {
    let center_idx = (first_idx + last_idx) as f64 / 2.0;
    let x = plot_left + ((center_idx / sample_rate - x_min) / x_range) * plot_width;
    let y_min_px = graph_y(f64::from(bucket_min), plot_top, plot_height, y_min, y_range);
    let y_max_px = graph_y(f64::from(bucket_max), plot_top, plot_height, y_min, y_range);
    points.push((x, f64::midpoint(y_min_px, y_max_px)));
}

/// Map a sample index to an X pixel coordinate.
fn graph_x(
    sample_idx: u64,
    sample_rate: f64,
    plot_left: f64,
    plot_width: f64,
    x_min: f64,
    x_range: f64,
) -> f64 {
    let t = sample_idx as f64 / sample_rate;
    plot_left + ((t - x_min) / x_range) * plot_width
}

/// Map a sample value to a Y pixel coordinate.
fn graph_y(sample: f64, plot_top: f64, plot_height: f64, y_min: f64, y_range: f64) -> f64 {
    plot_top + plot_height - ((sample - y_min) / y_range) * plot_height
}

#[allow(clippy::too_many_arguments)]
/// Draw plot axes, tick labels, and axis labels.
fn draw_axes(
    ctx: &CanvasRenderingContext2d,
    axis: &str,
    text: &str,
    plot_left: f64,
    plot_top: f64,
    plot_width: f64,
    plot_height: f64,
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    y_label: &str,
) -> Result<(), JsValue> {
    let plot_right = plot_left + plot_width;
    let plot_bottom = plot_top + plot_height;

    ctx.set_stroke_style_str(axis);
    ctx.set_line_width(1.0);
    ctx.begin_path();
    ctx.move_to(plot_left, plot_top);
    ctx.line_to(plot_left, plot_bottom);
    ctx.line_to(plot_right, plot_bottom);
    ctx.stroke();

    ctx.set_fill_style_str(text);
    ctx.set_font("12px sans-serif");

    let x_ticks = nice_ticks(x_min, x_max, AXIS_TICK_COUNT);
    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    for tick in &x_ticks {
        let t = (*tick - x_min) / (x_max - x_min).max(1e-9);
        let x = plot_left + t * plot_width;
        ctx.begin_path();
        ctx.move_to(x, plot_bottom);
        ctx.line_to(x, plot_bottom + 4.0);
        ctx.stroke();
        ctx.fill_text(&format_tick(*tick), x, plot_bottom + 6.0)?;
    }

    let y_ticks = nice_ticks(y_min, y_max, AXIS_TICK_COUNT);
    ctx.set_text_align("right");
    ctx.set_text_baseline("middle");
    for tick in &y_ticks {
        let t = (*tick - y_min) / (y_max - y_min).max(1e-9);
        let y = plot_bottom - t * plot_height;
        ctx.begin_path();
        ctx.move_to(plot_left - 4.0, y);
        ctx.line_to(plot_left, y);
        ctx.stroke();
        ctx.fill_text(&format_tick(*tick), plot_left - 6.0, y)?;
    }

    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    ctx.fill_text("Time (s)", plot_left + plot_width / 2.0, plot_bottom + 20.0)?;

    ctx.save();
    ctx.translate(plot_left - 40.0, plot_top + plot_height / 2.0)?;
    ctx.rotate(-std::f64::consts::FRAC_PI_2)?;
    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    ctx.fill_text(y_label, 0.0, 0.0)?;
    ctx.restore();

    Ok(())
}

/// Generate human-friendly tick values for an axis range.
fn nice_ticks(min: f64, max: f64, count: usize) -> Vec<f64> {
    if !min.is_finite() || !max.is_finite() || count < 2 {
        return Vec::new();
    }
    if (max - min).abs() < f64::EPSILON {
        return vec![min];
    }
    let range = max - min;
    let step = nice_step(range / (count as f64 - 1.0));
    let start = (min / step).floor() * step;
    let end = (max / step).ceil() * step;
    let mut ticks = Vec::new();
    let mut v = start;
    while v <= end + step * 0.5 {
        ticks.push(v);
        v += step;
    }
    ticks
}

/// Round an arbitrary tick interval to 1, 2, 5, or 10 times a power of ten.
fn nice_step(raw_step: f64) -> f64 {
    if raw_step <= 0.0 {
        return 1.0;
    }
    let exp = raw_step.log10().floor();
    let base = 10.0_f64.powf(exp);
    let scaled = raw_step / base;
    let nice_scaled = if scaled <= 1.0 {
        1.0
    } else if scaled <= 2.0 {
        2.0
    } else if scaled <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice_scaled * base
}

/// Format an axis tick with precision based on its magnitude.
fn format_tick(value: f64) -> String {
    let abs = value.abs();
    if abs >= 1000.0 {
        format!("{value:.0}")
    } else if abs >= 100.0 {
        format!("{value:.1}")
    } else if abs >= 10.0 {
        format!("{value:.2}")
    } else if abs >= 1.0 {
        format!("{value:.3}")
    } else {
        format!("{value:.4}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustradio::stream::TagValue;

    fn stream(samples: &[Float]) -> TaggedVec<Float> {
        TaggedVec {
            data: samples.to_vec(),
            tags: Vec::new(),
        }
    }

    fn triggered(points: usize, edge: TriggerEdge) -> TimeData {
        let mut data = TimeData::new(points);
        data.set_trigger(Some(TimeSinkTrigger { level: 0.0, edge }))
            .unwrap();
        data
    }

    fn samples(data: &TimeData, index: usize) -> Vec<Float> {
        data.visible_series()[index]
            .samples
            .iter()
            .copied()
            .collect()
    }

    #[test]
    fn free_running_retains_latest_samples_independently() {
        let mut data = TimeData::new(3);
        data.append_streams(&[stream(&[0., 1., 2., 3.]), stream(&[9.])])
            .unwrap();
        data.append_streams(&[stream(&[4.]), stream(&[8., 7.])])
            .unwrap();
        assert_eq!(samples(&data, 0), [2., 3., 4.]);
        assert_eq!(samples(&data, 1), [9., 8., 7.]);
        assert_eq!(data.series[0].start_index, 2);
        assert!(!data.append_streams(&[]).unwrap());
    }

    #[test]
    fn crossing_spans_updates_and_capture_holds_until_next_trigger() {
        let mut data = triggered(3, TriggerEdge::Rising);
        assert!(!data.append_streams(&[stream(&[1., 0., -1.])]).unwrap());
        assert!(data.series.is_empty());
        assert!(data.append_streams(&[stream(&[0., 2.])]).unwrap());
        assert_eq!(samples(&data, 0), [0., 2.]);
        assert_eq!(data.remaining, Some(1));
        data.append_streams(&[stream(&[3., 4., -1.])]).unwrap();
        assert_eq!(samples(&data, 0), [0., 2., 3.]);
        assert_eq!(data.remaining, None);
        assert!(!data.append_streams(&[stream(&[-2.])]).unwrap());
        assert_eq!(samples(&data, 0), [0., 2., 3.]);
        assert!(!data.append_streams(&[stream(&[1.])]).unwrap());
        assert_eq!(samples(&data, 0), [0., 2., 3.]);
        assert!(data.append_streams(&[stream(&[5., 6.])]).unwrap());
        assert_eq!(samples(&data, 0), [1., 5., 6.]);
    }

    #[test]
    fn falling_edge_ignores_crossings_during_capture() {
        let mut data = triggered(4, TriggerEdge::Falling);
        data.append_streams(&[stream(&[1., 0., 1., -1., 2.])])
            .unwrap();
        assert_eq!(samples(&data, 0), [0., 1., -1., 2.]);
        assert_eq!(data.remaining, None);
        // The final captured sample is also the history for the next edge.
        data.append_streams(&[stream(&[0.])]).unwrap();
        assert_eq!(samples(&data, 0), [0., 1., -1., 2.]);
        assert_eq!(
            data.capture[0].samples.iter().copied().collect::<Vec<_>>(),
            [0.]
        );
        assert_eq!(data.remaining, Some(3));
    }

    #[test]
    fn one_update_can_complete_and_retrigger_multiple_times() {
        let mut data = triggered(2, TriggerEdge::Rising);
        data.append_streams(&[stream(&[-1., 0., 1., -1., 2., 3., -1., 4.])])
            .unwrap();
        // A trailing trigger must not replace the last complete window with
        // a single point at the end of an input update.
        assert_eq!(samples(&data, 0), [2., 3.]);
        assert_eq!(data.remaining, Some(1));
        assert!(!data.append_streams(&[]).unwrap());
        assert_eq!(samples(&data, 0), [2., 3.]);
        assert!(data.append_streams(&[stream(&[5.])]).unwrap());
        assert_eq!(samples(&data, 0), [4., 5.]);
        let mut single = triggered(1, TriggerEdge::Rising);
        single
            .append_streams(&[stream(&[-1., 1., -1., 2.])])
            .unwrap();
        assert_eq!(samples(&single, 0), [2.]);
        assert_eq!(single.remaining, None);
    }

    #[test]
    fn all_series_capture_same_range_and_translate_tags() {
        let mut data = triggered(3, TriggerEdge::Rising);
        let mut first = stream(&[-1., 0., 1.]);
        first.tags = (0..3)
            .map(|pos| Tag::new(pos, "tag", TagValue::U64(pos as u64)))
            .collect();
        data.append_streams(&[first, stream(&[9., 10., 11.])])
            .unwrap();
        let mut next = stream(&[2., 3., -1., 4.]);
        next.tags = vec![
            Tag::new(0, "end", TagValue::Bool(true)),
            Tag::new(3, "next", TagValue::Bool(true)),
        ];
        data.append_streams(&[next, stream(&[12., 13., 14., 15.])])
            .unwrap();
        assert_eq!(samples(&data, 0), [0., 1., 2.]);
        assert_eq!(samples(&data, 1), [10., 11., 12.]);
        assert_eq!(data.series[0].tags.len(), 3);
        data.append_streams(&[stream(&[5., 6.]), stream(&[16., 17.])])
            .unwrap();
        assert_eq!(samples(&data, 0), [4., 5., 6.]);
        assert_eq!(samples(&data, 1), [15., 16., 17.]);
        assert_eq!(data.series[0].tags.len(), 1);
        assert_eq!(data.series[0].tags[0].key(), "next");
        assert_eq!(data.series[0].tags[0].pos(), 0);
        // A fresh capture can keep tags across two batches.
        data.clear();
        let mut first = stream(&[-1., 0., 1.]);
        first.tags = vec![
            Tag::new(0, "skip", TagValue::Bool(true)),
            Tag::new(2, "keep", TagValue::Bool(true)),
        ];
        data.append_streams(&[first]).unwrap();
        let mut last = stream(&[2., 3.]);
        last.tags = vec![
            Tag::new(0, "last", TagValue::Bool(true)),
            Tag::new(1, "skip", TagValue::Bool(true)),
        ];
        data.append_streams(&[last]).unwrap();
        assert_eq!(samples(&data, 0), [0., 1., 2.]);
        assert_eq!(
            data.series[0]
                .tags
                .iter()
                .map(|t| (t.pos(), t.key()))
                .collect::<Vec<_>>(),
            [(1, "keep"), (2, "last")]
        );
    }

    #[test]
    fn invalid_batch_does_not_mutate_capture_or_detector() {
        let mut data = triggered(3, TriggerEdge::Rising);
        assert!(data.append_streams(&[stream(&[-1.]), stream(&[])]).is_err());
        assert_eq!(data.series_count, None);
        assert_eq!(data.previous, None);
        data.append_streams(&[stream(&[-1., 0.]), stream(&[8., 9.])])
            .unwrap();
        assert!(data.append_streams(&[stream(&[-1.])]).is_err());
        assert!(
            data.append_streams(&[stream(&[-1., 1.]), stream(&[0.])])
                .is_err()
        );
        assert_eq!(data.previous, Some(0.));
        assert_eq!(data.remaining, Some(2));
        assert_eq!(samples(&data, 0), [0.]);
        data.append_streams(&[stream(&[2., 3.]), stream(&[10., 11.])])
            .unwrap();
        assert_eq!(samples(&data, 1), [9., 10., 11.]);
        data.clear();
        data.append_streams(&[stream(&[-1., 0.])]).unwrap();
        assert_eq!(data.series_count, Some(1));
    }

    #[test]
    fn nonfinite_samples_break_edge_history() {
        let mut data = triggered(2, TriggerEdge::Rising);
        assert!(
            !data
                .append_streams(&[stream(&[-1., Float::NAN, 1., -1., Float::INFINITY, 1.])])
                .unwrap()
        );
        data.append_streams(&[stream(&[-1., 0.])]).unwrap();
        assert_eq!(samples(&data, 0), [0.]);
    }

    #[test]
    fn configuration_validation_and_reset() {
        let mut data = triggered(2, TriggerEdge::Rising);
        data.append_streams(&[stream(&[-1., 0.])]).unwrap();
        let trigger = data.trigger;
        assert!(!data.set_trigger(trigger).unwrap());
        assert_eq!(samples(&data, 0), [0.]);
        for level in [Float::NAN, Float::INFINITY, Float::NEG_INFINITY] {
            assert!(
                data.set_trigger(Some(TimeSinkTrigger {
                    level,
                    edge: TriggerEdge::Falling
                }))
                .is_err()
            );
            assert_eq!(data.trigger, trigger);
            assert_eq!(samples(&data, 0), [0.]);
        }
        data.clear();
        assert_eq!(data.trigger, trigger);
        assert_eq!(data.previous, None);
        assert_eq!(data.remaining, None);
        assert!(data.set_trigger(None).unwrap());
        data.append_streams(&[stream(&[2., 3., 4.])]).unwrap();
        assert_eq!(samples(&data, 0), [3., 4.]);
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_tests {
    use super::*;
    use wasm_bindgen_test::*;

    // The experimental socket tests select browser execution when enabled.
    #[cfg(not(feature = "unstable"))]
    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test(async)]
    async fn theme_changes_redraw_a_paused_capture() {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let page = document.document_element().unwrap();
        let previous_theme = page.get_attribute("data-theme");
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let sink = TimeSink::mount(
            &root,
            TimeSinkOptions {
                max_points: 3,
                ..TimeSinkOptions::default()
            },
        )
        .unwrap();
        let autoscale = role::<HtmlButtonElement>(&root, "y-auto").unwrap();
        assert_eq!(autoscale.text_content().as_deref(), Some("Autoscale Off"));
        assert!(!sink.inner.borrow().auto_scale);
        autoscale.click();
        assert!(sink.inner.borrow().auto_scale);
        assert_eq!(autoscale.text_content().as_deref(), Some("Autoscale On"));
        autoscale.click();
        sink.set_trigger(Some(TimeSinkTrigger {
            level: 0.,
            edge: TriggerEdge::Rising,
        }))
        .unwrap();
        sink.update(vec![TaggedVec {
            data: vec![-1., 0., 0.5, 1.],
            tags: vec![],
        }])
        .unwrap();
        sink.set_paused(true).unwrap();
        for (theme, background) in [("dark", 11), ("light", 255)] {
            page.set_attribute("data-theme", theme).unwrap();
            // Attribute observers run at the next microtask checkpoint.
            wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::UNDEFINED))
                .await
                .unwrap();
            let inner = sink.inner.borrow();
            let pixel = inner.ctx.get_image_data(5., 5., 1., 1.).unwrap().data();
            assert_eq!(&pixel.0[..3], &[background; 3]);
            assert_eq!(
                inner.data.visible_series()[0]
                    .samples
                    .iter()
                    .copied()
                    .collect::<Vec<_>>(),
                [0., 0.5, 1.]
            );
            assert!(sink.paused());
        }
        if let Some(theme) = previous_theme {
            page.set_attribute("data-theme", &theme).unwrap();
        } else {
            page.remove_attribute("data-theme").unwrap();
        }
        root.remove();
    }

    #[wasm_bindgen_test]
    fn controls_and_paused_capture() {
        let document = web_sys::window().unwrap().document().unwrap();
        let style = document.create_element("style").unwrap();
        style.set_text_content(Some(include_str!("../../assets/rustradio.css")));
        document.body().unwrap().append_child(&style).unwrap();
        let root = document.create_element("div").unwrap();
        root.set_attribute("style", "width: 320px").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let sink = TimeSink::mount(
            &root,
            TimeSinkOptions {
                max_points: 3,
                sample_rate: 2.0,
                fixed_range: Some((-2.0, 5.0)),
                ..TimeSinkOptions::default()
            },
        )
        .unwrap();
        let mode = role::<HtmlSelectElement>(&root, "trigger-mode").unwrap();
        let level = role::<HtmlInputElement>(&root, "trigger-level").unwrap();
        let canvas = role::<HtmlCanvasElement>(&root, "canvas").unwrap();
        assert_eq!(sink.trigger(), None);
        assert!(level.disabled());
        assert!(
            root.scroll_width() <= root.client_width(),
            "controls overflow narrow panel"
        );
        // Run this test with both light and dark browser preferences. The CSS
        // must pick readable native colors even without application variables.
        assert_eq!(js_sys::eval(r#"getComputedStyle(document.querySelector('[data-role="trigger-mode"]')).colorScheme"#).unwrap().as_string().unwrap(), "light dark");
        mode.set_value("rising");
        mode.dispatch_event(&Event::new("change").unwrap()).unwrap();
        assert_eq!(
            sink.trigger(),
            Some(TimeSinkTrigger {
                level: 0.,
                edge: TriggerEdge::Rising
            })
        );
        assert!(!level.disabled());
        let snapshot = || canvas.to_data_url().unwrap();
        let waiting = snapshot();
        sink.update(vec![TaggedVec {
            data: vec![1., -1.],
            tags: vec![],
        }])
        .unwrap();
        assert!(snapshot() == waiting, "waiting canvas changed");
        sink.set_paused(true).unwrap();
        sink.update(vec![TaggedVec {
            data: vec![0., 1.],
            tags: vec![],
        }])
        .unwrap();
        assert!(snapshot() == waiting, "waiting canvas changed");
        assert_eq!(sink.inner.borrow().time_range(), Some((0., 1.)));
        sink.set_paused(false).unwrap();
        assert!(snapshot() != waiting, "capture was not drawn");
        sink.update(vec![TaggedVec {
            data: vec![2.],
            tags: vec![],
        }])
        .unwrap();
        let completed = snapshot();
        sink.update(vec![TaggedVec {
            data: vec![3., 4.],
            tags: vec![],
        }])
        .unwrap();
        assert!(snapshot() == completed, "completed capture changed");
        // A retrigger spanning updates keeps the entire displayed window in
        // place, including when another UI action requests a redraw.
        sink.update(vec![TaggedVec {
            data: vec![-1., 0., 4.],
            tags: vec![],
        }])
        .unwrap();
        assert!(
            snapshot() == completed,
            "partial replacement erased capture"
        );
        sink.draw().unwrap();
        assert!(
            snapshot() == completed,
            "redraw exposed partial replacement"
        );
        sink.update(vec![TaggedVec {
            data: vec![5.],
            tags: vec![],
        }])
        .unwrap();
        assert!(
            snapshot() != completed,
            "new completed capture was not drawn"
        );
        sink.set_paused(true).unwrap();
        sink.set_trigger(Some(TimeSinkTrigger {
            level: 2.,
            edge: TriggerEdge::Falling,
        }))
        .unwrap();
        assert_eq!(mode.value(), "falling");
        assert_eq!(level.value(), "2");
        assert!(snapshot() == waiting, "waiting canvas changed");
        level.set_value("3");
        level
            .dispatch_event(&Event::new("change").unwrap())
            .unwrap();
        assert_eq!(sink.trigger().unwrap().level, 3.);
        level.set_value("");
        level
            .dispatch_event(&Event::new("change").unwrap())
            .unwrap();
        assert_eq!(sink.trigger().unwrap().level, 3.);
        assert_eq!(level.value(), "3");
        sink.clear().unwrap();
        assert_eq!(sink.trigger().unwrap().level, 3.);
        mode.set_value("off");
        mode.dispatch_event(&Event::new("change").unwrap()).unwrap();
        assert_eq!(sink.trigger(), None);
        assert!(level.disabled());
        root.remove();
        style.remove();
    }
}
