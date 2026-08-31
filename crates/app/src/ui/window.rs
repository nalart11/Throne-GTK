//! Main window.
//!
//! At the top is the “channel”: state, server name, button, and live traffic line.
//! Below are three pages: servers, connections, and log. Everything that changes
//! over time is updated in one place — [`Window::apply_event`].

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::engine::{Command, Event, Status};
use crate::format;
use crate::state::State;
use crate::ui::sparkline::Sparkline;
use crate::ui::{connections, dialogs, logs, prefs, servers};

pub struct Window {
    pub root: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    state: Rc<State>,

    state_label: gtk::Label,
    name_label: gtk::Label,
    connect_button: gtk::Button,
    down_label: gtk::Label,
    up_label: gtk::Label,
    sparkline: Sparkline,
    mode_proxy: gtk::ToggleButton,
    mode_vpn: gtk::ToggleButton,

    servers: Rc<servers::ServersPage>,
    connections: Rc<connections::ConnectionsPage>,
    logs: Rc<logs::LogsPage>,

    /// Whether a latency test is currently running; this determines the test button label.
    testing: RefCell<bool>,
    /// Brings the interface into line with the settings: widget signals during
    /// this time do not represent user choices and must not save anything.
    syncing: std::cell::Cell<bool>,
}

impl Window {
    pub fn new(app: &adw::Application, state: Rc<State>) -> Rc<Self> {
        let root = adw::ApplicationWindow::builder()
            .application(app)
            .title("Throne GTK")
            .default_width(880)
            .default_height(660)
            .width_request(420)
            .height_request(480)
            .build();

        let toasts = adw::ToastOverlay::new();
        let stack = adw::ViewStack::new();

        let servers = servers::ServersPage::new(state.clone());
        let connections = connections::ConnectionsPage::new();
        let logs = logs::LogsPage::new(state.clone());

        stack
            .add_titled(&servers.widget, Some("servers"), "Серверы")
            .set_icon_name(Some("network-server-symbolic"));
        stack
            .add_titled(&connections.widget, Some("connections"), "Соединения")
            .set_icon_name(Some("network-transmit-receive-symbolic"));
        stack
            .add_titled(&logs.widget, Some("logs"), "Журнал")
            .set_icon_name(Some("text-x-generic-symbolic"));

        // ── header ───────────────────────────────────────────────────────
        let header = adw::HeaderBar::builder()
            .title_widget(
                &adw::ViewSwitcher::builder()
                    .stack(&stack)
                    .policy(adw::ViewSwitcherPolicy::Wide)
                    .build(),
            )
            .build();

        let add_button = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Добавить сервер или подписку")
            .build();
        header.pack_start(&add_button);

        let menu = gio_menu();
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Меню")
            .menu_model(&menu)
            .build();
        header.pack_end(&menu_button);

        // ── status panel ────────────────────────────────────────────
        let state_label = gtk::Label::builder()
            .label("НЕ ПОДКЛЮЧЕНО")
            .xalign(0.0)
            .css_classes(["channel-state"])
            .build();
        let name_label = gtk::Label::builder()
            .label("Сервер не выбран")
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["channel-name", "empty"])
            .build();

        let titles = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        titles.append(&state_label);
        titles.append(&name_label);

        let connect_button = gtk::Button::builder()
            .label("Подключить")
            .valign(gtk::Align::Center)
            .css_classes(["connect"])
            .build();

        let top = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .build();
        top.append(&titles);
        top.append(&connect_button);

        let sparkline = Sparkline::new();

        let down_label = gtk::Label::builder()
            .label("↓ 0 Б/с")
            .css_classes(["channel-rate", "numeric"])
            .build();
        let up_label = gtk::Label::builder()
            .label("↑ 0 Б/с")
            .css_classes(["channel-rate", "numeric"])
            .build();

        let (mode_box, mode_proxy, mode_vpn) = mode_switch(state.settings.borrow().mode.is_vpn());

        let rates = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(16)
            .build();
        rates.append(&down_label);
        rates.append(&up_label);
        let spacer = gtk::Box::builder().hexpand(true).build();
        rates.append(&spacer);
        rates.append(&mode_box);

        let channel = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["channel"])
            .build();
        channel.append(&top);
        channel.append(&sparkline.widget);
        channel.append(&rates);

        // On a wide monitor the list row would otherwise stretch across the full
        // width, forcing the eye to travel from the server name to its latency.
        let clamp = adw::Clamp::builder()
            .maximum_size(920)
            .tightening_threshold(720)
            .child(&channel)
            .build();

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        content.append(&clamp);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(
            &adw::Clamp::builder()
                .maximum_size(920)
                .tightening_threshold(720)
                .child(&stack)
                .vexpand(true)
                .build(),
        );
        stack.set_vexpand(true);

        let toolbar = adw::ToolbarView::builder().content(&content).build();
        toolbar.add_top_bar(&header);
        // In a narrow window the switcher moves down so page labels remain
        // readable and the header does not overflow.
        let bottom_switcher = adw::ViewSwitcherBar::builder().stack(&stack).build();
        toolbar.add_bottom_bar(&bottom_switcher);

        let breakpoint =
            adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 560px").unwrap());
        // In a narrow window the page switcher moves down; hide the header title
        // so two rows of labels are not stacked on top of each other.
        breakpoint.add_setter(&bottom_switcher, "reveal", Some(&true.to_value()));
        // Do not touch the header title here: replacing title-widget through the
        // breakpoint setter triggers a GLib warning on a temporary object.
        root.add_breakpoint(breakpoint);

        toasts.set_child(Some(&toolbar));
        root.set_content(Some(&toasts));

        let window = Rc::new(Self {
            root,
            toasts,
            state,
            state_label,
            name_label,
            connect_button,
            down_label,
            up_label,
            sparkline,
            mode_proxy,
            mode_vpn,
            servers,
            connections,
            logs,
            testing: RefCell::new(false),
            syncing: std::cell::Cell::new(false),
        });

        window.wire(&add_button);
        window.refresh_all();
        window
    }

    fn wire(self: &Rc<Self>, add_button: &gtk::Button) {
        // Connecting and disconnecting use one button: the label makes the state obvious.
        self.connect_button.connect_clicked({
            let this = Rc::downgrade(self);
            move |_| {
                let Some(this) = this.upgrade() else { return };
                if this.state.is_connected() {
                    this.state.disconnect();
                } else if this.state.selected_profile().is_some()
                    || this.state.auto_selected().is_some()
                {
                    this.state.connect_selected();
                } else {
                    this.toast("Сначала выберите сервер в списке");
                }
            }
        });

        add_button.connect_clicked({
            let this = Rc::downgrade(self);
            move |_| {
                let Some(this) = this.upgrade() else { return };
                dialogs::add_dialog(&this);
            }
        });

        for (button, vpn) in [(&self.mode_proxy, false), (&self.mode_vpn, true)] {
            button.connect_toggled({
                let this = Rc::downgrade(self);
                move |button| {
                    if !button.is_active() {
                        return;
                    }
                    let Some(this) = this.upgrade() else { return };
                    if this.syncing.get() {
                        return;
                    }
                    {
                        let mut settings = this.state.settings.borrow_mut();
                        let next = if vpn {
                            throne_config::settings::ProxyMode::Vpn
                        } else {
                            throne_config::settings::ProxyMode::Proxy
                        };
                        if settings.mode == next {
                            return;
                        }
                        settings.mode = next;
                    }
                    let _ = this.state.save_settings();
                    // The mode changes config inbounds; on a live connection it
                    // must be rebuilt, otherwise the switch lies.
                    if this.state.is_connected() {
                        this.state.connect_selected();
                        this.toast(if vpn {
                            "Переключаю на VPN — весь трафик системы"
                        } else {
                            "Переключаю на прокси — только приложения с настройкой"
                        });
                    }
                }
            });
        }

        // The server list decides when to connect and update the header.
        self.servers.connect_activated({
            let this = Rc::downgrade(self);
            move || {
                let Some(this) = this.upgrade() else { return };
                this.state.connect_selected();
            }
        });
        self.servers.connect_auto_pin({
            let this = Rc::downgrade(self);
            move |name| {
                let Some(this) = this.upgrade() else { return };
                this.state.engine.send(Command::AutoPin(name.clone()));
                this.toast(&if name.is_empty() {
                    "Закрепление снято — автовыбор снова решает сам".to_string()
                } else {
                    format!("Автовыбор закреплён на «{name}»")
                });
            }
        });
        self.servers.connect_speed_test({
            let this = Rc::downgrade(self);
            move |id| {
                let Some(this) = this.upgrade() else { return };
                this.start_speed_test(id);
            }
        });
        self.servers.connect_notify_message({
            let this = Rc::downgrade(self);
            move |message| {
                let Some(this) = this.upgrade() else { return };
                this.toast(&message);
            }
        });
        self.servers.connect_selection_changed({
            let this = Rc::downgrade(self);
            move || {
                let Some(this) = this.upgrade() else { return };
                this.refresh_channel();
            }
        });
    }

    /// The single point where updates from the background thread arrive.
    pub fn apply_event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Status(status) => {
                let failed = matches!(&status, Status::Failed(_));
                let message = match &status {
                    Status::Failed(e) => Some(e.clone()),
                    _ => None,
                };
                if matches!(status, Status::Disconnected | Status::Failed(_)) {
                    self.sparkline.clear();
                    self.down_label.set_label("↓ 0 Б/с");
                    self.up_label.set_label("↑ 0 Б/с");
                    self.connections.clear();
                }
                *self.state.status.borrow_mut() = status;
                self.refresh_channel();
                self.servers.refresh_active(self.state.connected_id());
                if failed {
                    if let Some(message) = message {
                        self.state.push_log(format!("ошибка: {message}"));
                        self.logs.refresh();
                        self.toast(&first_line(&message));
                    }
                }
            }
            Event::Traffic { down, up } => {
                self.sparkline.push(down, up);
                self.down_label
                    .set_label(&format!("↓ {}", format::rate(down)));
                self.up_label.set_label(&format!("↑ {}", format::rate(up)));
                let id = self.state.connected_id();
                if id != 0 {
                    let _ = self.state.store.borrow().add_traffic(id, down, up);
                }
            }
            Event::Connections(list) => self.connections.set(list),
            Event::AutoStatus {
                selected,
                pinned,
                alive,
                total,
                suspended,
            } => {
                // Auto-selection changes the server itself; the header must show
                // who is selected now, otherwise “Connected” says nothing.
                let name = if selected.is_empty() {
                    "выбирает сервер…".to_string()
                } else if !pinned.is_empty() && pinned == selected {
                    format!("{selected} · закреплён")
                } else {
                    selected.clone()
                };
                self.name_label.set_label(&format!("Авто · {name}"));
                self.name_label.remove_css_class("empty");
                self.state_label.set_label(&if suspended {
                    // The core distinguishes “network unavailable” from “servers
                    // are dead”; in the latter case retrying servers is pointless.
                    "СЕТЬ НЕДОСТУПНА".to_string()
                } else {
                    format!("ПОДКЛЮЧЕНО · ЖИВЫХ {alive} ИЗ {total}")
                });
                self.servers.mark_auto_member(&selected);
            }
            Event::LatencyResult { id, latency } => {
                let now = unix_now();
                let _ = self.state.store.borrow().set_latency(id, latency, now);
                self.servers.update_latency(id, latency);
            }
            Event::TestFinished { tested } => {
                *self.testing.borrow_mut() = false;
                self.servers.set_testing(false);
                self.toast(&match tested {
                    0 => "Проверка не дала результатов".to_string(),
                    n => format!("Проверено серверов: {n}"),
                });
            }
            Event::SpeedProgress { id, stage } => self.servers.set_speed_progress(id, &stage),
            Event::SpeedResult {
                id,
                download,
                upload,
                country,
                latency,
                error,
            } => {
                if error.is_empty() {
                    let _ = self
                        .state
                        .store
                        .borrow()
                        .set_speed(id, &download, &upload, &country);
                    if latency > 0 {
                        let _ = self
                            .state
                            .store
                            .borrow()
                            .set_latency(id, latency, unix_now());
                        self.servers.update_latency(id, latency);
                    }
                    self.servers.update_speed(id);
                    self.toast(&match (download.is_empty(), upload.is_empty()) {
                        (false, false) => format!("Замер: ↓ {download}, ↑ {upload}"),
                        (false, true) => format!("Замер: ↓ {download}"),
                        (true, false) => format!("Замер: ↑ {upload}"),
                        (true, true) => "Замер не дал результата".to_string(),
                    });
                } else {
                    // The row returns to its previous value: displayed progress
                    // must not remain in place of the result.
                    self.servers.update_speed(id);
                    self.toast(&first_line(&format!("Замер не удался: {error}")));
                }
            }
            Event::Subscription { gid, result } => self.apply_subscription(gid, result),
            Event::ConfigChecked(result) => match result {
                Ok(()) => self.toast("Конфигурация принята ядром"),
                Err(e) => self.toast(&first_line(&e)),
            },
            Event::Log(line) => {
                self.state.push_log(line);
                self.logs.refresh();
            }
            Event::Error(message) => {
                self.state.push_log(format!("ошибка: {message}"));
                self.logs.refresh();
                self.toast(&first_line(&message));
            }
        }
    }

    fn apply_subscription(
        self: &Rc<Self>,
        gid: i64,
        result: Result<throne_config::subscription::Parsed, String>,
    ) {
        self.servers.set_updating(false);
        let parsed = match result {
            Ok(parsed) => parsed,
            Err(e) => {
                self.state.push_log(format!("подписка: {e}"));
                self.logs.refresh();
                self.toast(&first_line(&e));
                return;
            }
        };

        let outcome = {
            let mut store = self.state.store.borrow_mut();
            store.replace_group_profiles(gid, &parsed.profiles)
        };
        match outcome {
            Ok((added, kept, removed)) => {
                {
                    let store = self.state.store.borrow();
                    if let Ok(Some(mut group)) = store.group(gid) {
                        group.info = parsed.info.clone();
                        group.sub_last_update = unix_now();
                        let _ = store.update_group(&group);
                    }
                }
                self.servers.reload();
                self.refresh_channel();
                self.toast(&format!(
                    "Обновлено: {added} новых, {kept} прежних, {removed} удалено"
                ));
                for error in parsed.errors.iter().take(5) {
                    self.state.push_log(format!("подписка: {error}"));
                }
                self.logs.refresh();
            }
            Err(e) => self.toast(&first_line(&format!("{e:#}"))),
        }
    }

    pub fn refresh_all(self: &Rc<Self>) {
        self.refresh_channel();
        self.servers.reload();
        self.logs.refresh();

        let vpn = self.state.settings.borrow().mode.is_vpn();
        self.syncing.set(true);
        if vpn {
            self.mode_vpn.set_active(true);
        } else {
            self.mode_proxy.set_active(true);
        }
        self.syncing.set(false);
    }

    /// Brings the header in line with the state: label, server name, and button.
    fn refresh_channel(self: &Rc<Self>) {
        let status = self.state.status.borrow().clone();
        let (label, class) = match &status {
            Status::Disconnected => ("НЕ ПОДКЛЮЧЕНО", ""),
            Status::Connecting => ("ПОДКЛЮЧАЮСЬ", "working"),
            Status::Connected { .. } => ("ПОДКЛЮЧЕНО", "live"),
            Status::Failed(_) => ("НЕ УДАЛОСЬ", "failed"),
        };
        self.state_label.set_label(label);
        set_state_class(&self.state_label, class);

        let name = match &status {
            Status::Connected { name, .. } => name.clone(),
            _ => match self.state.auto_selected() {
                Some(_) => "Автовыбор".to_string(),
                None => self
                    .state
                    .selected_profile()
                    .map(|p| p.name)
                    .unwrap_or_default(),
            },
        };
        if name.is_empty() {
            self.name_label.set_label("Сервер не выбран");
            self.name_label.add_css_class("empty");
        } else {
            self.name_label.set_label(&name);
            self.name_label.remove_css_class("empty");
        }

        let connected = matches!(status, Status::Connected { .. });
        self.connect_button.set_label(if connected {
            "Отключить"
        } else {
            "Подключить"
        });
        if connected {
            self.connect_button.add_css_class("live");
        } else {
            self.connect_button.remove_css_class("live");
        }
        self.connect_button
            .set_sensitive(!matches!(status, Status::Connecting));
    }

    pub fn toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::new(message));
    }

    pub fn state(&self) -> &Rc<State> {
        &self.state
    }

    pub fn servers_page(&self) -> &Rc<servers::ServersPage> {
        &self.servers
    }

    /// Latency testing does not interrupt the active connection; the test starts
    /// a separate core.
    pub fn start_test(self: &Rc<Self>) {
        if *self.testing.borrow() {
            self.state.engine.send(Command::StopTest);
            return;
        }
        let gid = self.state.current_gid.get();
        let profiles = match self.state.store.borrow().profiles(Some(gid)) {
            Ok(p) => p,
            Err(e) => {
                self.toast(&format!("Не удалось прочитать список: {e}"));
                return;
            }
        };
        if profiles.is_empty() {
            self.toast("В этой группе нет серверов");
            return;
        }
        let ids = profiles.iter().map(|p| p.id).collect();
        *self.testing.borrow_mut() = true;
        self.servers.set_testing(true);
        self.state.engine.send(Command::TestLatency {
            profiles,
            ids,
            settings: Box::new(self.state.settings.borrow().clone()),
        });
    }

    /// Asks auto-selection to recalculate its ranking right now.
    pub fn recheck_auto(self: &Rc<Self>) {
        if self.state.connected_id() >= 0 {
            self.toast("Автовыбор сейчас не используется");
            return;
        }
        self.state.engine.send(Command::AutoRecheck);
        self.toast("Перепроверяю серверы группы…");
    }

    /// Measures the selected server’s speed.
    pub fn start_speed_test(self: &Rc<Self>, id: i64) {
        let profile = self.state.store.borrow().profile(id).ok().flatten();
        let Some(profile) = profile else {
            self.toast("Сервер не найден");
            return;
        };
        let settings = self.state.settings.borrow().clone();
        if !settings.speed_test_download && !settings.speed_test_upload {
            self.toast("В настройках выключены и приём, и отдача — мерить нечего");
            return;
        }
        self.servers.set_speed_progress(id, "замеряю…");
        self.state.engine.send(Command::SpeedTest {
            profile: Box::new(profile),
            id,
            settings: Box::new(settings),
        });
    }

    pub fn open_preferences(self: &Rc<Self>) {
        prefs::open(self);
    }
}

fn set_state_class(label: &gtk::Label, class: &str) {
    for old in ["live", "working", "failed"] {
        label.remove_css_class(old);
    }
    if !class.is_empty() {
        label.add_css_class(class);
    }
}

/// Mode switch: two linked toggles instead of a drop-down list; there are exactly
/// two modes, and both must be visible without a click.
///
/// The initial state is set here before handlers are connected: GTK activates the
/// first button in the group, and a connected handler would mistake that for a
/// user choice, silently switching the application to VPN on every launch.
fn mode_switch(vpn: bool) -> (gtk::Box, gtk::ToggleButton, gtk::ToggleButton) {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["linked"])
        .valign(gtk::Align::Center)
        .build();

    let proxy = gtk::ToggleButton::builder()
        .label("Прокси")
        .tooltip_text("Только приложения, настроенные на локальный порт")
        .build();
    let vpn_button = gtk::ToggleButton::builder()
        .label("VPN")
        .tooltip_text("Весь трафик системы через TUN-интерфейс")
        .group(&proxy)
        .build();

    container.append(&proxy);
    container.append(&vpn_button);
    if vpn {
        vpn_button.set_active(true);
    } else {
        proxy.set_active(true);
    }
    (container, proxy, vpn_button)
}

fn gio_menu() -> gtk::gio::Menu {
    let menu = gtk::gio::Menu::new();

    let section = gtk::gio::Menu::new();
    section.append(Some("Добавить серверы"), Some("app.add"));
    section.append(Some("Проверить задержки"), Some("app.test"));
    section.append(Some("Замерить скорость"), Some("app.speed-test"));
    section.append(Some("Перепроверить автовыбор"), Some("app.auto-recheck"));
    section.append(Some("Обновить подписку"), Some("app.update-subscription"));
    menu.append_section(None, &section);

    let section = gtk::gio::Menu::new();
    section.append(Some("Правила маршрутизации"), Some("app.routes"));
    section.append(Some("Импорт из Throne"), Some("app.import"));
    section.append(Some("Проверить конфигурацию"), Some("app.check-config"));
    menu.append_section(None, &section);

    let section = gtk::gio::Menu::new();
    section.append(Some("Настройки"), Some("app.preferences"));
    section.append(Some("О программе"), Some("app.about"));
    menu.append_section(None, &section);

    menu
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Core errors can span multiple lines; only one fits in the toast message.
fn first_line(message: &str) -> String {
    message.lines().next().unwrap_or(message).to_string()
}
