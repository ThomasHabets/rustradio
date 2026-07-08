//! Like time sink, this is mostly LLM coded. It does work as a proof of
//! concept, but it needs to be properly reviewed.
use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use rustradio::Complex;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, Element, Event, HtmlCanvasElement};

use crate::TaggedVec;
use crate::mainthread::CLASS_SINK;

const CLASS_CONSTELLATION_SINK: &str = "rr-constellation-sink-section";

const CONSTELLATION_SINK_HTML: &str = r#"
<div class="rr-panel-header">
  <div>
    <h3 class="rr-panel-title" data-role="title"></h3>
    <p class="rr-panel-kicker" data-role="subtitle"></p>
  </div>
</div>
<div class="rr-panel-body">
  <canvas class="rr-constellation-sink-canvas" data-role="canvas"></canvas>
</div>
"#;

const DEFAULT_MAX_CONSTELLATION_POINTS: usize = 5_000;
const AXIS_MARGIN: f64 = 28.0;
const GRID_LINES: i32 = 4;

/// Convert browser/DOM failures into the crate-level error type exposed by the
/// constellation sink API.
fn dom_result<T>(result: Result<T, JsValue>, context: &str) -> rustradio::Result<T> {
    result.map_err(|err| {
        let detail = err.as_string().unwrap_or_else(|| format!("{err:?}"));
        rustradio::Error::msg(format!("{context}: {detail}"))
    })
}

/// Options for the constellation sink.
#[derive(Debug, Clone)]
pub struct ConstellationSinkOptions {
    pub title: String,
    pub subtitle: String,
    pub max_points: usize,
}

impl Default for ConstellationSinkOptions {
    /// Build a generic constellation sink configuration for callers that only
    /// need a mount point and complex stream updates.
    fn default() -> Self {
        Self {
            title: "Constellation".into(),
            subtitle: "I/Q sample plane".into(),
            max_points: DEFAULT_MAX_CONSTELLATION_POINTS,
        }
    }
}

/// Constellation sink. This is a handle to a graph element where complex
/// samples are shown on the I/Q plane.
///
/// The constellation sink can graph multiple streams at the same time using
/// different point colors.
#[derive(Clone)]
pub struct ConstellationSink {
    inner: Rc<RefCell<Inner>>,
}

impl ConstellationSink {
    /// Find a mount element by ID and replace its contents with a constellation
    /// sink.
    pub fn mount_by_id(
        id: &str,
        options: impl Borrow<ConstellationSinkOptions>,
    ) -> rustradio::Result<Self> {
        let options = options.borrow();
        let root = dom_result(
            get_element_by_id(id),
            &format!("finding constellation sink mount element {id}"),
        )?;
        Self::mount(&root, options)
    }

    /// Mount a self-contained constellation sink into an existing DOM element.
    pub fn mount(root: &Element, options: &ConstellationSinkOptions) -> rustradio::Result<Self> {
        dom_result(
            Self::mount_dom(root, options),
            "mounting constellation sink",
        )
    }

    /// Add new tagged complex streams to the sink and redraw.
    #[allow(clippy::needless_pass_by_value)]
    pub fn update(&self, streams: Vec<TaggedVec<Complex>>) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.append_streams(&streams);
        dom_result(inner.draw(), "updating constellation sink")
    }

    /// Drop all buffered series data and redraw the empty sink.
    #[allow(dead_code)]
    pub fn clear(&self) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.series.clear();
        dom_result(inner.draw(), "clearing constellation sink")
    }

    /// Mount the generated DOM and wire handlers, preserving browser-native
    /// errors until the public API boundary.
    fn mount_dom(root: &Element, options: &ConstellationSinkOptions) -> Result<Self, JsValue> {
        root.set_inner_html(CONSTELLATION_SINK_HTML);
        root.class_list()
            .add_2(CLASS_SINK, CLASS_CONSTELLATION_SINK)?;

        role::<Element>(root, "title")?.set_text_content(Some(&options.title));
        role::<Element>(root, "subtitle")?.set_text_content(Some(&options.subtitle));

        let canvas = role::<HtmlCanvasElement>(root, "canvas")?;
        let ctx = canvas
            .get_context("2d")?
            .ok_or(JsValue::from_str("no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;

        let sink = Self {
            inner: Rc::new(RefCell::new(Inner {
                canvas,
                ctx,
                series: Vec::new(),
                max_points: options.max_points.max(1),
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
                log::error!("constellation sink resize failed: {err:?}");
            }
        });
        let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
        window.add_event_listener_with_callback("resize", handler.as_ref().unchecked_ref())?;
        self.inner.borrow_mut().callbacks.push(handler);

        Ok(())
    }
}

struct ConstellationSeries {
    points: VecDeque<Complex>,
}

impl ConstellationSeries {
    /// Create one buffered plotted series with room for the first update.
    fn new(capacity: usize) -> Self {
        Self {
            points: VecDeque::with_capacity(capacity),
        }
    }

    /// Append complex samples and trim old points beyond the retention cap.
    fn append_points(&mut self, points: &[Complex], max_points: usize) {
        self.points.extend(points.iter().copied());
        while self.points.len() > max_points {
            self.points.pop_front();
        }
    }
}

struct Inner {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    series: Vec<ConstellationSeries>,
    max_points: usize,
    callbacks: Vec<Closure<dyn FnMut(Event)>>,
}

impl Inner {
    /// Append all streams from one update, creating series as needed.
    fn append_streams(&mut self, streams: &[TaggedVec<Complex>]) {
        for (idx, stream) in streams.iter().enumerate() {
            if self.series.len() <= idx {
                self.series.push(ConstellationSeries::new(
                    stream.data.len().min(self.max_points),
                ));
            }
            self.series[idx].append_points(&stream.data, self.max_points);
        }
    }

    /// Draw the full canvas, including axes, autoscale, and constellation
    /// points.
    fn draw(&mut self) -> Result<(), JsValue> {
        let (width, height) = resize_canvas_to_display_size(&self.canvas)?;

        let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
        let is_dark = window
            .match_media("(prefers-color-scheme: dark)")?
            .is_some_and(|m| m.matches());
        let bg = if is_dark { "#0b0b0b" } else { "#ffffff" };
        let axis = if is_dark { "#666" } else { "#888" };
        let grid = if is_dark { "#242424" } else { "#e7e7e7" };
        let text = if is_dark { "#ddd" } else { "#222" };
        let colors = ["#2b8cbe", "#31a354", "#756bb1", "#e6550d"];

        self.ctx.set_fill_style_str(bg);
        self.ctx.fill_rect(0.0, 0.0, width, height);
        self.ctx.set_stroke_style_str(axis);
        self.ctx
            .stroke_rect(0.5, 0.5, (width - 1.0).max(0.0), (height - 1.0).max(0.0));

        if !self.series.iter().any(|series| !series.points.is_empty()) {
            self.ctx.set_fill_style_str(text);
            self.ctx.set_font("12px sans-serif");
            self.ctx.fill_text("Waiting for IQ data...", 12.0, 20.0)?;
            return Ok(());
        }

        let plot_left = AXIS_MARGIN.min((width - 1.0).max(0.0));
        let plot_top = AXIS_MARGIN.min((height - 1.0).max(0.0));
        let plot_width = (width - AXIS_MARGIN * 2.0).max(1.0);
        let plot_height = (height - AXIS_MARGIN * 2.0).max(1.0);
        let center_x = plot_left + plot_width / 2.0;
        let center_y = plot_top + plot_height / 2.0;
        let plot_radius = plot_width.min(plot_height) / 2.0;
        let max_abs = self.data_max_abs().unwrap_or(1.0).max(1e-6) * 1.08;

        draw_axes(
            &self.ctx,
            grid,
            axis,
            text,
            center_x,
            center_y,
            plot_radius,
            max_abs,
        )?;

        for (idx, series) in self.series.iter().enumerate() {
            self.ctx.set_fill_style_str(colors[idx % colors.len()]);
            for point in &series.points {
                if !point.re.is_finite() || !point.im.is_finite() {
                    continue;
                }
                let x = center_x + (f64::from(point.re) / f64::from(max_abs)) * plot_radius;
                let y = center_y - (f64::from(point.im) / f64::from(max_abs)) * plot_radius;
                self.ctx.fill_rect(x - 1.5, y - 1.5, 3.0, 3.0);
            }
        }
        Ok(())
    }

    /// Compute the largest absolute I or Q value in the retained points.
    fn data_max_abs(&self) -> Option<f32> {
        let mut max_abs = 0.0_f32;
        for series in &self.series {
            for point in &series.points {
                if point.re.is_finite() {
                    max_abs = max_abs.max(point.re.abs());
                }
                if point.im.is_finite() {
                    max_abs = max_abs.max(point.im.abs());
                }
            }
        }
        if max_abs > 0.0 { Some(max_abs) } else { None }
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
            "missing constellation sink element role {role}"
        )))?
        .dyn_into::<T>()
        .map_err(|_| {
            JsValue::from_str(&format!(
                "constellation sink role {role} has wrong element type"
            ))
        })
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
/// Draw plot axes, grid lines, and axis labels.
fn draw_axes(
    ctx: &CanvasRenderingContext2d,
    grid: &str,
    axis: &str,
    text: &str,
    center_x: f64,
    center_y: f64,
    plot_radius: f64,
    max_abs: f32,
) -> Result<(), JsValue> {
    let grid_left = center_x - plot_radius;
    let grid_right = center_x + plot_radius;
    let grid_top = center_y - plot_radius;
    let grid_bottom = center_y + plot_radius;

    ctx.set_stroke_style_str(grid);
    ctx.set_line_width(1.0);
    for i in -GRID_LINES..=GRID_LINES {
        let t = f64::from(i) / f64::from(GRID_LINES);
        let x = center_x + t * plot_radius;
        let y = center_y + t * plot_radius;
        ctx.begin_path();
        ctx.move_to(x, grid_top);
        ctx.line_to(x, grid_bottom);
        ctx.stroke();
        ctx.begin_path();
        ctx.move_to(grid_left, y);
        ctx.line_to(grid_right, y);
        ctx.stroke();
    }

    ctx.set_stroke_style_str(axis);
    ctx.begin_path();
    ctx.move_to(grid_left, center_y);
    ctx.line_to(grid_right, center_y);
    ctx.move_to(center_x, grid_top);
    ctx.line_to(center_x, grid_bottom);
    ctx.stroke();

    ctx.set_fill_style_str(text);
    ctx.set_font("12px sans-serif");
    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    ctx.fill_text("I", grid_right - 6.0, center_y + 6.0)?;
    ctx.set_text_align("left");
    ctx.set_text_baseline("middle");
    ctx.fill_text("Q", center_x + 6.0, grid_top + 8.0)?;
    ctx.set_text_align("right");
    ctx.set_text_baseline("bottom");
    ctx.fill_text(
        &format!("+/-{}", format_tick(f64::from(max_abs))),
        grid_right - 6.0,
        grid_bottom - 6.0,
    )?;

    Ok(())
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
