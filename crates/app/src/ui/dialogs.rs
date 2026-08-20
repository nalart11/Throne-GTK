//! Диалоги: добавление серверов и подписок, импорт из Throne, «О программе».

use std::rc::Rc;

use adw::prelude::*;

use crate::engine::Command;
use crate::paths;
use crate::ui::Window;
use throne_config::profile::Group;
use throne_config::link;

/// Одно окно на оба способа добавления: вставить ссылки или подписаться.
/// Разделены заголовками, а не вкладками — выбор очевиден по тому, что у
/// человека в буфере обмена.
pub fn add_dialog(window: &Rc<Window>) {
    let dialog = adw::Dialog::builder()
        .title("Добавить серверы")
        .content_width(560)
        // Высота подобрана так, чтобы обе кнопки — «Добавить» и «Подписаться» —
        // были видны сразу: иначе второй способ добавления выглядит
        // отсутствующим.
        .content_height(660)
        .build();

    let header = adw::HeaderBar::new();

    let page = adw::PreferencesPage::new();

    // ── ссылки ──────────────────────────────────────────────────────────
    let links_group = adw::PreferencesGroup::builder()
        .title("Вставить ссылки")
        .description("По одной в строке: vless://, vmess://, trojan://, ss://, hysteria2://, tuic://")
        .build();

    let view = gtk::TextView::builder()
        .monospace(true)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(10)
        .right_margin(10)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .height_request(120)
        .child(&view)
        .css_classes(["card"])
        .build();
    links_group.add(&scroller);

    let add_links = gtk::Button::builder()
        .label("Добавить")
        .halign(gtk::Align::End)
        .margin_top(8)
        .css_classes(["suggested-action"])
        .build();
    links_group.add(&add_links);
    page.add(&links_group);

    // ── подписка ────────────────────────────────────────────────────────
    let sub_group = adw::PreferencesGroup::builder()
        .title("Подписка")
        .description("Ссылка на список серверов; обновляется по кнопке в списке")
        .build();

    let url_row = adw::EntryRow::builder().title("Адрес подписки").build();
    let name_row = adw::EntryRow::builder()
        .title("Название группы")
        .build();
    sub_group.add(&url_row);
    sub_group.add(&name_row);

    let add_sub = gtk::Button::builder()
        .label("Подписаться")
        .halign(gtk::Align::End)
        .margin_top(8)
        .css_classes(["suggested-action"])
        .build();
    sub_group.add(&add_sub);
    page.add(&sub_group);

    let toolbar = adw::ToolbarView::builder().content(&page).build();
    toolbar.add_top_bar(&header);
    dialog.set_child(Some(&toolbar));

    add_links.connect_clicked({
        let window = window.clone();
        let dialog = dialog.clone();
        let view = view.clone();
        move |_| {
            let buffer = view.buffer();
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            let (profiles, errors) = link::parse_many(&text);
            if profiles.is_empty() {
                window.toast(&match errors.first() {
                    Some(first) => format!("Не разобрать: {first}"),
                    None => "Вставьте хотя бы одну ссылку".to_string(),
                });
                return;
            }

            let gid = window.state().current_gid.get();
            let store = window.state().store.borrow();
            let mut added = 0;
            for mut profile in profiles {
                profile.gid = gid;
                if store.insert_profile(&profile).is_ok() {
                    added += 1;
                }
            }
            drop(store);

            window.servers_page().reload();
            dialog.close();
            window.toast(&match errors.len() {
                0 => format!("Добавлено серверов: {added}"),
                skipped => format!("Добавлено: {added}, пропущено: {skipped}"),
            });
        }
    });

    add_sub.connect_clicked({
        let window = window.clone();
        let dialog = dialog.clone();
        let url_row = url_row.clone();
        let name_row = name_row.clone();
        move |_| {
            let url = url_row.text().trim().to_string();
            if url.is_empty() {
                window.toast("Укажите адрес подписки");
                return;
            }
            let name = match name_row.text().trim() {
                "" => host_of(&url),
                name => name.to_string(),
            };

            let gid = {
                let store = window.state().store.borrow();
                store.insert_group(&Group {
                    name,
                    url: url.clone(),
                    ..Default::default()
                })
            };
            let Ok(gid) = gid else {
                window.toast("Не удалось создать группу");
                return;
            };

            window.state().current_gid.set(gid);
            window.servers_page().reload();
            window.servers_page().set_updating(true);
            window.state().engine.send(Command::FetchSubscription {
                gid,
                url,
                user_agent: window.state().settings.borrow().sub_user_agent.clone(),
            });
            dialog.close();
            window.toast("Загружаю подписку…");
        }
    });

    dialog.present(Some(&window.root));
}

/// Обновление текущей подписки.
pub fn update_subscription(window: &Rc<Window>) {
    let gid = window.state().current_gid.get();
    let group = window.state().store.borrow().group(gid).ok().flatten();
    let Some(group) = group else { return };
    if !group.is_subscription() {
        window.toast("Эта группа не подписка — обновлять нечего");
        return;
    }
    window.servers_page().set_updating(true);
    window.state().engine.send(Command::FetchSubscription {
        gid,
        url: group.url,
        user_agent: window.state().settings.borrow().sub_user_agent.clone(),
    });
    window.toast("Обновляю подписку…");
}

/// Перенос данных из оригинального Throne. Импорт добавляет группы, а не
/// заменяет существующие: повторный запуск создаст копии, о чём предупреждаем.
pub fn import_dialog(window: &Rc<Window>) {
    let path = paths::throne_database();
    let dialog = adw::AlertDialog::builder()
        .heading("Импорт из Throne")
        .body(format!(
            "Серверы, подписки и правила маршрутизации будут скопированы из\n{}\n\n\
             База Throne останется нетронутой. \
             Повторный импорт создаст ещё одну копию групп.",
            path.display()
        ))
        .build();
    dialog.add_response("cancel", "Отмена");
    dialog.add_response("import", "Импортировать");
    dialog.set_response_appearance("import", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("import"));

    dialog.connect_response(None, {
        let window = window.clone();
        move |_, response| {
            if response != "import" {
                return;
            }
            let outcome = window.state().store.borrow_mut().import_throne(&paths::throne_database());
            match outcome {
                Ok(done) => {
                    // Правила импорт кладёт прямо в базу — окну надо их
                    // перечитать, иначе они не появятся и будут затёрты
                    // следующим сохранением настроек.
                    if let Err(e) = window.state().reload_settings() {
                        tracing::warn!("настройки после импорта не перечитаны: {e:#}");
                    }
                    window.state().focus_group_with_servers();
                    window.servers_page().reload();
                    let mut text = format!(
                        "Перенесено групп: {}, серверов: {}, правил: {}",
                        done.groups, done.profiles, done.rules
                    );
                    // Про пропущенные молчать нельзя: человек считает, что
                    // маршрутизация переехала целиком, и не поймёт, почему
                    // часть трафика идёт не туда.
                    if done.skipped_rules > 0 {
                        text.push_str(&format!(
                            ". Правил не перенеслось: {} — задайте их заново",
                            done.skipped_rules
                        ));
                    }
                    window.toast(&text);
                }
                Err(e) => window.toast(&format!("Импорт не удался: {e}")),
            }
        }
    });

    dialog.present(Some(&window.root));
}

pub fn about(window: &Rc<Window>) {
    let about = adw::AboutDialog::builder()
        .application_name("Throne GTK")
        .application_icon("network-server-symbolic")
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Форк Throne на GTK4")
        .comments(
            "Клиент прокси на ядре sing-box и Xray.\n\
             Форк Throne с интерфейсом на GTK4 и libadwaita.",
        )
        .license_type(gtk::License::Gpl30)
        .website("https://github.com/throneproj/Throne")
        .build();
    about.present(Some(&window.root));
}

/// Имя хоста из адреса подписки — годится как название группы по умолчанию.
fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(url)
        .to_string()
}

/// Проверка конфигурации выбранного сервера без подключения.
pub fn check_config(window: &Rc<Window>) {
    let Some(profile) = window.state().selected_profile() else {
        window.toast("Сначала выберите сервер");
        return;
    };
    window.state().engine.send(Command::CheckConfig {
        profile: Box::new(profile),
        settings: Box::new(window.state().settings.borrow().clone()),
    });
    window.toast("Проверяю конфигурацию…");
}
