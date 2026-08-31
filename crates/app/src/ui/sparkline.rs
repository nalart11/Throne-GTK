//! Channel line: a live traffic graph beneath the status.
//!
//! This is the window’s main visual element: it shows that the connection is not
//! merely “connected” but actually carrying traffic. At rest, the line becomes a
//! thin thread along the baseline.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::cairo;
use gtk::prelude::*;

/// Number of samples stored by the graph. At one poll per second, this is two minutes.
const HISTORY: usize = 120;

#[derive(Default)]
struct Samples {
    down: Vec<f64>,
    up: Vec<f64>,
}

/// Graph with sample history.
#[derive(Clone)]
pub struct Sparkline {
    pub widget: gtk::DrawingArea,
    samples: Rc<RefCell<Samples>>,
}

impl Sparkline {
    pub fn new() -> Self {
        let widget = gtk::DrawingArea::builder()
            .height_request(56)
            .margin_top(6)
            .hexpand(true)
            // The line color comes from the widget style rather than hard-coded
            // values: the class delegates it to the theme like the rest of the window.
            .css_classes(["sparkline"])
            .build();
        let samples = Rc::new(RefCell::new(Samples::default()));

        widget.set_draw_func({
            let samples = samples.clone();
            move |area, ctx, width, height| {
                let samples = samples.borrow();
                draw(area, ctx, width as f64, height as f64, &samples);
            }
        });

        // Graph colors are drawn manually, so the widget must track light and dark
        // scheme changes itself: libadwaita repaints the rest, but not the canvas.
        adw::StyleManager::default().connect_dark_notify({
            let widget = widget.clone();
            move |_| widget.queue_draw()
        });

        Self { widget, samples }
    }

    /// Adds a sample: bytes transferred during the last second.
    pub fn push(&self, down: i64, up: i64) {
        {
            let mut s = self.samples.borrow_mut();
            s.down.push(down.max(0) as f64);
            s.up.push(up.max(0) as f64);
            if s.down.len() > HISTORY {
                s.down.remove(0);
                s.up.remove(0);
            }
        }
        self.widget.queue_draw();
    }

    /// Clears the history, for example on disconnect.
    pub fn clear(&self) {
        *self.samples.borrow_mut() = Samples::default();
        self.widget.queue_draw();
    }
}

fn draw(area: &gtk::DrawingArea, ctx: &cairo::Context, width: f64, height: f64, samples: &Samples) {
    // The color is the one the theme provided to the widget through CSS.
    let color = area.color();
    let (r, g, b) = (
        color.red() as f64,
        color.green() as f64,
        color.blue() as f64,
    );

    let baseline = height - 1.0;
    let peak = samples
        .down
        .iter()
        .chain(samples.up.iter())
        .fold(0.0_f64, |a, b| a.max(*b));

    // Below this throughput the graph is a straight thread; otherwise background
    // noise of a few kilobytes would fill the height and look like load. Idle is
    // now distinguished from load by saturation, not color: the window has one
    // color, the theme accent.
    let quiet = peak < 16_384.0;
    let scale = if quiet { 1.0 } else { peak };
    let fade = if quiet { 0.45 } else { 1.0 };

    ctx.set_line_width(1.0);
    ctx.set_source_rgba(r, g, b, 0.28 * fade);
    ctx.move_to(0.0, baseline);
    ctx.line_to(width, baseline);
    let _ = ctx.stroke();

    if samples.down.is_empty() {
        return;
    }

    let step = width / (HISTORY - 1) as f64;
    // The line is drawn left to right and reaches the right edge in two minutes,
    // after which the window starts sliding. Short history cannot be right-aligned:
    // the first seconds would look cut off at the edge.
    let offset = 0.0;

    let point = |index: usize, value: f64| -> (f64, f64) {
        let x = offset + step * index as f64;
        let y = if quiet {
            baseline
        } else {
            // Use a root instead of a linear scale; otherwise rare peaks of tens
            // of megabytes squeeze ordinary activity against zero.
            baseline - (value / scale).sqrt() * (height - 6.0)
        };
        (x, y)
    };

    // Download is filled, upload is outline-only: upload is almost always smaller,
    // and a fill would obscure it.
    for (values, filled) in [(&samples.down, true), (&samples.up, false)] {
        ctx.new_path();
        let mut previous: Option<(f64, f64)> = None;
        for (index, value) in values.iter().enumerate() {
            let (x, y) = point(index, *value);
            match previous {
                None => ctx.move_to(x, y),
                Some((px, py)) => {
                    // Smooth with horizontal tangents: a jump from zero to a
                    // megabyte would otherwise be drawn as a sheer wall, making
                    // the graph read as a fill rather than a line.
                    let control = step / 3.0;
                    ctx.curve_to(px + control, py, x - control, y, x, y);
                }
            }
            previous = Some((x, y));
        }

        if filled {
            let path = ctx.copy_path().ok();
            ctx.set_source_rgba(r, g, b, 0.95 * fade);
            ctx.set_line_width(1.6);
            let _ = ctx.stroke();

            if let Some(path) = path {
                ctx.new_path();
                ctx.append_path(&path);
                let (last_x, _) = point(values.len() - 1, 0.0);
                ctx.line_to(last_x, baseline);
                ctx.line_to(offset, baseline);
                ctx.close_path();
                let gradient = cairo::LinearGradient::new(0.0, 0.0, 0.0, baseline);
                gradient.add_color_stop_rgba(0.0, r, g, b, 0.28 * fade);
                gradient.add_color_stop_rgba(1.0, r, g, b, 0.02 * fade);
                let _ = ctx.set_source(&gradient);
                let _ = ctx.fill();
            }
        } else {
            ctx.set_source_rgba(r, g, b, 0.45 * fade);
            ctx.set_line_width(1.0);
            let _ = ctx.stroke();
        }
    }

    let _ = area;
}
