//! Страница со списком серверов.
//!
//! Строка показывает ровно то, по чему выбирают сервер: имя, куда он ведёт и
//! насколько быстро отвечает. Всё остальное — в контекстном меню.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;

use crate::format;
use crate::state::State;
use throne_config::profile::Group;

pub struct ServersPage {
    pub widget: gtk::Box,
    state: Rc<State>,
    list: gtk::ListBox,
    groups_drop: gtk::DropDown,
    /// Модель списка групп создаётся один раз и дальше только наполняется:
    /// подмена модели у DropDown освобождает его внутреннее выделение, и
    /// следующий set_selected обращается к уже мёртвому объекту — падение
    /// в g_object_notify_by_pspec.
    groups_model: gtk::StringList,
    group_info: gtk::Label,
    test_button: gtk::Button,
    update_button: gtk::Button,
    search: gtk::SearchEntry,
    empty: adw::StatusPage,
    scroller: gtk::ScrolledWindow,

    /// Строки по id профиля — чтобы точечно обновлять задержку, не перестраивая
    /// список: при двух сотнях серверов пересборка заметна глазом.
    rows: RefCell<HashMap<i64, Row>>,
    groups: RefCell<Vec<Group>>,
    on_activated: RefCell<Option<Rc<dyn Fn()>>>,
    on_selection: RefCell<Option<Rc<dyn Fn()>>>,
    on_notify: RefCell<Option<Rc<dyn Fn(String)>>>,
    on_speed_test: RefCell<Option<Rc<dyn Fn(i64)>>>,
    on_auto_pin: RefCell<Option<Rc<dyn Fn(String)>>>,
    /// Список приводится в соответствие данным: сигналы виджетов в это время
    /// не означают действие человека.
    syncing: std::cell::Cell<bool>,
}

#[derive(Clone)]
struct Row {
    root: gtk::ListBoxRow,
    /// Внутренний контейнер: именно на нём висит оформление строки, включая
    /// засечку активного сервера.
    content: gtk::Box,
    latency: gtk::Label,
    /// Скорость и страна из последнего замера; пусто — не мерили.
    speed: gtk::Label,
}

impl ServersPage {
    pub fn new(state: Rc<State>) -> Rc<Self> {
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Поиск по имени или адресу")
            .hexpand(true)
            .build();

        let groups_model = gtk::StringList::new(&[]);
        let groups_drop = gtk::DropDown::builder()
            .tooltip_text("Группа серверов")
            .model(&groups_model)
            .build();

        let test_button = gtk::Button::builder()
            .icon_name("network-cellular-signal-excellent-symbolic")
            .tooltip_text("Проверить задержки всех серверов группы")
            .build();
        let update_button = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Обновить подписку")
            .build();

        let toolbar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .margin_top(10)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        toolbar.append(&groups_drop);
        toolbar.append(&search);
        toolbar.append(&test_button);
        toolbar.append(&update_button);

        let group_info = gtk::Label::builder()
            .xalign(0.0)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(4)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["section-label"])
            .build();

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("network-server-symbolic")
            .title("Здесь пока пусто")
            .description("Добавьте подписку или вставьте ссылку на сервер кнопкой «+» в шапке.")
            .vexpand(true)
            .build();

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        widget.append(&toolbar);
        widget.append(&group_info);
        widget.append(&scroller);
        widget.append(&empty);

        let page = Rc::new(Self {
            widget,
            state,
            list,
            groups_drop,
            groups_model,
            group_info,
            test_button,
            update_button,
            search,
            empty,
            scroller,
            rows: RefCell::new(HashMap::new()),
            groups: RefCell::new(Vec::new()),
            on_activated: RefCell::new(None),
            on_selection: RefCell::new(None),
            on_notify: RefCell::new(None),
            on_speed_test: RefCell::new(None),
            on_auto_pin: RefCell::new(None),
            syncing: std::cell::Cell::new(false),
        });
        page.wire();
        page
    }

    fn wire(self: &Rc<Self>) {
        self.list.connect_row_selected({
            let this = Rc::downgrade(self);
            move |_, row| {
                let Some(this) = this.upgrade() else { return };
                let Some(row) = row else { return };
                let id = unsafe { row.data::<i64>("profile-id") };
                if let Some(id) = id {
                    this.state.selected_id.set(unsafe { *id.as_ref() });
                    let _ = this.state.save_settings();
                    // Колбэк вызываем вне borrow: он трогает те же виджеты.
                    let callback = this.on_selection.borrow().clone();
                    if let Some(callback) = callback {
                        callback();
                    }
                }
            }
        });

        // Двойной клик и Enter подключают — выбор мышью сам по себе ничего
        // не разрывает.
        self.list.connect_row_activated({
            let this = Rc::downgrade(self);
            move |_, _| {
                let Some(this) = this.upgrade() else { return };
                let callback = this.on_activated.borrow().clone();
                if let Some(callback) = callback {
                    callback();
                }
            }
        });

        self.search.connect_search_changed({
            let this = Rc::downgrade(self);
            move |_| {
                let Some(this) = this.upgrade() else { return };
                this.apply_filter();
            }
        });

        // Правая кнопка открывает меню строки. Действия немногочисленны и
        // работают с той строкой, по которой щёлкнули, а не с выделенной.
        let gesture = gtk::GestureClick::builder()
            .button(gtk::gdk::BUTTON_SECONDARY)
            .build();
        gesture.connect_pressed({
            let this = Rc::downgrade(self);
            move |gesture, _, x, y| {
                let Some(this) = this.upgrade() else { return };
                let Some(row) = this.list.row_at_y(y as i32) else { return };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                this.list.select_row(Some(&row));
                this.show_row_menu(&row, x, y);
            }
        });
        self.list.add_controller(gesture);

        self.groups_drop.connect_selected_notify({
            let this = Rc::downgrade(self);
            move |drop| {
                let Some(this) = this.upgrade() else { return };
                if this.syncing.get() {
                    return;
                }
                let index = drop.selected() as usize;
                let gid = this.groups.borrow().get(index).map(|g| g.id);
                if let Some(gid) = gid {
                    if this.state.current_gid.get() != gid {
                        this.state.current_gid.set(gid);
                        this.reload();
                    }
                }
            }
        });
    }

    /// Меню строки: закрепить в автовыборе, замерить скорость, скопировать
    /// ссылку, удалить. Действия выполняются сразу — подтверждения нет.
    fn show_row_menu(self: &Rc<Self>, row: &gtk::ListBoxRow, x: f64, y: f64) {
        let id = unsafe { row.data::<i64>("profile-id") };
        let Some(id) = id else { return };
        let id = unsafe { *id.as_ref() };

        // Строка автовыбора сама по себе не сервер: замерять и копировать
        // у неё нечего, зато с неё снимают закрепление.
        let is_auto_row = id < 0;
        let auto_running = self.state.connected_id() < 0;

        let copy = gtk::Button::builder()
            .label("Скопировать ссылку")
            .css_classes(["flat"])
            .halign(gtk::Align::Fill)
            .visible(!is_auto_row)
            .build();
        if let Some(label) = copy.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
            label.set_xalign(0.0);
        }

        let pin = gtk::Button::builder()
            .label(if is_auto_row {
                "Снять закрепление"
            } else {
                "Закрепить в автовыборе"
            })
            .css_classes(["flat"])
            .halign(gtk::Align::Fill)
            .visible(auto_running)
            .build();
        if let Some(label) = pin.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
            label.set_xalign(0.0);
        }

        let measure = gtk::Button::builder()
            .label("Замерить скорость")
            .visible(!is_auto_row)
            .css_classes(["flat"])
            .halign(gtk::Align::Fill)
            .build();
        if let Some(label) = measure.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
            label.set_xalign(0.0);
        }
        let delete = gtk::Button::builder()
            .label("Удалить сервер")
            .css_classes(["flat"])
            .halign(gtk::Align::Fill)
            .visible(!is_auto_row)
            .build();
        if let Some(label) = delete.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
            label.set_xalign(0.0);
        }

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();
        content.append(&pin);
        content.append(&measure);
        content.append(&copy);
        content.append(&delete);

        let popover = gtk::Popover::builder()
            .child(&content)
            .has_arrow(false)
            .halign(gtk::Align::Start)
            .build();
        popover.set_parent(&self.list);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

        pin.connect_clicked({
            let this = Rc::downgrade(self);
            let popover = popover.clone();
            move |_| {
                popover.popdown();
                let Some(this) = this.upgrade() else { return };
                // Пустое имя означает «сними закрепление».
                let name = if is_auto_row {
                    String::new()
                } else {
                    this.state
                        .store
                        .borrow()
                        .profile(id)
                        .ok()
                        .flatten()
                        .map(|p| p.name)
                        .unwrap_or_default()
                };
                let callback = this.on_auto_pin.borrow().clone();
                if let Some(callback) = callback {
                    callback(name);
                }
            }
        });

        measure.connect_clicked({
            let this = Rc::downgrade(self);
            let popover = popover.clone();
            move |_| {
                popover.popdown();
                let Some(this) = this.upgrade() else { return };
                let callback = this.on_speed_test.borrow().clone();
                if let Some(callback) = callback {
                    callback(id);
                }
            }
        });

        copy.connect_clicked({
            let this = Rc::downgrade(self);
            let popover = popover.clone();
            move |button| {
                popover.popdown();
                let Some(this) = this.upgrade() else { return };
                let profile = this.state.store.borrow().profile(id).ok().flatten();
                let Some(profile) = profile else { return };
                match throne_config::share::to_link(&profile) {
                    Ok(link) => {
                        button.clipboard().set_text(&link);
                        this.notify("Ссылка скопирована");
                    }
                    Err(e) => this.notify(&format!("Ссылку не собрать: {e}")),
                }
            }
        });

        delete.connect_clicked({
            let this = Rc::downgrade(self);
            let popover = popover.clone();
            move |_| {
                popover.popdown();
                let Some(this) = this.upgrade() else { return };
                let name = this
                    .state
                    .store
                    .borrow()
                    .profile(id)
                    .ok()
                    .flatten()
                    .map(|p| p.name)
                    .unwrap_or_default();
                if this.state.store.borrow().delete_profile(id).is_err() {
                    this.notify("Не удалось удалить сервер");
                    return;
                }
                if this.state.selected_id.get() == id {
                    this.state.selected_id.set(0);
                }
                this.reload();
                this.notify(&format!("Сервер «{name}» удалён"));
            }
        });

        popover.connect_closed(|popover| popover.unparent());
        popover.popup();
    }

    /// Сообщения показывает окно; страница о нём не знает и просто зовёт
    /// колбэк, который окно ей выдало.
    fn notify(&self, message: &str) {
        if let Some(callback) = self.on_notify.borrow().clone() {
            callback(message.to_string());
        }
    }

    pub fn connect_notify_message(&self, callback: impl Fn(String) + 'static) {
        *self.on_notify.borrow_mut() = Some(Rc::new(callback));
    }

    pub fn connect_speed_test(&self, callback: impl Fn(i64) + 'static) {
        *self.on_speed_test.borrow_mut() = Some(Rc::new(callback));
    }

    pub fn connect_auto_pin(&self, callback: impl Fn(String) + 'static) {
        *self.on_auto_pin.borrow_mut() = Some(Rc::new(callback));
    }

    /// Ход замера показываем прямо в строке: тост исчезает, а замер идёт
    /// секунды, и человеку нужно видеть, что именно сейчас происходит.
    pub fn set_speed_progress(&self, id: i64, stage: &str) {
        let rows = self.rows.borrow();
        let Some(row) = rows.get(&id) else { return };
        row.speed.set_label(stage);
        row.speed.set_visible(true);
    }

    pub fn update_speed(&self, id: i64) {
        let profile = self.state.store.borrow().profile(id).ok().flatten();
        let Some(profile) = profile else { return };
        let rows = self.rows.borrow();
        let Some(row) = rows.get(&id) else { return };
        let text = speed_label(&profile);
        row.speed.set_visible(!text.is_empty());
        row.speed.set_label(&text);
    }

    pub fn connect_activated(&self, callback: impl Fn() + 'static) {
        *self.on_activated.borrow_mut() = Some(Rc::new(callback));
    }

    pub fn connect_selection_changed(&self, callback: impl Fn() + 'static) {
        *self.on_selection.borrow_mut() = Some(Rc::new(callback));
    }

    pub fn test_button(&self) -> &gtk::Button {
        &self.test_button
    }

    pub fn update_button(&self) -> &gtk::Button {
        &self.update_button
    }

    /// Полная пересборка: после смены группы, импорта или обновления подписки.
    pub fn reload(self: &Rc<Self>) {
        self.reload_groups();

        let gid = self.state.current_gid.get();
        let profiles = self
            .state
            .store
            .borrow()
            .profiles(Some(gid))
            .unwrap_or_default();

        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.rows.borrow_mut().clear();

        let connected = self.state.connected_id();
        let selected = self.state.selected_id.get();
        let mut selected_row = None;

        // Первой строкой — автовыбор по всей группе. Он не сервер, а способ
        // подключения, поэтому стоит над списком, а не среди него.
        if profiles.len() > 1 {
            let auto_id = -gid;
            let row = build_auto_row(profiles.len(), connected == auto_id);
            unsafe { row.root.set_data("profile-id", auto_id) };
            self.list.append(&row.root);
            if selected == auto_id {
                selected_row = Some(row.root.clone());
            }
            self.rows.borrow_mut().insert(auto_id, row);
        }

        for profile in &profiles {
            let row = build_row(profile, profile.id == connected);
            self.list.append(&row.root);
            if profile.id == selected {
                selected_row = Some(row.root.clone());
            }
            self.rows.borrow_mut().insert(profile.id, row);
        }

        if let Some(row) = selected_row {
            self.list.select_row(Some(&row));
        }

        let has_servers = !profiles.is_empty();
        self.scroller.set_visible(has_servers);
        self.empty.set_visible(!has_servers);
        self.apply_filter();
        self.refresh_group_info();
    }

    fn reload_groups(&self) {
        let groups = self.state.store.borrow().groups().unwrap_or_default();
        let names: Vec<String> = groups
            .iter()
            .map(|g| {
                if g.is_subscription() {
                    format!("{} ⌁", g.name)
                } else {
                    g.name.clone()
                }
            })
            .collect();

        let current = self.state.current_gid.get();
        let index = groups.iter().position(|g| g.id == current).unwrap_or(0);
        *self.groups.borrow_mut() = groups;

        // Наполнение модели двигает выделение; пока это делаем мы, а не
        // человек, реакция на смену группы не нужна.
        self.syncing.set(true);
        let previous = self.groups_model.n_items();
        self.groups_model.splice(
            0,
            previous,
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        if index < names.len() {
            self.groups_drop.set_selected(index as u32);
        }
        self.syncing.set(false);

        if let Some(gid) = self.groups.borrow().get(index).map(|g| g.id) {
            self.state.current_gid.set(gid);
        }
    }

    fn refresh_group_info(&self) {
        let gid = self.state.current_gid.get();
        let group = self.state.store.borrow().group(gid).ok().flatten();
        let Some(group) = group else {
            self.group_info.set_label("");
            return;
        };

        self.update_button.set_sensitive(group.is_subscription());
        if !group.is_subscription() {
            self.group_info.set_label("Ручная группа");
            return;
        }
        let updated = format::since(group.sub_last_update);
        let text = if group.info.is_empty() {
            format!("Обновлено {updated}")
        } else {
            format!("{} · обновлено {updated}", group.info)
        };
        self.group_info.set_label(&text);
    }

    fn apply_filter(&self) {
        let needle = self.search.text().to_lowercase();
        let mut row = self.list.first_child();
        while let Some(current) = row {
            row = current.next_sibling();
            let Ok(list_row) = current.clone().downcast::<gtk::ListBoxRow>() else {
                continue;
            };
            if needle.is_empty() {
                list_row.set_visible(true);
                continue;
            }
            let haystack = unsafe { list_row.data::<String>("haystack") };
            let visible = haystack
                .map(|h| unsafe { h.as_ref().contains(&needle) })
                .unwrap_or(true);
            list_row.set_visible(visible);
        }
    }

    /// Точечное обновление одной строки по результату теста.
    pub fn update_latency(&self, id: i64, latency: i32) {
        let rows = self.rows.borrow();
        let Some(row) = rows.get(&id) else { return };
        let (text, class) = format::latency(latency);
        row.latency.set_label(&text);
        for old in ["fast", "medium", "slow"] {
            row.latency.remove_css_class(old);
        }
        if !class.is_empty() {
            row.latency.add_css_class(class);
        }
    }

    /// Переносит отметку активного сервера, не перестраивая список.
    pub fn refresh_active(&self, connected_id: i64) {
        for (id, row) in self.rows.borrow().iter() {
            if *id == connected_id {
                row.content.add_css_class("active");
            } else {
                row.content.remove_css_class("active");
            }
        }
    }

    /// Отмечает сервер, который автовыбор держит прямо сейчас. Засечку
    /// показываем на нём, а не на строке «Автовыбор»: человеку важно, через
    /// какой сервер он на самом деле выходит.
    pub fn mark_auto_member(&self, name: &str) {
        let profiles = self
            .state
            .store
            .borrow()
            .profiles(Some(self.state.current_gid.get()))
            .unwrap_or_default();
        let member_id = profiles
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.id)
            .unwrap_or(0);

        for (id, row) in self.rows.borrow().iter() {
            // Строка автовыбора остаётся отмеченной как активный способ
            // подключения, а участник получает свою засечку.
            let active = *id == member_id || (*id < 0 && self.state.connected_id() == *id);
            if active {
                row.content.add_css_class("active");
            } else {
                row.content.remove_css_class("active");
            }
        }
    }

    pub fn set_testing(&self, testing: bool) {
        self.test_button.set_icon_name(if testing {
            "process-stop-symbolic"
        } else {
            "network-cellular-signal-excellent-symbolic"
        });
        self.test_button.set_tooltip_text(Some(if testing {
            "Остановить проверку"
        } else {
            "Проверить задержки всех серверов группы"
        }));
    }

    pub fn set_updating(&self, updating: bool) {
        self.update_button.set_sensitive(!updating);
    }
}

fn build_row(profile: &throne_config::Profile, active: bool) -> Row {
    let name = gtk::Label::builder()
        .label(&profile.name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["name"])
        .build();

    let kind = gtk::Label::builder()
        .label(protocol_label(&profile.kind))
        .css_classes(["badge"])
        .valign(gtk::Align::Center)
        .build();

    let address = gtk::Label::builder()
        .label(&profile.address())
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .css_classes(["meta", "numeric"])
        .build();

    let speed = gtk::Label::builder()
        .label(&speed_label(profile))
        .xalign(0.0)
        .visible(!speed_label(profile).is_empty())
        .css_classes(["meta", "numeric", "speed"])
        .build();

    let meta = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    meta.append(&kind);
    meta.append(&address);
    meta.append(&speed);

    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    texts.append(&name);
    texts.append(&meta);

    let (text, class) = format::latency(profile.latency);
    let latency = gtk::Label::builder()
        .label(&text)
        .xalign(1.0)
        .valign(gtk::Align::Center)
        .css_classes(["latency", "numeric"])
        .build();
    if !class.is_empty() {
        latency.add_css_class(class);
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .css_classes(["server-row"])
        .build();
    content.append(&texts);
    content.append(&latency);

    let root = gtk::ListBoxRow::builder().child(&content).build();
    if active {
        content.add_css_class("active");
    }
    unsafe {
        root.set_data("profile-id", profile.id);
        root.set_data(
            "haystack",
            format!("{} {}", profile.name, profile.address()).to_lowercase(),
        );
    }

    Row {
        root,
        content,
        latency,
        speed,
    }
}

/// Строка автовыбора: под ней ядро само держит лучший сервер группы.
fn build_auto_row(count: usize, active: bool) -> Row {
    let name = gtk::Label::builder()
        .label("Автовыбор")
        .xalign(0.0)
        .css_classes(["name"])
        .build();

    let badge = gtk::Label::builder()
        .label("АВТО")
        .css_classes(["badge"])
        .valign(gtk::Align::Center)
        .build();

    let description = gtk::Label::builder()
        .label(&format!("лучший из {count} по задержке"))
        .xalign(0.0)
        .css_classes(["meta"])
        .build();

    let meta = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    meta.append(&badge);
    meta.append(&description);

    let speed = gtk::Label::builder()
        .xalign(0.0)
        .visible(false)
        .css_classes(["meta", "numeric", "speed"])
        .build();
    meta.append(&speed);

    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    texts.append(&name);
    texts.append(&meta);

    let latency = gtk::Label::builder()
        .label("")
        .xalign(1.0)
        .valign(gtk::Align::Center)
        .css_classes(["latency", "numeric"])
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .css_classes(["server-row"])
        .build();
    content.append(&texts);
    content.append(&latency);
    if active {
        content.add_css_class("active");
    }

    let root = gtk::ListBoxRow::builder().child(&content).build();
    unsafe {
        root.set_data("profile-id", -1_i64);
        root.set_data("haystack", "автовыбор auto".to_string());
    }

    Row {
        root,
        content,
        latency,
        speed,
    }
}

/// Что показать о последнем замере скорости: приём, отдача и страна выхода.
fn speed_label(profile: &throne_config::Profile) -> String {
    let mut parts = Vec::new();
    if !profile.dl_speed.is_empty() {
        parts.push(format!("↓ {}", profile.dl_speed));
    }
    if !profile.ul_speed.is_empty() {
        parts.push(format!("↑ {}", profile.ul_speed));
    }
    if !profile.test_country.is_empty() {
        parts.push(profile.test_country.clone());
    }
    parts.join("  ")
}

/// Короткая метка протокола: в строке важна не точная реализация, а то, чем
/// сервер отличается от соседа.
fn protocol_label(kind: &str) -> &str {
    match kind {
        "vless" => "VLESS",
        "xrayvless" => "XHTTP",
        "vmess" => "VMESS",
        "trojan" => "TROJAN",
        "shadowsocks" => "SS",
        "hysteria2" => "HY2",
        "tuic" => "TUIC",
        "wireguard" => "WG",
        "anytls" => "ANYTLS",
        "naive" => "NAIVE",
        "socks" => "SOCKS",
        "http" => "HTTP",
        other => other,
    }
}
