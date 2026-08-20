//! Линия канала — живой график трафика под статусом.
//!
//! Это главный визуальный элемент окна: по нему видно, что соединение не
//! просто «подключено», а действительно несёт трафик. В покое линия
//! выпрямляется в тонкую нить у основания.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::cairo;
use gtk::prelude::*;

/// Сколько отсчётов хранит график. При опросе раз в секунду это две минуты.
const HISTORY: usize = 120;

#[derive(Default)]
struct Samples {
    down: Vec<f64>,
    up: Vec<f64>,
}

/// График с историей отсчётов.
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
            // Цвет линии берётся из стиля виджета, а не задаётся числами:
            // класс отдаёт его теме, как и всему остальному в окне.
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

        // Цвета графика рисуются вручную, поэтому смену светлой и тёмной схемы
        // виджет обязан отслеживать сам: остальной интерфейс перекрашивает
        // libadwaita, а холст — нет.
        adw::StyleManager::default().connect_dark_notify({
            let widget = widget.clone();
            move |_| widget.queue_draw()
        });

        Self { widget, samples }
    }

    /// Добавляет отсчёт: байты за прошедшую секунду.
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

    /// Сбрасывает историю — например, при отключении.
    pub fn clear(&self) {
        *self.samples.borrow_mut() = Samples::default();
        self.widget.queue_draw();
    }
}

fn draw(area: &gtk::DrawingArea, ctx: &cairo::Context, width: f64, height: f64, samples: &Samples) {
    // Цвет — тот, что тема дала виджету через CSS.
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

    // Ниже этого потока график — прямая нить: иначе фоновый шум в пару
    // килобайт рисовался бы во всю высоту и выглядел как нагрузка. Покой от
    // нагрузки отличается теперь не цветом, а насыщенностью: цвет в окне
    // ровно один, и это акцент темы.
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
    // Линия пишется слева направо и доходит до правого края за две минуты,
    // после чего окно начинает скользить. Прижимать короткую историю вправо
    // нельзя: первые секунды выглядели бы обрубком у самого края.
    let offset = 0.0;

    let point = |index: usize, value: f64| -> (f64, f64) {
        let x = offset + step * index as f64;
        let y = if quiet {
            baseline
        } else {
            // Корень вместо линейной шкалы: иначе редкие пики в десятки
            // мегабайт прижимают обычную работу к нулю.
            baseline - (value / scale).sqrt() * (height - 6.0)
        };
        (x, y)
    };

    // Приём — заливка, отдача — только контур: у отдачи почти всегда меньший
    // масштаб, и заливка перекрывала бы её.
    for (values, filled) in [(&samples.down, true), (&samples.up, false)] {
        ctx.new_path();
        let mut previous: Option<(f64, f64)> = None;
        for (index, value) in values.iter().enumerate() {
            let (x, y) = point(index, *value);
            match previous {
                None => ctx.move_to(x, y),
                Some((px, py)) => {
                    // Сглаживание горизонтальными касательными: скачок с нуля
                    // до мегабайта иначе рисуется отвесной стеной, и график
                    // читается как заливка, а не как линия.
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
