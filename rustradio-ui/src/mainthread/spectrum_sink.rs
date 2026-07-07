//! Like time sink, this is mostly LLM coded. It does work as a proof of
//! concept, but it needs to be properly reviewed.
use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use js_sys::Uint8ClampedArray;
use rustradio::Float;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, Element, Event, HtmlCanvasElement, ImageData};

use crate::TaggedVec;

const SPECTRUM_SINK_HTML: &str = r#"
<div class="rr-panel-header">
  <div>
    <h3 class="rr-panel-title" data-role="title"></h3>
    <p class="rr-panel-kicker" data-role="subtitle"></p>
  </div>
</div>
<div class="rr-panel-body">
  <canvas class="rr-spectrum-sink-canvas" data-role="canvas"></canvas>
</div>
"#;

const WATERFALL_SINK_HTML: &str = r#"
<div class="rr-panel-header">
  <div>
    <h3 class="rr-panel-title" data-role="title"></h3>
    <p class="rr-panel-kicker" data-role="subtitle"></p>
  </div>
</div>
<div class="rr-panel-body">
  <div class="rr-waterfall-sink-layout" data-role="waterfall-layout">
    <div class="rr-waterfall-sink-y-label" data-role="waterfall-y-label">
      <span class="rr-waterfall-sink-y-label-text">Time</span>
    </div>
    <div class="rr-waterfall-sink-plot" data-role="waterfall-plot">
      <canvas class="rr-waterfall-sink-canvas" data-role="canvas"></canvas>
      <div class="rr-waterfall-sink-grid rr-waterfall-sink-grid-0" data-role="waterfall-grid-0"></div>
      <div class="rr-waterfall-sink-grid rr-waterfall-sink-grid-1" data-role="waterfall-grid-1"></div>
      <div class="rr-waterfall-sink-grid rr-waterfall-sink-grid-2" data-role="waterfall-grid-2"></div>
      <div class="rr-waterfall-sink-grid rr-waterfall-sink-grid-3" data-role="waterfall-grid-3"></div>
      <div class="rr-waterfall-sink-grid rr-waterfall-sink-grid-4" data-role="waterfall-grid-4"></div>
      <div class="rr-waterfall-sink-status" data-role="waterfall-status">Waiting for waterfall data...</div>
    </div>
    <div class="rr-waterfall-sink-x-ticks" data-role="waterfall-x-ticks">
      <span class="rr-waterfall-sink-tick rr-waterfall-sink-tick-0" data-role="waterfall-tick-0"></span>
      <span class="rr-waterfall-sink-tick rr-waterfall-sink-tick-1" data-role="waterfall-tick-1"></span>
      <span class="rr-waterfall-sink-tick rr-waterfall-sink-tick-2" data-role="waterfall-tick-2"></span>
      <span class="rr-waterfall-sink-tick rr-waterfall-sink-tick-3" data-role="waterfall-tick-3"></span>
      <span class="rr-waterfall-sink-tick rr-waterfall-sink-tick-4" data-role="waterfall-tick-4"></span>
    </div>
    <div class="rr-waterfall-sink-x-label" data-role="waterfall-x-label">Frequency (Hz)</div>
  </div>
</div>
"#;

const DEFAULT_MAX_WATERFALL_FRAMES: usize = 40;
const DEFAULT_WATERFALL_MIN_DB: f32 = -120.0;
const DEFAULT_WATERFALL_MAX_DB: f32 = 0.0;
const AXIS_MARGIN_LEFT: f64 = 54.0;
const AXIS_MARGIN_RIGHT: f64 = 14.0;
const AXIS_MARGIN_TOP: f64 = 12.0;
const AXIS_MARGIN_BOTTOM: f64 = 32.0;
const AXIS_TICK_COUNT: usize = 5;

/// Convert browser/DOM failures into the crate-level error type exposed by the
/// spectrum and waterfall sink APIs.
fn dom_result<T>(result: Result<T, JsValue>, context: &str) -> rustradio::Result<T> {
    result.map_err(|err| {
        let detail = err.as_string().unwrap_or_else(|| format!("{err:?}"));
        rustradio::Error::msg(format!("{context}: {detail}"))
    })
}

/// Options for the spectrum sink.
#[derive(Debug, Clone)]
pub struct SpectrumSinkOptions {
    pub title: String,
    pub subtitle: String,
    pub sample_rate: f32,
}

impl Default for SpectrumSinkOptions {
    /// Build a generic spectrum sink configuration for callers that only need a
    /// mount point and FFT power frame updates.
    fn default() -> Self {
        Self {
            title: "Spectrum".into(),
            subtitle: "FFT power frame".into(),
            sample_rate: 1.0,
        }
    }
}

/// Options for the waterfall sink.
#[derive(Debug, Clone)]
pub struct WaterfallSinkOptions {
    pub title: String,
    pub subtitle: String,
    pub sample_rate: f32,
    pub max_frames: usize,
    pub min_db: f32,
    pub max_db: f32,
}

impl Default for WaterfallSinkOptions {
    /// Build a generic waterfall sink configuration for callers that only need
    /// a mount point and FFT power frame updates.
    fn default() -> Self {
        Self {
            title: "Waterfall".into(),
            subtitle: "FFT power history".into(),
            sample_rate: 1.0,
            max_frames: DEFAULT_MAX_WATERFALL_FRAMES,
            min_db: DEFAULT_WATERFALL_MIN_DB,
            max_db: DEFAULT_WATERFALL_MAX_DB,
        }
    }
}

/// Spectrum sink. This is a handle to a graph element where one FFT power frame
/// is shown as a frequency-domain trace.
#[derive(Clone)]
pub struct SpectrumSink {
    inner: Rc<RefCell<SpectrumInner>>,
}

impl SpectrumSink {
    /// Find a mount element by ID and replace its contents with a spectrum
    /// sink.
    pub fn mount_by_id(
        id: &str,
        options: impl Borrow<SpectrumSinkOptions>,
    ) -> rustradio::Result<Self> {
        let root = dom_result(
            get_element_by_id(id),
            &format!("finding spectrum sink mount element {id}"),
        )?;
        Self::mount(&root, options)
    }

    /// Mount a self-contained spectrum sink into an existing DOM element.
    pub fn mount(
        root: &Element,
        options: impl Borrow<SpectrumSinkOptions>,
    ) -> rustradio::Result<Self> {
        dom_result(
            Self::mount_dom(root, options.borrow()),
            "mounting spectrum sink",
        )
    }

    /// Add the newest FFT power frame to the sink and redraw.
    pub fn update(&self, frames: &[TaggedVec<Float>]) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.set_latest(frames);
        dom_result(inner.draw(), "updating spectrum sink")
    }

    /// Set the sample rate used to convert FFT bin indexes into frequencies.
    #[allow(dead_code)]
    pub fn set_sample_rate(&self, sample_rate: f32) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.set_sample_rate(sample_rate);
        dom_result(inner.draw(), "setting spectrum sink sample rate")
    }

    /// Drop the retained FFT frame and redraw the empty sink.
    #[allow(dead_code)]
    pub fn clear(&self) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.latest = None;
        dom_result(inner.draw(), "clearing spectrum sink")
    }

    /// Mount the generated DOM and wire handlers, preserving browser-native
    /// errors until the public API boundary.
    fn mount_dom(root: &Element, options: &SpectrumSinkOptions) -> Result<Self, JsValue> {
        root.set_inner_html(SPECTRUM_SINK_HTML);

        role::<Element>(root, "title")?.set_text_content(Some(&options.title));
        role::<Element>(root, "subtitle")?.set_text_content(Some(&options.subtitle));

        let canvas = role::<HtmlCanvasElement>(root, "canvas")?;
        let ctx = canvas
            .get_context("2d")?
            .ok_or(JsValue::from_str("no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;

        let sink = Self {
            inner: Rc::new(RefCell::new(SpectrumInner {
                canvas,
                ctx,
                latest: None,
                sample_rate: sanitize_sample_rate(options.sample_rate),
                callbacks: Vec::new(),
            })),
        };
        sink.install_handlers()?;
        sink.draw()?;
        Ok(sink)
    }

    /// Redraw the current sink state.
    fn draw(&self) -> Result<(), JsValue> {
        self.inner.borrow_mut().draw()
    }

    /// Install resize callbacks for this sink instance.
    fn install_handlers(&self) -> Result<(), JsValue> {
        let inner = self.inner.clone();
        let handler = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            if let Err(err) = inner.borrow_mut().draw() {
                log::error!("spectrum sink resize failed: {err:?}");
            }
        });
        let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
        window.add_event_listener_with_callback("resize", handler.as_ref().unchecked_ref())?;
        self.inner.borrow_mut().callbacks.push(handler);

        Ok(())
    }
}

/// Waterfall sink. This is a handle to a graph element where FFT power frames
/// are shown as a scrolling frequency-over-time image.
#[derive(Clone)]
pub struct WaterfallSink {
    inner: Rc<RefCell<WaterfallInner>>,
}

impl WaterfallSink {
    /// Find a mount element by ID and replace its contents with a waterfall
    /// sink.
    pub fn mount_by_id(
        id: &str,
        options: impl Borrow<WaterfallSinkOptions>,
    ) -> rustradio::Result<Self> {
        let root = dom_result(
            get_element_by_id(id),
            &format!("finding waterfall sink mount element {id}"),
        )?;
        Self::mount(&root, options)
    }

    /// Mount a self-contained waterfall sink into an existing DOM element.
    pub fn mount(
        root: &Element,
        options: impl Borrow<WaterfallSinkOptions>,
    ) -> rustradio::Result<Self> {
        dom_result(
            Self::mount_dom(root, options.borrow()),
            "mounting waterfall sink",
        )
    }

    /// Add FFT power frames to the waterfall history and redraw.
    pub fn update(&self, frames: &[TaggedVec<Float>]) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.append_frames(frames);
        dom_result(inner.draw(), "updating waterfall sink")
    }

    /// Set the sample rate used to convert FFT bin indexes into frequencies.
    #[allow(dead_code)]
    pub fn set_sample_rate(&self, sample_rate: f32) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.set_sample_rate(sample_rate);
        dom_result(inner.draw(), "setting waterfall sink sample rate")
    }

    /// Drop the retained history and redraw the empty sink.
    #[allow(dead_code)]
    pub fn clear(&self) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.history.clear();
        inner.bitmap_needs_rebuild = true;
        inner.pending_rows = 0;
        dom_result(inner.draw(), "clearing waterfall sink")
    }

    /// Mount the generated DOM and wire handlers, preserving browser-native
    /// errors until the public API boundary.
    fn mount_dom(root: &Element, options: &WaterfallSinkOptions) -> Result<Self, JsValue> {
        root.set_inner_html(WATERFALL_SINK_HTML);

        role::<Element>(root, "title")?.set_text_content(Some(&options.title));
        role::<Element>(root, "subtitle")?.set_text_content(Some(&options.subtitle));

        let canvas = role::<HtmlCanvasElement>(root, "canvas")?;
        let ctx = canvas
            .get_context("2d")?
            .ok_or(JsValue::from_str("no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let (bitmap_canvas, bitmap_ctx) = create_canvas_2d()?;
        let status = role::<Element>(root, "waterfall-status")?;
        let tick_labels = waterfall_role_elements(root, "waterfall-tick")?;

        let sink = Self {
            inner: Rc::new(RefCell::new(WaterfallInner {
                canvas,
                ctx,
                bitmap_canvas,
                bitmap_ctx,
                status,
                tick_labels,
                history: VecDeque::new(),
                sample_rate: sanitize_sample_rate(options.sample_rate),
                max_frames: options.max_frames.max(1),
                min_db: options.min_db,
                max_db: options.max_db,
                bitmap_width: 0,
                bitmap_height: 0,
                bitmap_needs_rebuild: true,
                pending_rows: 0,
                callbacks: Vec::new(),
            })),
        };
        sink.install_handlers()?;
        sink.draw()?;
        Ok(sink)
    }

    /// Redraw the current sink state.
    fn draw(&self) -> Result<(), JsValue> {
        self.inner.borrow_mut().draw()
    }

    /// Install resize callbacks for this sink instance.
    fn install_handlers(&self) -> Result<(), JsValue> {
        let inner = self.inner.clone();
        let handler = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            if let Err(err) = inner.borrow_mut().draw() {
                log::error!("waterfall sink resize failed: {err:?}");
            }
        });
        let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
        window.add_event_listener_with_callback("resize", handler.as_ref().unchecked_ref())?;
        self.inner.borrow_mut().callbacks.push(handler);

        Ok(())
    }
}

struct SpectrumInner {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    latest: Option<Vec<f32>>,
    sample_rate: f32,
    callbacks: Vec<Closure<dyn FnMut(Event)>>,
}

impl SpectrumInner {
    /// Store a positive finite sample rate, ignoring invalid values.
    fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate.is_finite() && sample_rate > 0.0 {
            self.sample_rate = sample_rate;
        }
    }

    /// Keep the newest frame from a worker update.
    fn set_latest(&mut self, frames: &[TaggedVec<Float>]) {
        for frame in frames {
            self.latest = Some(frame.data.clone());
        }
    }

    /// Draw the full canvas, including axes and the latest spectrum trace.
    fn draw(&mut self) -> Result<(), JsValue> {
        let (width, height) = resize_canvas_to_display_size(&self.canvas)?;
        let theme = CanvasTheme::current()?;

        self.ctx.set_fill_style_str(theme.bg);
        self.ctx.fill_rect(0.0, 0.0, width, height);
        self.ctx.set_stroke_style_str(theme.axis);
        self.ctx
            .stroke_rect(0.5, 0.5, (width - 1.0).max(0.0), (height - 1.0).max(0.0));

        let Some(frame) = &self.latest else {
            self.ctx.set_fill_style_str(theme.text);
            self.ctx.set_font("12px sans-serif");
            self.ctx
                .fill_text("Waiting for spectrum data...", 12.0, 20.0)?;
            return Ok(());
        };
        if frame.is_empty() {
            return Ok(());
        }

        let plot_left = AXIS_MARGIN_LEFT.min((width - 1.0).max(0.0));
        let plot_top = AXIS_MARGIN_TOP.min((height - 1.0).max(0.0));
        let plot_width = (width - AXIS_MARGIN_LEFT - AXIS_MARGIN_RIGHT).max(1.0);
        let plot_height = (height - AXIS_MARGIN_TOP - AXIS_MARGIN_BOTTOM).max(1.0);

        let (mut y_min, mut y_max) = data_range(frame).unwrap_or((-120.0, 0.0));
        if (y_max - y_min).abs() < f32::EPSILON {
            y_min -= 1.0;
            y_max += 1.0;
        }
        let padding = ((y_max - y_min) * 0.08).max(1.0);
        y_min -= padding;
        y_max += padding;

        draw_spectrum_axes(
            &self.ctx,
            &theme,
            plot_left,
            plot_top,
            plot_width,
            plot_height,
            self.sample_rate,
            f64::from(y_min),
            f64::from(y_max),
        )?;

        let y_range = f64::from(y_max - y_min).max(1.0e-9);
        let len = frame.len();
        let half = len / 2;
        let denom = (len.saturating_sub(1)).max(1) as f64;

        self.ctx.set_stroke_style_str(theme.trace);
        self.ctx.set_line_width(1.25);
        self.ctx.begin_path();
        for i in 0..len {
            let bin = frame[(i + half) % len];
            let x = plot_left + (i as f64 / denom) * plot_width;
            let y = plot_top + plot_height
                - ((f64::from(bin) - f64::from(y_min)) / y_range) * plot_height;
            if i == 0 {
                self.ctx.move_to(x, y);
            } else {
                self.ctx.line_to(x, y);
            }
        }
        self.ctx.stroke();

        Ok(())
    }
}

struct WaterfallInner {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    bitmap_canvas: HtmlCanvasElement,
    bitmap_ctx: CanvasRenderingContext2d,
    status: Element,
    tick_labels: Vec<Element>,
    history: VecDeque<Vec<f32>>,
    sample_rate: f32,
    max_frames: usize,
    min_db: f32,
    max_db: f32,
    bitmap_width: u32,
    bitmap_height: u32,
    bitmap_needs_rebuild: bool,
    pending_rows: usize,
    callbacks: Vec<Closure<dyn FnMut(Event)>>,
}

impl WaterfallInner {
    /// Store a positive finite sample rate, ignoring invalid values.
    fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate.is_finite() && sample_rate > 0.0 {
            self.sample_rate = sample_rate;
        }
    }

    /// Append all non-empty frames from a worker update and trim retained
    /// history.
    fn append_frames(&mut self, frames: &[TaggedVec<Float>]) {
        for frame in frames {
            if !frame.data.is_empty() {
                self.history.push_back(frame.data.clone());
                self.pending_rows = self.pending_rows.saturating_add(1);
                while self.history.len() > self.max_frames {
                    self.history.pop_front();
                }
            }
        }
    }

    /// Draw only the waterfall pixels; axes and labels are regular DOM.
    fn draw(&mut self) -> Result<(), JsValue> {
        let (width, height) = resize_canvas_to_display_size(&self.canvas)?;
        self.update_axis_html(self.history.is_empty())?;

        self.ctx.clear_rect(0.0, 0.0, width, height);
        if self.history.is_empty() {
            return Ok(());
        }

        let bitmap_width = width.round().max(1.0) as u32;
        let bitmap_height = self.max_frames.min(height.round().max(1.0) as usize);

        self.update_waterfall_bitmap(bitmap_width, bitmap_height.max(1) as u32)?;
        self.ctx.set_image_smoothing_enabled(false);
        self.ctx
            .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &self.bitmap_canvas,
                0.0,
                0.0,
                f64::from(self.bitmap_width),
                f64::from(self.bitmap_height),
                0.0,
                0.0,
                width,
                height,
            )?;

        Ok(())
    }

    /// Update waterfall tick labels and status visibility outside the canvas.
    fn update_axis_html(&self, show_status: bool) -> Result<(), JsValue> {
        let status_class = if show_status {
            "rr-waterfall-sink-status"
        } else {
            "rr-waterfall-sink-status rr-waterfall-sink-status-hidden"
        };
        self.status.set_attribute("class", status_class)?;
        for (i, tick) in self.tick_labels.iter().enumerate() {
            let t = axis_tick_fraction(i);
            let freq = (t - 0.5) * f64::from(self.sample_rate);
            tick.set_text_content(Some(&format_hz(freq)));
        }
        Ok(())
    }

    /// Update the retained waterfall bitmap. The steady-state path scrolls
    /// existing rows and uploads only the newly appended rows.
    fn update_waterfall_bitmap(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
        if self.bitmap_width != width || self.bitmap_height != height {
            self.bitmap_canvas.set_width(width);
            self.bitmap_canvas.set_height(height);
            self.bitmap_width = width;
            self.bitmap_height = height;
            self.bitmap_needs_rebuild = true;
        }

        if self.bitmap_needs_rebuild || self.pending_rows >= height as usize {
            self.rebuild_waterfall_bitmap()?;
            return Ok(());
        }

        let rows = self
            .pending_rows
            .min(self.history.len())
            .min(height as usize);
        if rows == 0 {
            return Ok(());
        }

        let keep_rows = height as usize - rows;
        if keep_rows > 0 {
            self.bitmap_ctx
                .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                    &self.bitmap_canvas,
                    0.0,
                    rows as f64,
                    f64::from(width),
                    keep_rows as f64,
                    0.0,
                    0.0,
                    f64::from(width),
                    keep_rows as f64,
                )?;
        }

        let first = self.history.len() - rows;
        let mut pixels = vec![0; width as usize * rows * 4];
        for (row, frame) in self.history.iter().skip(first).enumerate() {
            let start = row * width as usize * 4;
            write_waterfall_pixels(
                frame,
                &mut pixels[start..start + width as usize * 4],
                width,
                self.min_db,
                self.max_db,
            );
        }
        put_bitmap_pixels(
            &self.bitmap_ctx,
            pixels,
            width,
            rows as u32,
            0.0,
            (height as usize - rows) as f64,
        )?;
        self.pending_rows = 0;
        Ok(())
    }

    /// Rebuild the whole retained waterfall bitmap after resize or large
    /// batches that replace the entire visible history.
    fn rebuild_waterfall_bitmap(&mut self) -> Result<(), JsValue> {
        let width = self.bitmap_width;
        let height = self.bitmap_height;
        let mut pixels = vec![0; width as usize * height as usize * 4];
        let visible_rows = self.history.len().min(height as usize);
        let first = self.history.len().saturating_sub(visible_rows);
        let top = height as usize - visible_rows;
        for (row, frame) in self.history.iter().skip(first).enumerate() {
            let y = top + row;
            let start = y * width as usize * 4;
            write_waterfall_pixels(
                frame,
                &mut pixels[start..start + width as usize * 4],
                width,
                self.min_db,
                self.max_db,
            );
        }
        put_bitmap_pixels(&self.bitmap_ctx, pixels, width, height, 0.0, 0.0)?;
        self.pending_rows = 0;
        self.bitmap_needs_rebuild = false;
        Ok(())
    }
}

struct CanvasTheme {
    bg: &'static str,
    axis: &'static str,
    grid: &'static str,
    text: &'static str,
    trace: &'static str,
}

impl CanvasTheme {
    /// Read browser color preference and return canvas colors.
    fn current() -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
        let is_dark = window
            .match_media("(prefers-color-scheme: dark)")?
            .is_some_and(|m| m.matches());
        Ok(if is_dark {
            Self {
                bg: "#0b0b0b",
                axis: "#666",
                grid: "#242424",
                text: "#ddd",
                trace: "#7fcdbb",
            }
        } else {
            Self {
                bg: "#ffffff",
                axis: "#888",
                grid: "#e7e7e7",
                text: "#222",
                trace: "#2b8cbe",
            }
        })
    }
}

/// Sanitize a sample rate supplied by the caller.
fn sanitize_sample_rate(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate
    } else {
        1.0
    }
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
            "missing spectrum/waterfall sink element role {role}"
        )))?
        .dyn_into::<T>()
        .map_err(|_| {
            JsValue::from_str(&format!(
                "spectrum/waterfall sink role {role} has wrong element type"
            ))
        })
}

/// Find the fixed waterfall tick/grid elements by their numbered roles.
fn waterfall_role_elements(root: &Element, prefix: &str) -> Result<Vec<Element>, JsValue> {
    (0..AXIS_TICK_COUNT)
        .map(|i| role::<Element>(root, &format!("{prefix}-{i}")))
        .collect()
}

/// Create a canvas and its 2D context for retained bitmap rendering.
fn create_canvas_2d() -> Result<(HtmlCanvasElement, CanvasRenderingContext2d), JsValue> {
    let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let canvas = document
        .create_element("canvas")?
        .dyn_into::<HtmlCanvasElement>()?;
    let ctx = canvas
        .get_context("2d")?
        .ok_or(JsValue::from_str("no 2d context"))?
        .dyn_into::<CanvasRenderingContext2d>()?;
    Ok((canvas, ctx))
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

/// Compute the finite value range in one FFT power frame.
fn data_range(values: &[f32]) -> Option<(f32, f32)> {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for &value in values {
        if !value.is_finite() {
            continue;
        }
        min = min.min(value);
        max = max.max(value);
    }
    if min.is_finite() && max.is_finite() {
        Some((min, max))
    } else {
        None
    }
}

/// Convert one FFT power frame into one RGBA waterfall bitmap row.
fn write_waterfall_pixels(frame: &[f32], pixels: &mut [u8], width: u32, min_db: f32, max_db: f32) {
    if frame.is_empty() {
        return;
    }
    let len = frame.len();
    let half = len / 2;
    for x in 0..width as usize {
        let i = x * len / width as usize;
        let value = frame[(i + half) % len];
        let rgba = waterfall_rgba(value, min_db, max_db);
        let offset = x * 4;
        pixels[offset..offset + 4].copy_from_slice(&rgba);
    }
}

/// Upload RGBA waterfall pixels to a canvas.
fn put_bitmap_pixels(
    ctx: &CanvasRenderingContext2d,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    x: f64,
    y: f64,
) -> Result<(), JsValue> {
    let data = Uint8ClampedArray::new_with_length(pixels.len() as u32);
    data.copy_from(&pixels);
    let image = ImageData::new_with_js_u8_clamped_array_and_sh(&data, width, height)?;
    // With unstable API this function takes i32, not f64.
    ctx.put_image_data(&image, x as _, y as _)
}

#[allow(clippy::too_many_arguments)]
/// Draw spectrum plot axes, grid lines, and labels.
fn draw_spectrum_axes(
    ctx: &CanvasRenderingContext2d,
    theme: &CanvasTheme,
    plot_left: f64,
    plot_top: f64,
    plot_width: f64,
    plot_height: f64,
    sample_rate: f32,
    y_min: f64,
    y_max: f64,
) -> Result<(), JsValue> {
    let plot_right = plot_left + plot_width;
    let plot_bottom = plot_top + plot_height;

    ctx.set_stroke_style_str(theme.grid);
    ctx.set_line_width(1.0);
    for i in 0..AXIS_TICK_COUNT {
        let t = axis_tick_fraction(i);
        let x = plot_left + t * plot_width;
        let y = plot_top + t * plot_height;
        ctx.begin_path();
        ctx.move_to(x, plot_top);
        ctx.line_to(x, plot_bottom);
        ctx.stroke();
        ctx.begin_path();
        ctx.move_to(plot_left, y);
        ctx.line_to(plot_right, y);
        ctx.stroke();
    }

    ctx.set_stroke_style_str(theme.axis);
    ctx.begin_path();
    ctx.move_to(plot_left, plot_top);
    ctx.line_to(plot_left, plot_bottom);
    ctx.line_to(plot_right, plot_bottom);
    ctx.stroke();

    ctx.set_fill_style_str(theme.text);
    ctx.set_font("12px sans-serif");
    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    for i in 0..AXIS_TICK_COUNT {
        let t = axis_tick_fraction(i);
        let freq = (t - 0.5) * f64::from(sample_rate);
        ctx.fill_text(
            &format_hz(freq),
            plot_left + t * plot_width,
            plot_bottom + 6.0,
        )?;
    }

    ctx.set_text_align("right");
    ctx.set_text_baseline("middle");
    for i in 0..AXIS_TICK_COUNT {
        let t = axis_tick_fraction(i);
        let value = y_max - t * (y_max - y_min);
        ctx.fill_text(
            &format!("{value:.0}"),
            plot_left - 6.0,
            plot_top + t * plot_height,
        )?;
    }

    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    ctx.fill_text(
        "Frequency (Hz)",
        plot_left + plot_width / 2.0,
        plot_bottom + 20.0,
    )?;

    ctx.save();
    ctx.translate(plot_left - 40.0, plot_top + plot_height / 2.0)?;
    ctx.rotate(-std::f64::consts::FRAC_PI_2)?;
    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    ctx.fill_text("Power (dB)", 0.0, 0.0)?;
    ctx.restore();

    Ok(())
}

/// Convert a tick index into a 0..1 axis position.
fn axis_tick_fraction(index: usize) -> f64 {
    if AXIS_TICK_COUNT <= 1 {
        0.0
    } else {
        index as f64 / (AXIS_TICK_COUNT - 1) as f64
    }
}

/// Map a power value to the waterfall RGBA palette.
fn waterfall_rgba(value: f32, min_db: f32, max_db: f32) -> [u8; 4] {
    const COLORS: [[u8; 4]; 16] = [
        [0x00, 0x16, 0x5f, 0xff],
        [0x00, 0x2a, 0x86, 0xff],
        [0x00, 0x40, 0xad, 0xff],
        [0x00, 0x59, 0xc8, 0xff],
        [0x00, 0x74, 0xd0, 0xff],
        [0x00, 0x8f, 0xc2, 0xff],
        [0x00, 0xa9, 0xaa, 0xff],
        [0x17, 0xbd, 0x8b, 0xff],
        [0x4c, 0xc8, 0x69, 0xff],
        [0x84, 0xce, 0x4b, 0xff],
        [0xbf, 0xd1, 0x3d, 0xff],
        [0xe8, 0xca, 0x39, 0xff],
        [0xf5, 0xae, 0x32, 0xff],
        [0xf5, 0x8a, 0x2d, 0xff],
        [0xee, 0x63, 0x2f, 0xff],
        [0xeb, 0xeb, 0xeb, 0xff],
    ];
    if !value.is_finite() {
        COLORS[0]
    } else {
        let range = (max_db - min_db).max(f32::EPSILON);
        let t = ((value - min_db) / range).clamp(0.0, 1.0);
        let idx = (t * (COLORS.len() - 1) as f32).round() as usize;
        COLORS[idx]
    }
}

/// Format a frequency value for the axis.
fn format_hz(value: f64) -> String {
    let abs = value.abs();
    if abs >= 1000.0 {
        format!("{:.1}k", value / 1000.0)
    } else {
        format!("{value:.0}")
    }
}
