//! Live connections page.
//!
//! Answers one question: what is currently going through the proxy. Therefore
//! sorting is by downloaded volume, not time: what actually uses the channel is on top.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::engine::Connection;
use crate::format;

/// Users do not read more rows anyway, and redrawing once per second becomes noticeable.
const MAX_ROWS: usize = 120;

pub struct ConnectionsPage {
    pub widget: gtk::Box,
    list: gtk::ListBox,
    summary: gtk::Label,
    empty: adw::StatusPage,
    scroller: gtk::ScrolledWindow,
    last: RefCell<Vec<Connection>>,
}

impl ConnectionsPage {
    pub fn new() -> Rc<Self> {
        let summary = gtk::Label::builder()
            .xalign(0.0)
            .margin_top(10)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["section-label"])
            .label("НЕТ СОЕДИНЕНИЙ")
            .build();

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["navigation-sidebar"])
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("network-transmit-receive-symbolic")
            .title("Соединений нет")
            .description("Здесь появятся запросы приложений, как только пойдёт трафик.")
            .vexpand(true)
            .build();

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        widget.append(&summary);
        widget.append(&scroller);
        widget.append(&empty);
        scroller.set_visible(false);

        Rc::new(Self {
            widget,
            list,
            summary,
            empty,
            scroller,
            last: RefCell::new(Vec::new()),
        })
    }

    pub fn set(&self, mut connections: Vec<Connection>) {
        connections.sort_by_key(|c| std::cmp::Reverse(c.download + c.upload));
        let total = connections.len();
        connections.truncate(MAX_ROWS);

        // Nothing changed, so leave the widgets alone: redrawing the list once
        // per second is noticeable while scrolling.
        if *self.last.borrow() == connections {
            return;
        }

        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for connection in &connections {
            self.list.append(&build_row(connection));
        }

        self.summary.set_label(&match total {
            0 => "НЕТ СОЕДИНЕНИЙ".to_string(),
            n if n > MAX_ROWS => format!("АКТИВНО {n} · ПОКАЗАНО {MAX_ROWS}"),
            n => format!("АКТИВНО {n}"),
        });
        self.scroller.set_visible(total > 0);
        self.empty.set_visible(total == 0);
        *self.last.borrow_mut() = connections;
    }

    pub fn clear(&self) {
        self.set(Vec::new());
    }
}

impl PartialEq for Connection {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.download == other.download && self.upload == other.upload
    }
}

fn build_row(connection: &Connection) -> gtk::ListBoxRow {
    // The domain is not always known (bare IP, sniffing disabled); show the
    // destination address then, while keeping the label in place.
    let title = if connection.domain.is_empty() {
        connection.dest.clone()
    } else {
        connection.domain.clone()
    };

    let name = gtk::Label::builder()
        .label(&title)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .css_classes(["name"])
        .build();

    let mut parts = Vec::new();
    if !connection.process.is_empty() {
        parts.push(connection.process.clone());
    }
    parts.push(connection.network.to_uppercase());
    // created_at is in milliseconds; connections live for seconds, and their
    // age is calculated in seconds as well.
    if connection.created_at > 0 {
        parts.push(format::since(connection.created_at / 1000));
    }
    let process = parts.join(" · ");
    let meta = gtk::Label::builder()
        .label(&process)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["meta"])
        .build();

    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    texts.append(&name);
    texts.append(&meta);

    let volume = gtk::Label::builder()
        .label(format!(
            "↓ {}  ↑ {}",
            format::bytes(connection.download),
            format::bytes(connection.upload)
        ))
        .xalign(1.0)
        .valign(gtk::Align::Center)
        .css_classes(["meta", "numeric"])
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .css_classes(["server-row"])
        .build();
    content.append(&texts);
    content.append(&volume);

    gtk::ListBoxRow::builder()
        .child(&content)
        .activatable(false)
        .build()
}
