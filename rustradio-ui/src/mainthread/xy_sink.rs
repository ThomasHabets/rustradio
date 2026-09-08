//! Responsive arbitrary-X plots for application-level measurements.

use std::borrow::Borrow;
use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, Element, Event, HtmlCanvasElement};

use crate::mainthread::CLASS_SINK;

const CLASS_XY_SINK: &str = "rr-xy-sink-section";
const AXIS_MARGIN_LEFT: f64 = 72.0;
const AXIS_MARGIN_RIGHT: f64 = 16.0;
const AXIS_MARGIN_TOP: f64 = 18.0;
const AXIS_MARGIN_BOTTOM: f64 = 56.0;
const AXIS_TICK_COUNT: usize = 5;

const XY_SINK_HTML: &str = r#"
<div class="rr-panel-header">
  <div>
    <h3 class="rr-panel-title" data-role="title"></h3>
    <p class="rr-panel-kicker" data-role="subtitle"></p>
  </div>
</div>
<div class="rr-panel-body">
  <canvas class="rr-xy-sink-canvas" data-role="canvas"></canvas>
</div>
"#;

/// One arbitrary-X point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XyPoint {
    pub x: f64,
    pub y: f64,
}

impl XyPoint {
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// One labelled and colored plotted series.
#[derive(Clone, Debug, PartialEq)]
pub struct XySeries {
    pub label: String,
    pub color: String,
    pub points: Vec<XyPoint>,
}

/// A labelled X-axis interval drawn behind the plotted series.
#[derive(Clone, Debug, PartialEq)]
pub struct XyRegion {
    pub x_min: f64,
    pub x_max: f64,
    pub label: String,
    pub fill_color: String,
}

/// Tick-label formatting for one plot axis.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum XyAxisFormat {
    #[default]
    Decimal,
    Scientific,
    FrequencyHz,
}

/// Options fixed for the lifetime of an XY sink.
#[derive(Clone, Debug)]
pub struct XySinkOptions {
    pub title: String,
    pub subtitle: String,
    pub x_label: String,
    pub y_label: String,
    pub x_format: XyAxisFormat,
    pub y_format: XyAxisFormat,
    pub include_y_zero: bool,
}

impl Default for XySinkOptions {
    fn default() -> Self {
        Self {
            title: "Plot".into(),
            subtitle: String::new(),
            x_label: "X".into(),
            y_label: "Y".into(),
            x_format: XyAxisFormat::Decimal,
            y_format: XyAxisFormat::Decimal,
            include_y_zero: false,
        }
    }
}

/// Handle to a responsive arbitrary-X canvas plot.
#[derive(Clone)]
pub struct XySink {
    inner: Rc<RefCell<XyInner>>,
}

impl XySink {
    /// Find a mount element by ID and replace its contents with an XY sink.
    pub fn mount_by_id(id: &str, options: impl Borrow<XySinkOptions>) -> rustradio::Result<Self> {
        let root = dom_result(
            get_element_by_id(id),
            &format!("finding XY sink mount {id}"),
        )?;
        Self::mount(&root, options)
    }

    /// Mount a self-contained XY sink into an existing DOM element.
    pub fn mount(root: &Element, options: impl Borrow<XySinkOptions>) -> rustradio::Result<Self> {
        dom_result(Self::mount_dom(root, options.borrow()), "mounting XY sink")
    }

    /// Replace all series and regions, then redraw the plot.
    pub fn update(&self, series: Vec<XySeries>, regions: Vec<XyRegion>) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.series = series;
        inner.regions = regions;
        dom_result(inner.draw(), "updating XY sink")
    }

    /// Drop retained data and redraw the empty plot.
    pub fn clear(&self) -> rustradio::Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.series.clear();
        inner.regions.clear();
        dom_result(inner.draw(), "clearing XY sink")
    }

    /// Export the current canvas, including axes and legend, as a PNG data URL.
    pub fn png_data_url(&self) -> rustradio::Result<String> {
        dom_result(
            self.inner
                .as_ref()
                .borrow()
                .canvas
                .to_data_url_with_type("image/png"),
            "exporting XY sink PNG",
        )
    }

    fn mount_dom(root: &Element, options: &XySinkOptions) -> Result<Self, JsValue> {
        root.set_inner_html(XY_SINK_HTML);
        root.class_list().add_2(CLASS_SINK, CLASS_XY_SINK)?;
        role::<Element>(root, "title")?.set_text_content(Some(&options.title));
        role::<Element>(root, "subtitle")?.set_text_content(Some(&options.subtitle));

        let canvas = role::<HtmlCanvasElement>(root, "canvas")?;
        let ctx = canvas
            .get_context("2d")?
            .ok_or(JsValue::from_str("no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let sink = Self {
            inner: Rc::new(RefCell::new(XyInner {
                canvas,
                ctx,
                options: options.clone(),
                series: Vec::new(),
                regions: Vec::new(),
                callbacks: Vec::new(),
            })),
        };
        sink.install_handlers()?;
        sink.inner.borrow_mut().draw()?;
        Ok(sink)
    }

    fn install_handlers(&self) -> Result<(), JsValue> {
        let inner = self.inner.clone();
        let handler = Closure::<dyn FnMut(Event)>::new(move |_event| {
            if let Err(error) = inner.borrow_mut().draw() {
                log::error!("XY sink resize failed: {error:?}");
            }
        });
        web_sys::window()
            .ok_or(JsValue::from_str("no window"))?
            .add_event_listener_with_callback("resize", handler.as_ref().unchecked_ref())?;
        self.inner.borrow_mut().callbacks.push(handler);
        Ok(())
    }
}

struct XyInner {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    options: XySinkOptions,
    series: Vec<XySeries>,
    regions: Vec<XyRegion>,
    callbacks: Vec<Closure<dyn FnMut(Event)>>,
}

impl XyInner {
    fn draw(&mut self) -> Result<(), JsValue> {
        let (width, height) = resize_canvas_to_display_size(&self.canvas)?;
        let theme = CanvasTheme::current()?;
        self.ctx.set_fill_style_str(theme.bg);
        self.ctx.fill_rect(0.0, 0.0, width, height);
        self.ctx.set_stroke_style_str(theme.axis);
        self.ctx
            .stroke_rect(0.5, 0.5, (width - 1.0).max(0.0), (height - 1.0).max(0.0));

        let Some(((x_min, x_max), (y_min, y_max))) =
            plot_ranges(&self.series, self.options.include_y_zero)
        else {
            self.ctx.set_fill_style_str(theme.text);
            self.ctx.set_font("12px sans-serif");
            self.ctx.fill_text("Waiting for plot data...", 12.0, 20.0)?;
            return Ok(());
        };

        let plot_left = AXIS_MARGIN_LEFT.min((width - 1.0).max(0.0));
        let plot_top = AXIS_MARGIN_TOP.min((height - 1.0).max(0.0));
        let plot_width = (width - AXIS_MARGIN_LEFT - AXIS_MARGIN_RIGHT).max(1.0);
        let plot_height = (height - AXIS_MARGIN_TOP - AXIS_MARGIN_BOTTOM).max(1.0);
        self.draw_regions(
            &theme,
            plot_left,
            plot_top,
            plot_width,
            plot_height,
            x_min,
            x_max,
        )?;
        draw_axes(
            &self.ctx,
            &theme,
            &self.options,
            PlotArea {
                left: plot_left,
                top: plot_top,
                width: plot_width,
                height: plot_height,
            },
            (x_min, x_max),
            (y_min, y_max),
        )?;
        self.draw_series(
            plot_left,
            plot_top,
            plot_width,
            plot_height,
            (x_min, x_max),
            (y_min, y_max),
        );
        self.draw_legend(&theme, plot_left, plot_top + plot_height + 39.0)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_regions(
        &self,
        theme: &CanvasTheme,
        plot_left: f64,
        plot_top: f64,
        plot_width: f64,
        plot_height: f64,
        x_min: f64,
        x_max: f64,
    ) -> Result<(), JsValue> {
        let x_range = x_max - x_min;
        for region in &self.regions {
            let low = region.x_min.max(x_min);
            let high = region.x_max.min(x_max);
            if !low.is_finite() || !high.is_finite() || high <= low {
                continue;
            }
            let left = plot_left + (low - x_min) / x_range * plot_width;
            let right = plot_left + (high - x_min) / x_range * plot_width;
            self.ctx.set_fill_style_str(&region.fill_color);
            self.ctx
                .fill_rect(left, plot_top, right - left, plot_height);
            self.ctx.set_stroke_style_str(theme.axis);
            self.ctx
                .stroke_rect(left, plot_top, right - left, plot_height);
            if right - left >= 20.0 && !region.label.is_empty() {
                self.ctx.set_fill_style_str(theme.text);
                self.ctx.set_font("12px sans-serif");
                self.ctx.set_text_align("center");
                self.ctx.set_text_baseline("top");
                self.ctx
                    .fill_text(&region.label, (left + right) / 2.0, plot_top + 7.0)?;
            }
        }
        Ok(())
    }

    fn draw_series(
        &self,
        plot_left: f64,
        plot_top: f64,
        plot_width: f64,
        plot_height: f64,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        let bucket_count = plot_width.round().max(1.0) as usize;
        for series in &self.series {
            self.ctx.set_stroke_style_str(&series.color);
            self.ctx.set_fill_style_str(&series.color);
            self.ctx.set_line_width(1.0);
            for (bucket, extent) in bucket_extents(&series.points, x_range, bucket_count)
                .into_iter()
                .enumerate()
            {
                let Some((minimum, maximum)) = extent else {
                    continue;
                };
                let x = plot_left + (bucket as f64 + 0.5) / bucket_count as f64 * plot_width;
                let y_min = graph_y(maximum, plot_top, plot_height, y_range);
                let y_max = graph_y(minimum, plot_top, plot_height, y_range);
                if (y_max - y_min).abs() < 0.75 {
                    self.ctx.fill_rect(x - 0.6, y_min - 0.6, 1.2, 1.2);
                } else {
                    self.ctx.begin_path();
                    self.ctx.move_to(x, y_min);
                    self.ctx.line_to(x, y_max);
                    self.ctx.stroke();
                }
            }
        }
    }

    fn draw_legend(&self, theme: &CanvasTheme, left: f64, y: f64) -> Result<(), JsValue> {
        let mut x = left;
        self.ctx.set_font("12px sans-serif");
        self.ctx.set_text_align("left");
        self.ctx.set_text_baseline("middle");
        for series in &self.series {
            self.ctx.set_fill_style_str(&series.color);
            self.ctx.fill_rect(x, y - 2.0, 12.0, 4.0);
            self.ctx.set_fill_style_str(theme.text);
            self.ctx.fill_text(&series.label, x + 17.0, y)?;
            x += 25.0 + series.label.len() as f64 * 7.0;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct PlotArea {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

#[allow(clippy::too_many_arguments)]
fn draw_axes(
    ctx: &CanvasRenderingContext2d,
    theme: &CanvasTheme,
    options: &XySinkOptions,
    plot: PlotArea,
    x_range: (f64, f64),
    y_range: (f64, f64),
) -> Result<(), JsValue> {
    let right = plot.left + plot.width;
    let bottom = plot.top + plot.height;
    ctx.set_line_width(1.0);
    ctx.set_stroke_style_str(theme.grid);
    for index in 0..AXIS_TICK_COUNT {
        let t = tick_fraction(index);
        let x = plot.left + t * plot.width;
        let y = plot.top + t * plot.height;
        ctx.begin_path();
        ctx.move_to(x, plot.top);
        ctx.line_to(x, bottom);
        ctx.stroke();
        ctx.begin_path();
        ctx.move_to(plot.left, y);
        ctx.line_to(right, y);
        ctx.stroke();
    }
    ctx.set_stroke_style_str(theme.axis);
    ctx.begin_path();
    ctx.move_to(plot.left, plot.top);
    ctx.line_to(plot.left, bottom);
    ctx.line_to(right, bottom);
    ctx.stroke();

    ctx.set_fill_style_str(theme.text);
    ctx.set_font("12px sans-serif");
    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    for index in 0..AXIS_TICK_COUNT {
        let t = tick_fraction(index);
        let value = x_range.0 + t * (x_range.1 - x_range.0);
        ctx.fill_text(
            &format_axis(value, options.x_format),
            plot.left + t * plot.width,
            bottom + 6.0,
        )?;
    }
    ctx.fill_text(
        &options.x_label,
        plot.left + plot.width / 2.0,
        bottom + 22.0,
    )?;

    ctx.set_text_align("right");
    ctx.set_text_baseline("middle");
    for index in 0..AXIS_TICK_COUNT {
        let t = tick_fraction(index);
        let value = y_range.1 - t * (y_range.1 - y_range.0);
        ctx.fill_text(
            &format_axis(value, options.y_format),
            plot.left - 7.0,
            plot.top + t * plot.height,
        )?;
    }
    ctx.save();
    ctx.translate(plot.left - 54.0, plot.top + plot.height / 2.0)?;
    ctx.rotate(-std::f64::consts::FRAC_PI_2)?;
    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    ctx.fill_text(&options.y_label, 0.0, 0.0)?;
    ctx.restore();
    Ok(())
}

fn plot_ranges(series: &[XySeries], include_y_zero: bool) -> Option<((f64, f64), (f64, f64))> {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;
    for point in series.iter().flat_map(|series| &series.points) {
        if !point.x.is_finite() || !point.y.is_finite() {
            continue;
        }
        x_min = x_min.min(point.x);
        x_max = x_max.max(point.x);
        y_min = y_min.min(point.y);
        y_max = y_max.max(point.y);
    }
    if !(x_min.is_finite() && x_max.is_finite() && y_min.is_finite() && y_max.is_finite()) {
        return None;
    }
    if x_max <= x_min {
        x_min -= 0.5;
        x_max += 0.5;
    }
    if include_y_zero {
        y_min = y_min.min(0.0);
        y_max = y_max.max(0.0);
    }
    if y_max <= y_min {
        let padding = y_max.abs().max(1.0) * 0.05;
        y_min -= padding;
        y_max += padding;
    } else {
        let padding = (y_max - y_min) * 0.05;
        if include_y_zero && y_min == 0.0 {
            y_max += padding;
        } else {
            y_min -= padding;
            y_max += padding;
        }
    }
    Some(((x_min, x_max), (y_min, y_max)))
}

fn bucket_extents(
    points: &[XyPoint],
    x_range: (f64, f64),
    bucket_count: usize,
) -> Vec<Option<(f64, f64)>> {
    let mut buckets = vec![None; bucket_count.max(1)];
    let width = x_range.1 - x_range.0;
    if !width.is_finite() || width <= 0.0 {
        return buckets;
    }
    let last = buckets.len() - 1;
    for point in points {
        if !point.x.is_finite()
            || !point.y.is_finite()
            || point.x < x_range.0
            || point.x > x_range.1
        {
            continue;
        }
        let bucket =
            (((point.x - x_range.0) / width * buckets.len() as f64).floor() as usize).min(last);
        buckets[bucket] = Some(match buckets[bucket] {
            Some((minimum, maximum)) => (minimum.min(point.y), maximum.max(point.y)),
            None => (point.y, point.y),
        });
    }
    buckets
}

fn graph_y(value: f64, top: f64, height: f64, range: (f64, f64)) -> f64 {
    top + height - (value - range.0) / (range.1 - range.0) * height
}

fn format_axis(value: f64, format: XyAxisFormat) -> String {
    match format {
        XyAxisFormat::Decimal => {
            if value.abs() >= 100.0 {
                format!("{value:.0}")
            } else if value.abs() >= 1.0 {
                format!("{value:.2}")
            } else {
                format!("{value:.3}")
            }
        }
        XyAxisFormat::Scientific => format!("{value:.1e}"),
        XyAxisFormat::FrequencyHz => format_hz(value),
    }
}

fn format_hz(value: f64) -> String {
    let abs = value.abs();
    if abs >= 1e9 {
        format!("{:.3}G", value / 1e9)
    } else if abs >= 1e6 {
        format!("{:.3}M", value / 1e6)
    } else if abs >= 1e3 {
        format!("{:.3}k", value / 1e3)
    } else {
        format!("{value:.0}")
    }
}

fn tick_fraction(index: usize) -> f64 {
    index as f64 / (AXIS_TICK_COUNT - 1) as f64
}

struct CanvasTheme {
    bg: &'static str,
    axis: &'static str,
    grid: &'static str,
    text: &'static str,
}

impl CanvasTheme {
    fn current() -> Result<Self, JsValue> {
        let dark = web_sys::window()
            .ok_or(JsValue::from_str("no window"))?
            .match_media("(prefers-color-scheme: dark)")?
            .is_some_and(|media| media.matches());
        Ok(if dark {
            Self {
                bg: "#0b0b0b",
                axis: "#777",
                grid: "#292929",
                text: "#ddd",
            }
        } else {
            Self {
                bg: "#fff",
                axis: "#888",
                grid: "#ddd",
                text: "#222",
            }
        })
    }
}

fn dom_result<T>(result: Result<T, JsValue>, context: &str) -> rustradio::Result<T> {
    result.map_err(|error| {
        let detail = error.as_string().unwrap_or_else(|| format!("{error:?}"));
        rustradio::Error::msg(format!("{context}: {detail}"))
    })
}

fn get_element_by_id(id: &str) -> Result<Element, JsValue> {
    web_sys::window()
        .ok_or(JsValue::from_str("no window"))?
        .document()
        .ok_or(JsValue::from_str("no document"))?
        .get_element_by_id(id)
        .ok_or(JsValue::from_str(&format!(
            "can't find element with id {id}"
        )))
}

fn role<T: JsCast>(root: &Element, name: &str) -> Result<T, JsValue> {
    root.query_selector(&format!("[data-role=\"{name}\"]"))?
        .ok_or(JsValue::from_str(&format!("missing XY sink role {name}")))?
        .dyn_into::<T>()
        .map_err(|_| JsValue::from_str(&format!("XY sink role {name} has wrong type")))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_ignore_non_finite_points_and_include_zero() {
        let series = vec![XySeries {
            label: "a".into(),
            color: "blue".into(),
            points: vec![
                XyPoint::new(10.0, 2.0),
                XyPoint::new(20.0, 4.0),
                XyPoint::new(f64::NAN, 9.0),
            ],
        }];
        let ((x_min, x_max), (y_min, y_max)) = plot_ranges(&series, true).unwrap();
        assert_eq!((x_min, x_max), (10.0, 20.0));
        assert_eq!(y_min, 0.0);
        assert!(y_max > 4.0);
    }

    #[test]
    fn dense_buckets_keep_extrema() {
        let points = vec![
            XyPoint::new(0.0, 1.0),
            XyPoint::new(0.1, -4.0),
            XyPoint::new(0.2, 8.0),
            XyPoint::new(1.0, 2.0),
        ];
        let buckets = bucket_extents(&points, (0.0, 1.0), 2);
        assert_eq!(buckets[0], Some((-4.0, 8.0)));
        assert_eq!(buckets[1], Some((2.0, 2.0)));
    }

    #[test]
    fn formats_frequency_and_scientific_ticks() {
        assert_eq!(format_axis(5.8e9, XyAxisFormat::FrequencyHz), "5.800G");
        assert_eq!(format_axis(3.5e-10, XyAxisFormat::Scientific), "3.5e-10");
    }
}
