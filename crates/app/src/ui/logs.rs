//! Журнал ядра.

use std::rc::Rc;

use adw::prelude::*;

use crate::state::State;

pub struct LogsPage {
    pub widget: gtk::Box,
    view: gtk::TextView,
    scroller: gtk::ScrolledWindow,
    state: Rc<State>,
}

impl LogsPage {
    pub fn new(state: Rc<State>) -> Rc<Self> {
        let view = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .css_classes(["log-view"])
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&view)
            .build();

        let copy = gtk::Button::builder()
            .label("Скопировать")
            .tooltip_text("Скопировать журнал в буфер обмена")
            .build();
        let clear = gtk::Button::builder().label("Очистить").build();

        let toolbar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::End)
            .margin_top(10)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        toolbar.append(&copy);
        toolbar.append(&clear);

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        widget.append(&toolbar);
        widget.append(&scroller);

        let page = Rc::new(Self {
            widget,
            view,
            scroller,
            state,
        });

        copy.connect_clicked({
            let page = Rc::downgrade(&page);
            move |button| {
                let Some(page) = page.upgrade() else { return };
                let text = page.state.log.borrow().iter().cloned().collect::<Vec<_>>().join("\n");
                button.clipboard().set_text(&text);
            }
        });
        clear.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                let Some(page) = page.upgrade() else { return };
                page.state.log.borrow_mut().clear();
                page.refresh();
            }
        });

        page
    }

    pub fn refresh(&self) {
        let buffer = self.view.buffer();
        // Прокрутку двигаем только если пользователь и так внизу: иначе
        // читать середину журнала на активном соединении невозможно.
        let adjustment = self.scroller.vadjustment();
        let at_bottom =
            adjustment.value() + adjustment.page_size() >= adjustment.upper() - 24.0;

        let text = self
            .state
            .log
            .borrow()
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        buffer.set_text(&text);

        if at_bottom {
            let end = buffer.end_iter();
            let mark = buffer.create_mark(None, &end, false);
            self.view.scroll_mark_onscreen(&mark);
            buffer.delete_mark(&mark);
        }
    }
}
