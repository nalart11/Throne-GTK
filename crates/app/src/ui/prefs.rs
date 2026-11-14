//! Settings.
//!
//! Each switch writes its value immediately; there is no separate “Apply” button.
//! Changes affecting a live connection apply on the next connection; this is
//! stated in the subtitle rather than discovered through experience.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::ui::Window;
use nolurix_config::settings::{DomainStrategy, Language, TunStack};

pub fn open(window: &Rc<Window>) {
    let dialog = adw::PreferencesDialog::builder()
        .title(crate::i18n::tr("Настройки", "Settings"))
        .content_width(560)
        .content_height(640)
        .build();

    dialog.add(&connection_page(window));
    dialog.add(&network_page(window));
    dialog.add(&core_page(window));
    dialog.present(Some(&window.root));
}

/// Small helper: apply a settings change and save it.
fn edit(window: &Rc<Window>, apply: impl FnOnce(&mut nolurix_config::Settings)) {
    {
        let mut settings = window.state().settings.borrow_mut();
        apply(&mut settings);
    }
    if let Err(e) = window.state().save_settings() {
        window.toast(&crate::tr_format!(
            "Не удалось сохранить настройки: {e}",
            "Could not save settings: {e}"
        ));
    }
}

fn connection_page(window: &Rc<Window>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(crate::i18n::tr("Соединение", "Connection"))
        .icon_name("network-workgroup-symbolic")
        .build();

    let settings = window.state().settings.borrow().clone();

    let group = adw::PreferencesGroup::builder()
        .title(crate::i18n::tr("Локальный прокси", "Local proxy"))
        .description(crate::i18n::tr(
            "Порт, на который настраиваются браузер и другие программы",
            "Port used by the browser and other applications",
        ))
        .build();

    let port = adw::SpinRow::builder()
        .title(crate::i18n::tr("Порт", "Port"))
        .subtitle(crate::i18n::tr(
            "SOCKS5 и HTTP на одном порту",
            "SOCKS5 and HTTP on the same port",
        ))
        .adjustment(&gtk::Adjustment::new(
            settings.mixed_port as f64,
            1024.0,
            65535.0,
            1.0,
            10.0,
            0.0,
        ))
        .build();
    port.connect_value_notify({
        let window = window.clone();
        move |row| {
            let value = row.value() as u16;
            edit(&window, |s| s.mixed_port = value);
        }
    });
    group.add(&port);

    let lan = adw::SwitchRow::builder()
        .title(crate::i18n::tr(
            "Открыть доступ из локальной сети",
            "Allow LAN access",
        ))
        .subtitle(crate::i18n::tr(
            "Прокси примет подключения с других устройств в сети",
            "Accept proxy connections from other devices on the network",
        ))
        .active(settings.allow_lan)
        .build();
    lan.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.allow_lan = value);
        }
    });
    group.add(&lan);

    let sniffing = adw::SwitchRow::builder()
        .title(crate::i18n::tr(
            "Определять протокол и домен",
            "Detect protocol and domain",
        ))
        .subtitle(crate::i18n::tr(
            "Нужно для правил по доменам и понятных имён в соединениях",
            "Required for domain rules and readable names in connections",
        ))
        .active(settings.sniffing)
        .build();
    sniffing.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.sniffing = value);
        }
    });
    group.add(&sniffing);
    page.add(&group);

    let group = adw::PreferencesGroup::builder()
        .title(crate::i18n::tr("Режим VPN", "VPN mode"))
        .description(crate::i18n::tr(
            "Применяется при следующем подключении",
            "Applies on the next connection",
        ))
        .build();

    let stacks = [
        crate::i18n::tr("Смешанный", "Mixed"),
        crate::i18n::tr("Системный", "System"),
        "gVisor",
    ];
    let stack_values = [TunStack::Mixed, TunStack::System, TunStack::GVisor];
    let stack = adw::ComboRow::builder()
        .title(crate::i18n::tr("Сетевой стек", "Network stack"))
        .subtitle(crate::i18n::tr(
            "Смешанный подходит в большинстве случаев",
            "Mixed works best in most cases",
        ))
        .model(&gtk::StringList::new(&stacks))
        .selected(
            stack_values
                .iter()
                .position(|s| *s == settings.tun_stack)
                .unwrap_or(0) as u32,
        )
        .build();
    stack.connect_selected_notify({
        let window = window.clone();
        move |row| {
            let value = stack_values[row.selected() as usize];
            edit(&window, |s| s.tun_stack = value);
        }
    });
    group.add(&stack);

    let mtu = adw::SpinRow::builder()
        .title("MTU")
        .adjustment(&gtk::Adjustment::new(
            settings.tun_mtu as f64,
            576.0,
            65535.0,
            1.0,
            100.0,
            0.0,
        ))
        .build();
    mtu.connect_value_notify({
        let window = window.clone();
        move |row| {
            let value = row.value() as u32;
            edit(&window, |s| s.tun_mtu = value);
        }
    });
    group.add(&mtu);

    let strict = adw::SwitchRow::builder()
        .title(crate::i18n::tr("Строгая маршрутизация", "Strict routing"))
        .subtitle(crate::i18n::tr(
            "Закрывает обход туннеля мимо правил",
            "Prevent traffic from bypassing tunnel rules",
        ))
        .active(settings.tun_strict_route)
        .build();
    strict.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.tun_strict_route = value);
        }
    });
    group.add(&strict);

    let ipv6 = adw::SwitchRow::builder()
        .title(crate::i18n::tr("IPv6 в туннеле", "IPv6 in tunnel"))
        .active(settings.tun_ipv6)
        .build();
    ipv6.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.tun_ipv6 = value);
        }
    });
    group.add(&ipv6);
    page.add(&group);

    let group = adw::PreferencesGroup::builder()
        .title(crate::i18n::tr("Окно", "Window"))
        .description(crate::i18n::tr(
            "Значок в трее рисует панель рабочего стола; без неё окно закрывается как обычно",
            "The desktop panel provides the tray icon; without it, the window closes normally.",
        ))
        .build();

    let languages = ["Русский", "English"];
    let language = adw::ComboRow::builder()
        .title(crate::i18n::tr("Язык", "Language"))
        .subtitle(crate::i18n::tr(
            "Изменение вступит в силу после перезапуска приложения",
            "The change takes effect after you restart the app.",
        ))
        .model(&gtk::StringList::new(&languages))
        .selected(if settings.language == Language::Ru {
            0
        } else {
            1
        })
        .build();
    // Connect after selecting the saved value: initialization must never save.
    language.connect_selected_notify({
        let window = window.clone();
        move |row| {
            let value = if row.selected() == 0 {
                Language::Ru
            } else {
                Language::En
            };
            edit(&window, |s| s.language = value);
        }
    });
    group.add(&language);

    let to_tray = adw::SwitchRow::builder()
        .title(crate::i18n::tr(
            "Прятать в значок при закрытии",
            "Hide to tray on close",
        ))
        .subtitle(crate::i18n::tr(
            "Соединение продолжает работать, окно возвращается щелчком по значку",
            "The connection stays active; click the tray icon to restore the window",
        ))
        .active(settings.close_to_tray)
        .build();
    to_tray.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.close_to_tray = value);
        }
    });
    group.add(&to_tray);

    let minimized = adw::SwitchRow::builder()
        .title(crate::i18n::tr(
            "Запускать в значке",
            "Start minimized to tray",
        ))
        .subtitle(crate::i18n::tr(
            "При старте показывать только значок, без окна",
            "Show only the tray icon at startup, without the window",
        ))
        .active(settings.start_minimized)
        .build();
    minimized.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.start_minimized = value);
        }
    });
    group.add(&minimized);
    page.add(&group);

    page
}

fn network_page(window: &Rc<Window>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(crate::i18n::tr("Сеть", "Network"))
        .icon_name("network-server-symbolic")
        .build();

    let settings = window.state().settings.borrow().clone();

    let group = adw::PreferencesGroup::builder()
        .title("DNS")
        .description(crate::i18n::tr(
            "Схема в адресе задаёт транспорт: udp://, tls://, https://, quic://",
            "The address scheme selects the transport: udp://, tls://, https://, quic://",
        ))
        .build();

    let enabled = adw::SwitchRow::builder()
        .title(crate::i18n::tr("Своя служба DNS", "Built-in DNS service"))
        .subtitle(crate::i18n::tr(
            "Без неё запросы уходят системному резолверу",
            "When disabled, queries use the system resolver",
        ))
        .active(settings.dns_enabled)
        .build();
    enabled.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.dns_enabled = value);
        }
    });
    group.add(&enabled);

    let remote = adw::EntryRow::builder()
        .title(crate::i18n::tr("Через прокси", "Through proxy"))
        .text(&settings.dns_remote)
        .build();
    remote.connect_changed({
        let window = window.clone();
        move |row| {
            let value = row.text().to_string();
            edit(&window, |s| s.dns_remote = value);
        }
    });
    group.add(&remote);

    let direct = adw::EntryRow::builder()
        .title(crate::i18n::tr("Напрямую", "Direct"))
        .text(&settings.dns_direct)
        .build();
    direct.connect_changed({
        let window = window.clone();
        move |row| {
            let value = row.text().to_string();
            edit(&window, |s| s.dns_direct = value);
        }
    });
    group.add(&direct);

    let strategies = [
        crate::i18n::tr("Как есть", "As is"),
        crate::i18n::tr("Сначала IPv4", "Prefer IPv4"),
        crate::i18n::tr("Сначала IPv6", "Prefer IPv6"),
        crate::i18n::tr("Только IPv4", "IPv4 only"),
        crate::i18n::tr("Только IPv6", "IPv6 only"),
    ];
    let strategy_values = [
        DomainStrategy::AsIs,
        DomainStrategy::PreferIpv4,
        DomainStrategy::PreferIpv6,
        DomainStrategy::Ipv4Only,
        DomainStrategy::Ipv6Only,
    ];
    let strategy = adw::ComboRow::builder()
        .title(crate::i18n::tr("Разрешение имён", "Name resolution"))
        .model(&gtk::StringList::new(&strategies))
        .selected(
            strategy_values
                .iter()
                .position(|s| *s == settings.dns_strategy)
                .unwrap_or(1) as u32,
        )
        .build();
    strategy.connect_selected_notify({
        let window = window.clone();
        move |row| {
            let value = strategy_values[row.selected() as usize];
            edit(&window, |s| s.dns_strategy = value);
        }
    });
    group.add(&strategy);

    let fakedns = adw::SwitchRow::builder()
        .title(crate::i18n::tr("Подставные адреса", "Fake IP addresses"))
        .subtitle(crate::i18n::tr(
            "Ускоряет соединение, но ломает программы, которым нужен настоящий IP",
            "Speeds up connections but breaks applications that need a real IP",
        ))
        .active(settings.fakedns)
        .build();
    fakedns.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.fakedns = value);
        }
    });
    group.add(&fakedns);
    page.add(&group);

    let group = adw::PreferencesGroup::builder()
        .title(crate::i18n::tr("Маршрутизация", "Routing"))
        .build();

    let bypass = adw::SwitchRow::builder()
        .title(crate::i18n::tr(
            "Локальные адреса мимо прокси",
            "Bypass proxy for local addresses",
        ))
        .subtitle(crate::i18n::tr(
            "Домашняя сеть, .local и приватные диапазоны идут напрямую",
            "Home network, .local and private ranges connect directly",
        ))
        .active(settings.bypass_private)
        .build();
    bypass.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.bypass_private = value);
        }
    });
    group.add(&bypass);

    let rules = adw::ActionRow::builder()
        .title(crate::i18n::tr("Правила маршрутизации", "Routing rules"))
        .subtitle(rules_subtitle(&settings))
        .activatable(true)
        .build();
    rules.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    rules.connect_activated({
        let window = window.clone();
        move |row| {
            crate::ui::routes::open(&window);
            // The subtitle updates when the user returns from the editor.
            let window = window.clone();
            let row = row.clone();
            glib::idle_add_local_once(move || {
                row.set_subtitle(&rules_subtitle(&window.state().settings.borrow()));
            });
        }
    });
    group.add(&rules);
    page.add(&group);

    let group = adw::PreferencesGroup::builder()
        .title(crate::i18n::tr("Проверка серверов", "Server tests"))
        .build();

    let url = adw::EntryRow::builder()
        .title(crate::i18n::tr("Адрес для проверки", "Test URL"))
        .text(&settings.test_url)
        .build();
    url.connect_changed({
        let window = window.clone();
        move |row| {
            let value = row.text().to_string();
            edit(&window, |s| s.test_url = value);
        }
    });
    group.add(&url);

    let timeout = adw::SpinRow::builder()
        .title(crate::i18n::tr("Таймаут, мс", "Timeout, ms"))
        .adjustment(&gtk::Adjustment::new(
            settings.test_timeout_ms as f64,
            500.0,
            30000.0,
            100.0,
            1000.0,
            0.0,
        ))
        .build();
    timeout.connect_value_notify({
        let window = window.clone();
        move |row| {
            let value = row.value() as i32;
            edit(&window, |s| s.test_timeout_ms = value);
        }
    });
    group.add(&timeout);

    let concurrency = adw::SpinRow::builder()
        .title(crate::i18n::tr(
            "Одновременных проверок",
            "Concurrent tests",
        ))
        .adjustment(&gtk::Adjustment::new(
            settings.test_concurrency as f64,
            1.0,
            128.0,
            1.0,
            4.0,
            0.0,
        ))
        .build();
    concurrency.connect_value_notify({
        let window = window.clone();
        move |row| {
            let value = row.value() as i32;
            edit(&window, |s| s.test_concurrency = value);
        }
    });
    group.add(&concurrency);
    page.add(&group);

    let group = adw::PreferencesGroup::builder()
        .title(crate::i18n::tr(
            "Автовыбор сервера",
            "Automatic server selection",
        ))
        .description(crate::i18n::tr(
            "Ядро само держит лучший сервер группы и уходит с упавшего",
            "The core keeps the best server in the group and switches away from failed servers",
        ))
        .build();

    let interval = adw::SpinRow::builder()
        .title(crate::i18n::tr(
            "Перепроверка, секунд",
            "Retest interval, seconds",
        ))
        .subtitle(crate::i18n::tr(
            "Как часто пересматриваются лучшие серверы группы",
            "How often to reassess the best servers in the group",
        ))
        .adjustment(&gtk::Adjustment::new(
            settings.auto_interval_secs as f64,
            30.0,
            3600.0,
            10.0,
            60.0,
            0.0,
        ))
        .build();
    interval.connect_value_notify({
        let window = window.clone();
        move |row| {
            let value = row.value() as i64;
            edit(&window, |s| s.auto_interval_secs = value);
        }
    });
    group.add(&interval);

    let tolerance = adw::SpinRow::builder()
        .title(crate::i18n::tr(
            "Запас на переключение, мс",
            "Switching margin, ms",
        ))
        .subtitle(crate::i18n::tr(
            "Насколько новый сервер должен быть быстрее текущего",
            "How much faster a new server must be than the current one",
        ))
        .adjustment(&gtk::Adjustment::new(
            settings.auto_tolerance_ms as f64,
            0.0,
            2000.0,
            10.0,
            50.0,
            0.0,
        ))
        .build();
    tolerance.connect_value_notify({
        let window = window.clone();
        move |row| {
            let value = row.value() as i32;
            edit(&window, |s| s.auto_tolerance_ms = value);
        }
    });
    group.add(&tolerance);

    let balance = adw::SwitchRow::builder()
        .title(crate::i18n::tr(
            "Распределять по нескольким серверам",
            "Distribute across multiple servers",
        ))
        .subtitle(crate::i18n::tr(
            "Вместо одного лучшего используются все подходящие по очереди",
            "Use all suitable servers in turn instead of a single best server",
        ))
        .active(settings.auto_balance)
        .build();
    balance.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.auto_balance = value);
        }
    });
    group.add(&balance);
    page.add(&group);

    let group = adw::PreferencesGroup::builder()
        .title(crate::i18n::tr("Замер скорости", "Speed test"))
        .description(crate::i18n::tr(
            "Запускается для одного сервера из меню строки",
            "Run for a single server from its row menu",
        ))
        .build();

    let download = adw::SwitchRow::builder()
        .title(crate::i18n::tr("Мерить приём", "Measure download"))
        .active(settings.speed_test_download)
        .build();
    download.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.speed_test_download = value);
        }
    });
    group.add(&download);

    let upload = adw::SwitchRow::builder()
        .title(crate::i18n::tr("Мерить отдачу", "Measure upload"))
        .subtitle(crate::i18n::tr(
            "Удваивает время замера",
            "Doubles the test duration",
        ))
        .active(settings.speed_test_upload)
        .build();
    upload.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.speed_test_upload = value);
        }
    });
    group.add(&upload);

    let speed_timeout = adw::SpinRow::builder()
        .title(crate::i18n::tr(
            "Длительность замера, мс",
            "Test duration, ms",
        ))
        .adjustment(&gtk::Adjustment::new(
            settings.speed_test_timeout_ms as f64,
            2000.0,
            60000.0,
            500.0,
            5000.0,
            0.0,
        ))
        .build();
    speed_timeout.connect_value_notify({
        let window = window.clone();
        move |row| {
            let value = row.value() as i32;
            edit(&window, |s| s.speed_test_timeout_ms = value);
        }
    });
    group.add(&speed_timeout);
    page.add(&group);

    page
}

fn core_page(window: &Rc<Window>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(crate::i18n::tr("Ядро", "Core"))
        .icon_name("applications-engineering-symbolic")
        .build();

    let settings = window.state().settings.borrow().clone();

    let group = adw::PreferencesGroup::builder()
        .title(crate::i18n::tr("Соединения", "Connections"))
        .build();

    let mux = adw::SwitchRow::builder()
        .title(crate::i18n::tr("Мультиплексирование", "Multiplexing"))
        .subtitle(crate::i18n::tr(
            "Несколько запросов в одном соединении. Не применяется к XTLS и QUIC",
            "Multiple requests over one connection. Does not apply to XTLS or QUIC",
        ))
        .active(settings.mux_enabled)
        .build();
    mux.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.mux_enabled = value);
        }
    });
    group.add(&mux);

    let protocols = ["h2mux", "smux", "yamux"];
    let protocol = adw::ComboRow::builder()
        .title(crate::i18n::tr(
            "Протокол мультиплексирования",
            "Multiplexing protocol",
        ))
        .model(&gtk::StringList::new(&protocols))
        .selected(
            protocols
                .iter()
                .position(|p| *p == settings.mux_protocol)
                .unwrap_or(0) as u32,
        )
        .build();
    protocol.connect_selected_notify({
        let window = window.clone();
        move |row| {
            let value = protocols[row.selected() as usize].to_string();
            edit(&window, |s| s.mux_protocol = value);
        }
    });
    group.add(&protocol);

    let stats = adw::SwitchRow::builder()
        .title(crate::i18n::tr(
            "Считать трафик и соединения",
            "Track traffic and connections",
        ))
        .subtitle(crate::i18n::tr(
            "Нужно для графика и страницы соединений",
            "Required for the chart and connections page",
        ))
        .active(settings.stats_enabled)
        .build();
    stats.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.stats_enabled = value);
        }
    });
    group.add(&stats);
    page.add(&group);

    let group = adw::PreferencesGroup::builder()
        .title(crate::i18n::tr("Журнал", "Log"))
        .build();
    let levels = ["trace", "debug", "info", "warn", "error"];
    let level = adw::ComboRow::builder()
        .title(crate::i18n::tr("Подробность", "Verbosity"))
        .model(&gtk::StringList::new(&levels))
        .selected(
            levels
                .iter()
                .position(|l| *l == settings.log_level)
                .unwrap_or(2) as u32,
        )
        .build();
    level.connect_selected_notify({
        let window = window.clone();
        move |row| {
            let value = levels[row.selected() as usize].to_string();
            edit(&window, |s| s.log_level = value);
        }
    });
    group.add(&level);
    page.add(&group);

    let group = adw::PreferencesGroup::builder()
        .title(crate::i18n::tr("Подписки", "Subscriptions"))
        .build();

    let auto = adw::SwitchRow::builder()
        .title(crate::i18n::tr(
            "Обновлять автоматически",
            "Update automatically",
        ))
        .active(settings.sub_auto_update)
        .build();
    auto.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.sub_auto_update = value);
        }
    });
    group.add(&auto);

    let interval = adw::SpinRow::builder()
        .title(crate::i18n::tr("Интервал, минут", "Interval, minutes"))
        .adjustment(&gtk::Adjustment::new(
            settings.sub_auto_update_minutes as f64,
            10.0,
            10080.0,
            10.0,
            60.0,
            0.0,
        ))
        .build();
    interval.connect_value_notify({
        let window = window.clone();
        move |row| {
            let value = row.value() as i64;
            edit(&window, |s| s.sub_auto_update_minutes = value);
        }
    });
    group.add(&interval);

    let agent = adw::EntryRow::builder()
        .title("User-Agent")
        .text(&settings.sub_user_agent)
        .build();
    agent.connect_changed({
        let window = window.clone();
        move |row| {
            let value = row.text().to_string();
            edit(&window, |s| s.sub_user_agent = value);
        }
    });
    group.add(&agent);

    let send_hwid = adw::SwitchRow::builder()
        .title(crate::i18n::tr(
            "Отправлять HWID при обновлении",
            "Send HWID when updating",
        ))
        .subtitle(crate::i18n::tr(
            "Помогает подписке связать обновление с этим устройством",
            "Helps the subscription associate updates with this device",
        ))
        .active(settings.sub_send_hwid)
        .build();
    send_hwid.connect_active_notify({
        let window = window.clone();
        move |row| {
            let value = row.is_active();
            edit(&window, |s| s.sub_send_hwid = value);
        }
    });
    group.add(&send_hwid);

    let custom_hwid = adw::EntryRow::builder()
        .title(crate::i18n::tr(
            "Параметры HWID (необязательно)",
            "HWID parameters (optional)",
        ))
        .tooltip_text(crate::i18n::tr(
            "Формат: hwid=...,os=...,osVersion=...,model=...",
            "Format: hwid=...,os=...,osVersion=...,model=...",
        ))
        .text(&settings.sub_custom_hwid_params)
        .build();
    custom_hwid.connect_changed({
        let window = window.clone();
        move |row| {
            let value = row.text().to_string();
            edit(&window, |s| s.sub_custom_hwid_params = value);
        }
    });
    group.add(&custom_hwid);
    page.add(&group);

    page
}

/// How many rules are configured, visible without opening the editor.
fn rules_subtitle(settings: &nolurix_config::Settings) -> String {
    let total = settings.route_rules.len();
    let enabled = settings.route_rules.iter().filter(|r| r.enabled).count();
    let raw = !settings.custom_route_rules.trim().is_empty();
    match (total, raw) {
        (0, false) => crate::i18n::tr("Ничего не задано", "No rules configured").into(),
        (0, true) => crate::i18n::tr("Задано текстом", "Configured as text").into(),
        (n, false) if n == enabled => crate::tr_format!("Правил: {n}", "Rules: {n}"),
        (n, false) => crate::tr_format!(
            "Правил: {n}, из них выключено {}",
            "Rules: {n}, disabled: {}",
            n - enabled
        ),
        (n, true) => crate::tr_format!(
            "Правил: {n} и правила текстом",
            "Rules: {n}, plus text rules"
        ),
    }
}

/// Text editor for cases that cannot be expressed by the form: this is a sing-box
/// config fragment, and a form layered over it would misrepresent its capabilities.
pub(crate) fn open_rules_editor(window: &Rc<Window>) {
    let view = gtk::TextView::builder()
        .monospace(true)
        .top_margin(10)
        .bottom_margin(10)
        .left_margin(12)
        .right_margin(12)
        .build();
    view.buffer()
        .set_text(&window.state().settings.borrow().custom_route_rules);

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&view)
        .build();

    let dialog = adw::Dialog::builder()
        .title(crate::i18n::tr(
            "Свои правила маршрутизации",
            "Custom routing rules",
        ))
        .content_width(620)
        .content_height(520)
        .build();

    let header = adw::HeaderBar::new();
    let save = gtk::Button::builder()
        .label(crate::i18n::tr("Сохранить", "Save"))
        .css_classes(["suggested-action"])
        .build();
    header.pack_end(&save);

    let hint = gtk::Label::builder()
        .label(crate::i18n::tr("Правила добавляются перед автоматическими. Пример:\n[{\"domain_suffix\": [\".ru\"], \"action\": \"route\", \"outbound\": \"direct\"}]", "Rules are added before automatic rules. Example:\n[{\"domain_suffix\": [\".ru\"], \"action\": \"route\", \"outbound\": \"direct\"}]"))
        .xalign(0.0)
        .margin_top(10)
        .margin_start(12)
        .margin_end(12)
        .wrap(true)
        .css_classes(["empty-body"])
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content.append(&hint);
    content.append(&scroller);

    let toolbar = adw::ToolbarView::builder().content(&content).build();
    toolbar.add_top_bar(&header);
    dialog.set_child(Some(&toolbar));

    save.connect_clicked({
        let window = window.clone();
        let dialog = dialog.clone();
        let view = view.clone();
        move |_| {
            let buffer = view.buffer();
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            // Validate parsing here; otherwise the error appears only when
            // connecting, after the rules have already been forgotten.
            if !text.trim().is_empty() {
                if let Err(e) = serde_json::from_str::<serde_json::Value>(text.trim()) {
                    window.toast(&crate::tr_format!("Это не JSON: {e}", "Invalid JSON: {e}"));
                    return;
                }
            }
            edit(&window, |s| s.custom_route_rules = text);
            dialog.close();
            window.toast(crate::i18n::tr("Правила сохранены", "Rules saved"));
        }
    });

    dialog.present(Some(&window.root));
}
