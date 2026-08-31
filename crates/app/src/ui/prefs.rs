//! Настройки.
//!
//! Каждый переключатель пишет значение сразу — отдельной кнопки «Применить»
//! нет. Изменения, влияющие на живое соединение, применяются при следующем
//! подключении; об этом сказано в подписи, а не выясняется опытом.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::ui::Window;
use throne_config::settings::{DomainStrategy, TunStack};

pub fn open(window: &Rc<Window>) {
    let dialog = adw::PreferencesDialog::builder()
        .title("Настройки")
        .content_width(560)
        .content_height(640)
        .build();

    dialog.add(&connection_page(window));
    dialog.add(&network_page(window));
    dialog.add(&core_page(window));
    dialog.present(Some(&window.root));
}

/// Мелкая обвязка: применить изменение к настройкам и сохранить.
fn edit(window: &Rc<Window>, apply: impl FnOnce(&mut throne_config::Settings)) {
    {
        let mut settings = window.state().settings.borrow_mut();
        apply(&mut settings);
    }
    if let Err(e) = window.state().save_settings() {
        window.toast(&format!("Не удалось сохранить настройки: {e}"));
    }
}

fn connection_page(window: &Rc<Window>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Соединение")
        .icon_name("network-workgroup-symbolic")
        .build();

    let settings = window.state().settings.borrow().clone();

    let group = adw::PreferencesGroup::builder()
        .title("Локальный прокси")
        .description("Порт, на который настраиваются браузер и другие программы")
        .build();

    let port = adw::SpinRow::builder()
        .title("Порт")
        .subtitle("SOCKS5 и HTTP на одном порту")
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
        .title("Открыть доступ из локальной сети")
        .subtitle("Прокси примет подключения с других устройств в сети")
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
        .title("Определять протокол и домен")
        .subtitle("Нужно для правил по доменам и понятных имён в соединениях")
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
        .title("Режим VPN")
        .description("Применяется при следующем подключении")
        .build();

    let stacks = ["Смешанный", "Системный", "gVisor"];
    let stack_values = [TunStack::Mixed, TunStack::System, TunStack::GVisor];
    let stack = adw::ComboRow::builder()
        .title("Сетевой стек")
        .subtitle("Смешанный подходит в большинстве случаев")
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
        .title("Строгая маршрутизация")
        .subtitle("Закрывает обход туннеля мимо правил")
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
        .title("IPv6 в туннеле")
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
        .title("Окно")
        .description(
            "Значок в трее рисует панель рабочего стола; без неё окно закрывается как обычно",
        )
        .build();

    let to_tray = adw::SwitchRow::builder()
        .title("Прятать в значок при закрытии")
        .subtitle("Соединение продолжает работать, окно возвращается щелчком по значку")
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
        .title("Запускать в значке")
        .subtitle("При старте показывать только значок, без окна")
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
        .title("Сеть")
        .icon_name("network-server-symbolic")
        .build();

    let settings = window.state().settings.borrow().clone();

    let group = adw::PreferencesGroup::builder()
        .title("DNS")
        .description("Схема в адресе задаёт транспорт: udp://, tls://, https://, quic://")
        .build();

    let enabled = adw::SwitchRow::builder()
        .title("Своя служба DNS")
        .subtitle("Без неё запросы уходят системному резолверу")
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
        .title("Через прокси")
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
        .title("Напрямую")
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
        "Как есть",
        "Сначала IPv4",
        "Сначала IPv6",
        "Только IPv4",
        "Только IPv6",
    ];
    let strategy_values = [
        DomainStrategy::AsIs,
        DomainStrategy::PreferIpv4,
        DomainStrategy::PreferIpv6,
        DomainStrategy::Ipv4Only,
        DomainStrategy::Ipv6Only,
    ];
    let strategy = adw::ComboRow::builder()
        .title("Разрешение имён")
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
        .title("Подставные адреса")
        .subtitle("Ускоряет соединение, но ломает программы, которым нужен настоящий IP")
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
        .title("Маршрутизация")
        .build();

    let bypass = adw::SwitchRow::builder()
        .title("Локальные адреса мимо прокси")
        .subtitle("Домашняя сеть, .local и приватные диапазоны идут напрямую")
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
        .title("Правила маршрутизации")
        .subtitle(&rules_subtitle(&settings))
        .activatable(true)
        .build();
    rules.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    rules.connect_activated({
        let window = window.clone();
        move |row| {
            crate::ui::routes::open(&window);
            // Подпись обновится, когда человек вернётся из редактора.
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
        .title("Проверка серверов")
        .build();

    let url = adw::EntryRow::builder()
        .title("Адрес для проверки")
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
        .title("Таймаут, мс")
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
        .title("Одновременных проверок")
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
        .title("Автовыбор сервера")
        .description("Ядро само держит лучший сервер группы и уходит с упавшего")
        .build();

    let interval = adw::SpinRow::builder()
        .title("Перепроверка, секунд")
        .subtitle("Как часто пересматриваются лучшие серверы группы")
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
        .title("Запас на переключение, мс")
        .subtitle("Насколько новый сервер должен быть быстрее текущего")
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
        .title("Распределять по нескольким серверам")
        .subtitle("Вместо одного лучшего используются все подходящие по очереди")
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
        .title("Замер скорости")
        .description("Запускается для одного сервера из меню строки")
        .build();

    let download = adw::SwitchRow::builder()
        .title("Мерить приём")
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
        .title("Мерить отдачу")
        .subtitle("Удваивает время замера")
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
        .title("Длительность замера, мс")
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
        .title("Ядро")
        .icon_name("applications-engineering-symbolic")
        .build();

    let settings = window.state().settings.borrow().clone();

    let group = adw::PreferencesGroup::builder().title("Соединения").build();

    let mux = adw::SwitchRow::builder()
        .title("Мультиплексирование")
        .subtitle("Несколько запросов в одном соединении. Не применяется к XTLS и QUIC")
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
        .title("Протокол мультиплексирования")
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
        .title("Считать трафик и соединения")
        .subtitle("Нужно для графика и страницы соединений")
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

    let group = adw::PreferencesGroup::builder().title("Журнал").build();
    let levels = ["trace", "debug", "info", "warn", "error"];
    let level = adw::ComboRow::builder()
        .title("Подробность")
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

    let group = adw::PreferencesGroup::builder().title("Подписки").build();

    let auto = adw::SwitchRow::builder()
        .title("Обновлять автоматически")
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
        .title("Интервал, минут")
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
        .title("Отправлять HWID при обновлении")
        .subtitle("Помогает подписке связать обновление с этим устройством")
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
        .title("Параметры HWID (необязательно)")
        .tooltip_text("Формат: hwid=...,os=...,osVersion=...,model=...")
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

/// Сколько правил задано — видно, не открывая редактор.
fn rules_subtitle(settings: &throne_config::Settings) -> String {
    let total = settings.route_rules.len();
    let enabled = settings.route_rules.iter().filter(|r| r.enabled).count();
    let raw = !settings.custom_route_rules.trim().is_empty();
    match (total, raw) {
        (0, false) => "Ничего не задано".into(),
        (0, true) => "Задано текстом".into(),
        (n, false) if n == enabled => format!("Правил: {n}"),
        (n, false) => format!("Правил: {n}, из них выключено {}", n - enabled),
        (n, true) => format!("Правил: {n} и правила текстом"),
    }
}

/// Текстовый редактор для случаев, которые не выражаются формой: это
/// фрагмент конфига sing-box, и форма поверх него врала бы о его возможностях.
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
        .title("Свои правила маршрутизации")
        .content_width(620)
        .content_height(520)
        .build();

    let header = adw::HeaderBar::new();
    let save = gtk::Button::builder()
        .label("Сохранить")
        .css_classes(["suggested-action"])
        .build();
    header.pack_end(&save);

    let hint = gtk::Label::builder()
        .label("Правила добавляются перед автоматическими. Пример:\n[{\"domain_suffix\": [\".ru\"], \"action\": \"route\", \"outbound\": \"direct\"}]")
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
            // Проверяем разбор здесь: иначе ошибка всплывёт только при
            // подключении, когда правила уже забыты.
            if !text.trim().is_empty() {
                if let Err(e) = serde_json::from_str::<serde_json::Value>(text.trim()) {
                    window.toast(&format!("Это не JSON: {e}"));
                    return;
                }
            }
            edit(&window, |s| s.custom_route_rules = text);
            dialog.close();
            window.toast("Правила сохранены");
        }
    });

    dialog.present(Some(&window.root));
}
