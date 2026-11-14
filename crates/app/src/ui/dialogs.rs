//! Dialogs: adding servers and subscriptions, importing from Throne, and “About.”

use std::rc::Rc;

use adw::prelude::*;

use crate::engine::Command;
use crate::i18n::tr;
use crate::paths;
use crate::ui::Window;
use nolurix_config::link;
use nolurix_config::profile::Group;

/// One window for both adding methods: paste links or subscribe.
/// They are separated by headings rather than tabs; the choice is obvious from
/// what the user has in the clipboard.
pub fn add_dialog(window: &Rc<Window>) {
    let dialog = adw::Dialog::builder()
        .title(tr("Добавить серверы", "Add servers"))
        .content_width(560)
        // The height is chosen so both buttons, “Add” and “Subscribe,” are visible
        // immediately; otherwise the second adding method appears to be missing.
        .content_height(660)
        .build();

    let header = adw::HeaderBar::new();

    let page = adw::PreferencesPage::new();

    // ── links ──────────────────────────────────────────────────────────
    let links_group = adw::PreferencesGroup::builder()
        .title(tr("Вставить ссылки", "Paste links"))
        .description(tr(
            "По одной в строке: vless://, vmess://, trojan://, ss://, hysteria2://, tuic://",
            "One per line: vless://, vmess://, trojan://, ss://, hysteria2://, tuic://",
        ))
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
        .label(tr("Добавить", "Add"))
        .halign(gtk::Align::End)
        .margin_top(8)
        .css_classes(["suggested-action"])
        .build();
    links_group.add(&add_links);
    page.add(&links_group);

    // ── subscription ────────────────────────────────────────────────────────
    let sub_group = adw::PreferencesGroup::builder()
        .title(tr("Подписка", "Subscription"))
        .description(tr(
            "Ссылка на список серверов; обновляется по кнопке в списке",
            "A server list URL; update it using the refresh button",
        ))
        .build();

    let url_row = adw::EntryRow::builder()
        .title(tr("Адрес подписки", "Subscription URL"))
        .build();
    let name_row = adw::EntryRow::builder()
        .title(tr("Название группы", "Group name"))
        .build();
    sub_group.add(&url_row);
    sub_group.add(&name_row);

    let add_sub = gtk::Button::builder()
        .label(tr("Подписаться", "Subscribe"))
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
                    Some(first) => {
                        crate::tr_format!("Не разобрать: {first}", "Could not parse: {first}")
                    }
                    None => {
                        tr("Вставьте хотя бы одну ссылку", "Paste at least one link").to_string()
                    }
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
                0 => crate::tr_format!("Добавлено серверов: {added}", "Servers added: {added}"),
                skipped => crate::tr_format!(
                    "Добавлено: {added}, пропущено: {skipped}",
                    "Added: {added}, skipped: {skipped}"
                ),
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
                window.toast(tr("Укажите адрес подписки", "Enter a subscription URL"));
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
                window.toast(tr("Не удалось создать группу", "Could not create group"));
                return;
            };

            window.state().current_gid.set(gid);
            window.servers_page().reload();
            window.servers_page().set_updating(true);
            window.state().engine.send(Command::FetchSubscription {
                gid,
                url,
                user_agent: window.state().settings.borrow().sub_user_agent.clone(),
                send_hwid: window.state().settings.borrow().sub_send_hwid,
                custom_hwid_params: window
                    .state()
                    .settings
                    .borrow()
                    .sub_custom_hwid_params
                    .clone(),
            });
            dialog.close();
            window.toast(tr("Загружаю подписку…", "Downloading subscription…"));
        }
    });

    dialog.present(Some(&window.root));
}

/// Update the current subscription.
pub fn update_subscription(window: &Rc<Window>) {
    let gid = window.state().current_gid.get();
    let group = window.state().store.borrow().group(gid).ok().flatten();
    let Some(group) = group else { return };
    if !group.is_subscription() {
        window.toast(tr(
            "Эта группа не подписка — обновлять нечего",
            "This group is not a subscription — nothing to update",
        ));
        return;
    }
    window.servers_page().set_updating(true);
    window.state().engine.send(Command::FetchSubscription {
        gid,
        url: group.url,
        user_agent: window.state().settings.borrow().sub_user_agent.clone(),
        send_hwid: window.state().settings.borrow().sub_send_hwid,
        custom_hwid_params: window
            .state()
            .settings
            .borrow()
            .sub_custom_hwid_params
            .clone(),
    });
    window.toast(tr("Обновляю подписку…", "Updating subscription…"));
}

/// Import data from the original Throne. Import adds groups rather than replacing
/// existing ones; a second run creates copies, as explained to the user.
pub fn import_dialog(window: &Rc<Window>) {
    let path = paths::throne_database();
    let dialog = adw::AlertDialog::builder()
        .heading(tr("Импорт из Throne", "Import from Throne"))
        .body(crate::tr_format!(
            "Серверы, подписки и правила маршрутизации будут скопированы из\n{}\n\n\
             База Throne останется нетронутой. \
             Повторный импорт создаст ещё одну копию групп.",
            "Servers, subscriptions and routing rules will be copied from\n{}\n\nThe Throne database will remain unchanged. Importing again will create another copy of the groups.",
            path.display()
        ))
        .build();
    dialog.add_response("cancel", tr("Отмена", "Cancel"));
    dialog.add_response("import", tr("Импортировать", "Import"));
    dialog.set_response_appearance("import", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("import"));

    dialog.connect_response(None, {
        let window = window.clone();
        move |_, response| {
            if response != "import" {
                return;
            }
            let outcome = window
                .state()
                .store
                .borrow_mut()
                .import_throne(&paths::throne_database());
            match outcome {
                Ok(done) => {
                    // Import places rules directly in the database; the window must
                    // reload them or they will not appear and will be overwritten
                    // by the next settings save.
                    if let Err(e) = window.state().reload_settings() {
                        tracing::warn!("настройки после импорта не перечитаны: {e:#}");
                    }
                    window.state().focus_group_with_servers();
                    window.servers_page().reload();
                    let mut text = crate::tr_format!(
                        "Перенесено групп: {}, серверов: {}, правил: {}",
                        "Imported groups: {}, servers: {}, rules: {}",
                        done.groups,
                        done.profiles,
                        done.rules
                    );
                    // Omitted items must be reported: the user would assume routing
                    // was fully imported and would not understand misplaced traffic.
                    if done.skipped_rules > 0 {
                        text.push_str(&crate::tr_format!(
                            ". Правил не перенеслось: {} — задайте их заново",
                            ". Rules skipped: {} — recreate them manually",
                            done.skipped_rules
                        ));
                    }
                    window.toast(&text);
                }
                Err(e) => window.toast(&crate::tr_format!(
                    "Импорт не удался: {e}",
                    "Import failed: {e}"
                )),
            }
        }
    });

    dialog.present(Some(&window.root));
}

pub fn about(window: &Rc<Window>) {
    let about = adw::AboutDialog::builder()
        .application_name("Nolurix")
        .application_icon("network-server-symbolic")
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Nolurix contributors")
        .comments(
            tr("Nolurix — независимый клиент прокси на ядре sing-box и Xray.\n\
             Интерфейс на GTK4 и libadwaita.", "Nolurix — an independent proxy client powered by sing-box and Xray.\nBuilt with GTK4 and libadwaita."),
        )
        .license_type(gtk::License::Gpl30)
        .website("https://github.com/nalart11/Nolurix")
        .build();
    about.add_credit_section(
        Some(tr("На основе проекта", "Originally based on")),
        &["Throne contributors https://github.com/throneproj/Throne"],
    );
    about.present(Some(&window.root));
}

/// Host name from the subscription address, suitable as the default group name.
fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(url)
        .to_string()
}

/// Check the selected server configuration without connecting.
pub fn check_config(window: &Rc<Window>) {
    let Some(profile) = window.state().selected_profile() else {
        window.toast(tr("Сначала выберите сервер", "Select a server first"));
        return;
    };
    window.state().engine.send(Command::CheckConfig {
        profile: Box::new(profile),
        settings: Box::new(window.state().settings.borrow().clone()),
    });
    window.toast(tr("Проверяю конфигурацию…", "Checking configuration…"));
}
