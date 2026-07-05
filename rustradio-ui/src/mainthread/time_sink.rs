use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use log::debug;
use rustradio::Float;
use rustradio::stream::Tag;
use wasm_bindgen::prelude::*;
use web_sys::{
    CanvasRenderingContext2d, Element, Event, HtmlButtonElement, HtmlCanvasElement,
    HtmlInputElement,
};

use crate::TaggedVec;

/// Convert browser/DOM failures into the crate-level error type exposed by the
/// time sink API.
fn dom_result<T>(result: Result<T, JsValue>, context: &str) -> rustradio::Result<T> {
    result.map_err(|err| {
        let detail = err.as_string().unwrap_or_else(|| format!("{err:?}"));
        rustradio::Error::msg(format!("{context}: {detail}"))
    })
}

const TIME_SINK_HTML: &str = r#"
<div class="panel-header rr-time-sink-header">
  <div>
    <h2 class="panel-title" data-role="title"></h2>
    <p class="panel-kicker" data-role="subtitle"></p>
  </div>
  <div class="rr-time-sink-controls" aria-label="Time sink controls">
    <label class="control-field">
      <span>Y min</span>
      <input data-role="y-min" type="number" step="any" value="-1">
    </label>
    <label class="control-field">
      <span>Y max</span>
      <input data-role="y-max" type="number" step="any" value="1">
    </label>
    <button data-role="y-apply" type="button">Apply</button>
    <button data-role="y-zoom-in" type="button">Zoom In</button>
    <button data-role="y-zoom-out" type="button">Zoom Out</button>
    <button data-role="y-auto" type="button">Autoscale On</button>
    <button data-role="pause" type="button">Pause</button>
  </div>
</div>
<div class="panel-body">
  <canvas class="rr-time-sink-canvas" data-role="canvas"></canvas>
</div>
"#;

const DEFAULT_MAX_GRAPH_POINTS: usize = 10_000;
const AXIS_MARGIN_LEFT: f64 = 56.0;
const AXIS_MARGIN_RIGHT: f64 = 12.0;
const AXIS_MARGIN_TOP: f64 = 12.0;
const AXIS_MARGIN_BOTTOM: f64 = 30.0;
const AXIS_TICK_COUNT: usize = 6;

/// Options for the time sink.
#[derive(Debug, Clone)]
pub struct TimeSinkOptions {
    pub title: String,
    pub subtitle: String,
    pub y_label: String,
    pub sample_rate: f64,
    pub max_points: usize,
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

        role::<Element>(root, "title")?.set_text_content(Some(&options.title));
        role::<Element>(root, "subtitle")?.set_text_content(Some(&options.subtitle));

        let canvas = role::<HtmlCanvasElement>(root, "canvas")?;
        let ctx = canvas
            .get_context("2d")?
            .ok_or(JsValue::from_str("no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;

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
            series: Vec::new(),
            y_min: -1.0,
            y_max: 1.0,
            auto_scale: true,
            paused: false,
            sample_rate: options.sample_rate,
            max_points: options.max_points.max(1),
            y_label: options.y_label,
            sync_inputs: true,
            callbacks: Vec::new(),
        }));

        let sink = Self { inner };
        sink.install_handlers()?;
        sink.draw()?;
        Ok(sink)
    }

    /// Add new tagged float streams to the sink and redraw unless paused.
    ///
    /// In order to graph a complex signal, first split it into two Float
    /// streams.
    #[allow(clippy::needless_pass_by_value)]
    pub fn update(&self, streams: Vec<TaggedVec<Float>>) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.append_streams(&streams);
        let result = if inner.paused {
            inner.sync_controls()
        } else {
            inner.draw()
        };
        dom_result(result, "updating time sink")
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
        inner.series.clear();
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
            if inner.paused {
                inner.sync_controls()
            } else {
                inner.draw()
            }
        })?;

        let inner = self.inner.clone();
        let handler = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            if let Err(err) = inner.borrow_mut().draw() {
                log::error!("time sink resize failed: {err:?}");
            }
        });
        let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
        window.add_event_listener_with_callback("resize", handler.as_ref().unchecked_ref())?;
        self.inner.borrow_mut().callbacks.push(handler);

        Ok(())
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
    series: Vec<GraphSeries>,
    y_min: f32,
    y_max: f32,
    auto_scale: bool,
    paused: bool,
    sample_rate: f64,
    max_points: usize,
    y_label: String,
    sync_inputs: bool,
    callbacks: Vec<Closure<dyn FnMut(Event)>>,
}

impl Inner {
    /// Store a positive finite sample rate, ignoring invalid values.
    fn set_sample_rate(&mut self, sample_rate: f64) {
        if sample_rate.is_finite() && sample_rate > 0.0 {
            self.sample_rate = sample_rate;
        }
    }

    /// Append all streams from one update, creating series as needed.
    fn append_streams(&mut self, streams: &[TaggedVec<Float>]) {
        for (idx, stream) in streams.iter().enumerate() {
            if self.series.len() <= idx {
                self.series
                    .push(GraphSeries::new(stream.data.len().min(self.max_points)));
            }
            self.series[idx].append_stream(stream, self.max_points);
        }
    }

    /// Draw the full canvas, including axes, controls, autoscale, and traces.
    fn draw(&mut self) -> Result<(), JsValue> {
        let (width, height) = resize_canvas_to_display_size(&self.canvas)?;
        let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
        let is_dark = window
            .match_media("(prefers-color-scheme: dark)")?
            .is_some_and(|m| m.matches());
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
            self.ctx
                .fill_text("Waiting for float data...", 12.0, 20.0)?;
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

        for (idx, series) in self.series.iter().enumerate() {
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
        for series in &self.series {
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
        let mut min_idx: Option<u64> = None;
        let mut max_idx: Option<u64> = None;
        for series in &self.series {
            let len = series.samples.len() as u64;
            if len == 0 {
                continue;
            }
            let series_min = series.start_index;
            let series_max = series.start_index + len - 1;
            min_idx = Some(min_idx.map_or(series_min, |v| v.min(series_min)));
            max_idx = Some(max_idx.map_or(series_max, |v| v.max(series_max)));
        }
        let sample_rate = if self.sample_rate > 0.0 {
            self.sample_rate
        } else {
            1.0
        };
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

    /// Mirror internal pause/autoscale/Y-range state into generated controls.
    fn sync_controls(&mut self) -> Result<(), JsValue> {
        if self.sync_inputs {
            self.y_min_input.set_value(&format!("{}", self.y_min));
            self.y_max_input.set_value(&format!("{}", self.y_max));
            self.sync_inputs = false;
        }

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
