//! Routing rules.
//!
//! Rule list and single-rule editor. List order is application order: the first
//! matching rule is used, so the move arrows set priority rather than decorate.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::i18n::tr;
use crate::ui::Window;
use nolurix_config::route::{Condition, MatchKind, MatchMode, RouteAction, RouteRule};

// Localized presentation only: stored rule names and condition values are data.
fn kind_title(kind: MatchKind) -> &'static str {
    let en = match kind {
        MatchKind::DomainSuffix => "Domain and subdomains",
        MatchKind::Domain => "Exact domain",
        MatchKind::DomainKeyword => "Domain keyword",
        MatchKind::DomainRegex => "Domain expression",
        MatchKind::IpCidr => "Address or subnet",
        MatchKind::Port => "Port",
        MatchKind::Process => "Process",
        MatchKind::RuleSet => "Rule set",
    };
    tr(kind.title(), en)
}

fn kind_hint(kind: MatchKind) -> &'static str {
    let en = match kind {
        MatchKind::DomainSuffix => "example.com — the domain and all subdomains",
        MatchKind::Domain => "example.com — this domain only",
        MatchKind::DomainKeyword => "part of the name, for example: googlevideo",
        MatchKind::DomainRegex => "regular expression: ^ads\\..*",
        MatchKind::IpCidr => "10.0.0.0/8 or 192.168.1.5",
        MatchKind::Port => "443 or a range: 8000:8100",
        MatchKind::Process => "firefox, Telegram",
        MatchKind::RuleSet => "geosite-ru, geoip-ru — downloaded automatically",
    };
    tr(kind.hint(), en)
}

fn action_title(action: RouteAction) -> &'static str {
    tr(
        action.title(),
        match action {
            RouteAction::Proxy => "Through proxy",
            RouteAction::Direct => "Direct",
            RouteAction::Block => "Block",
        },
    )
}

fn mode_title(mode: MatchMode) -> &'static str {
    tr(
        mode.title(),
        match mode {
            MatchMode::Any => "Match any condition",
            MatchMode::All => "Match all conditions",
        },
    )
}

fn conditions_title(rule: &RouteRule) -> String {
    if rule.conditions.is_empty() {
        return tr("Условий нет", "No conditions").into();
    }
    let separator = match rule.match_mode {
        MatchMode::Any => tr(" или ", " or "),
        MatchMode::All => tr(" и ", " and "),
    };
    rule.conditions
        .iter()
        .map(|c| kind_title(c.kind))
        .collect::<Vec<_>>()
        .join(separator)
}

fn rule_display_name(rule: &RouteRule) -> String {
    if !rule.name.trim().is_empty() {
        return rule.name.clone();
    }
    let values: Vec<&String> = rule
        .conditions
        .iter()
        .flat_map(|c| c.values.iter())
        .filter(|v| !v.trim().is_empty())
        .collect();
    let summary = match values.len() {
        0 => tr("условие не задано", "no condition set").into(),
        1 => values[0].clone(),
        n => crate::tr_format!("{} и ещё {}", "{} and {} more", values[0], n - 1),
    };
    format!("{}: {}", conditions_title(rule), summary)
}

pub fn open(window: &Rc<Window>) {
    let dialog = adw::Dialog::builder()
        .title(tr("Правила маршрутизации", "Routing rules"))
        .content_width(620)
        .content_height(640)
        .build();

    let header = adw::HeaderBar::new();
    let add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(tr("Добавить правило", "Add rule"))
        .build();
    header.pack_start(&add);

    let import = gtk::Button::builder()
        .icon_name("document-import-symbolic")
        .tooltip_text(tr(
            "Перенести правила из Throne",
            "Import rules from Throne",
        ))
        .build();
    header.pack_end(&import);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title(tr("Правила", "Rules"))
        .description(tr(
            "Применяется первое подошедшее. Всё, что не подошло, идёт через прокси",
            "The first matching rule applies. Unmatched traffic goes through the proxy",
        ))
        .build();
    page.add(&group);

    let extra = adw::PreferencesGroup::builder()
        .title(tr("Дополнительно", "Advanced"))
        .description(tr(
            "Правила в формате sing-box для случаев, которые не выражаются формой",
            "Rules in sing-box format for cases not covered by the editor",
        ))
        .build();
    let raw_row = adw::ActionRow::builder()
        .title(tr("Правила текстом", "Raw rules"))
        .activatable(true)
        .build();
    raw_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    raw_row.connect_activated({
        let window = window.clone();
        move |_| crate::ui::prefs::open_rules_editor(&window)
    });
    extra.add(&raw_row);
    page.add(&extra);

    let toolbar = adw::ToolbarView::builder().content(&page).build();
    toolbar.add_top_bar(&header);
    dialog.set_child(Some(&toolbar));

    let rebuild: Rc<dyn Fn()> = {
        let window = window.clone();
        let group = group.clone();
        let dialog = dialog.clone();
        Rc::new(move || fill(&window, &group, &dialog))
    };
    rebuild();

    add.connect_clicked({
        let window = window.clone();
        let dialog = dialog.clone();
        let rebuild = rebuild.clone();
        // A new rule appears in the list only after saving; otherwise a user who
        // changes their mind leaves empty rows behind.
        move |_| edit_rule(&window, &dialog, None, rebuild.clone())
    });

    import.connect_clicked({
        let window = window.clone();
        let rebuild = rebuild.clone();
        // Rules are imported separately from servers: they have usually already
        // been imported, and another full import would duplicate the groups.
        move |_| {
            let outcome = window
                .state()
                .store
                .borrow_mut()
                .import_throne_routes(&crate::paths::throne_database());
            match outcome {
                Ok((0, 0)) => window.toast(tr(
                    "В Throne нет правил для переноса",
                    "No rules to import from Throne",
                )),
                Ok((added, skipped)) => {
                    // Import writes directly to the database while the window has
                    // its own settings copy: without reloading, rules are hidden,
                    // and the first settings save overwrites them.
                    if let Err(e) = window.state().reload_settings() {
                        window.toast(&crate::tr_format!(
                            "Правила перенесены, но не прочитаны: {e}",
                            "Rules imported but could not be loaded: {e}"
                        ));
                        return;
                    }
                    rebuild();
                    let mut text =
                        crate::tr_format!("Перенесено правил: {added}", "Rules imported: {added}");
                    // This cannot be omitted: the user would assume routing was
                    // fully imported and would not understand misplaced traffic.
                    if skipped > 0 {
                        text.push_str(&crate::tr_format!(
                            ". Не перенеслось: {skipped} — задайте их заново",
                            ". Skipped: {skipped} — recreate them manually"
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

/// Rebuilds the rule list completely: there are few rules, and partial updates
/// of rows with switches and arrows would cost more than they provide.
fn fill(window: &Rc<Window>, group: &adw::PreferencesGroup, dialog: &adw::Dialog) {
    while let Some(child) = group.first_child().and_then(find_listbox_row) {
        group.remove(&child);
    }

    let rules = window.state().settings.borrow().route_rules.clone();
    if rules.is_empty() {
        let empty = adw::ActionRow::builder()
            .title(tr("Правил нет", "No rules"))
            .subtitle(tr(
                "Весь трафик идёт через прокси, кроме локальных адресов",
                "All traffic goes through the proxy except local addresses",
            ))
            .build();
        empty.add_css_class("dim-label");
        group.add(&empty);
        return;
    }

    let rebuild: Rc<dyn Fn()> = {
        let window = window.clone();
        let group = group.clone();
        let dialog = dialog.clone();
        Rc::new(move || fill(&window, &group, &dialog))
    };

    let total = rules.len();
    for (index, rule) in rules.iter().enumerate() {
        let row = adw::ActionRow::builder()
            .title(rule_display_name(rule))
            .subtitle(format!(
                "{} · {}",
                conditions_title(rule),
                action_title(rule.action)
            ))
            .activatable(true)
            .build();

        let toggle = gtk::Switch::builder()
            .active(rule.enabled)
            .valign(gtk::Align::Center)
            .tooltip_text(tr(
                "Включить или выключить правило",
                "Enable or disable rule",
            ))
            .build();
        toggle.connect_state_set({
            let window = window.clone();
            let rebuild = rebuild.clone();
            move |_, state| {
                {
                    let mut settings = window.state().settings.borrow_mut();
                    if let Some(rule) = settings.route_rules.get_mut(index) {
                        rule.enabled = state;
                    }
                }
                save(&window);
                rebuild();
                glib_propagation()
            }
        });

        let up = gtk::Button::builder()
            .icon_name("go-up-symbolic")
            .valign(gtk::Align::Center)
            .sensitive(index > 0)
            .tooltip_text(tr("Выше по приоритету", "Higher priority"))
            .css_classes(["flat"])
            .build();
        up.connect_clicked({
            let window = window.clone();
            let rebuild = rebuild.clone();
            move |_| {
                move_rule(&window, index, index - 1);
                rebuild();
            }
        });

        let down = gtk::Button::builder()
            .icon_name("go-down-symbolic")
            .valign(gtk::Align::Center)
            .sensitive(index + 1 < total)
            .tooltip_text(tr("Ниже по приоритету", "Lower priority"))
            .css_classes(["flat"])
            .build();
        down.connect_clicked({
            let window = window.clone();
            let rebuild = rebuild.clone();
            move |_| {
                move_rule(&window, index, index + 1);
                rebuild();
            }
        });

        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text(tr("Удалить правило", "Delete rule"))
            .css_classes(["flat"])
            .build();
        remove.connect_clicked({
            let window = window.clone();
            let rebuild = rebuild.clone();
            move |_| {
                {
                    let mut settings = window.state().settings.borrow_mut();
                    if index < settings.route_rules.len() {
                        settings.route_rules.remove(index);
                    }
                }
                save(&window);
                rebuild();
            }
        });

        row.add_suffix(&toggle);
        row.add_suffix(&up);
        row.add_suffix(&down);
        row.add_suffix(&remove);
        row.connect_activated({
            let window = window.clone();
            let dialog = dialog.clone();
            let rebuild = rebuild.clone();
            move |_| edit_rule(&window, &dialog, Some(index), rebuild.clone())
        });

        group.add(&row);
    }
}

fn find_listbox_row(widget: gtk::Widget) -> Option<gtk::Widget> {
    // AdwPreferencesGroup stores rows inside a nested list; search there rather
    // than for the first descendant, or the group heading would be removed.
    let mut queue = vec![widget];
    while let Some(current) = queue.pop() {
        if current.is::<adw::ActionRow>() {
            return Some(current);
        }
        let mut child = current.first_child();
        while let Some(node) = child {
            child = node.next_sibling();
            queue.push(node);
        }
    }
    None
}

fn glib_propagation() -> gtk::glib::Propagation {
    gtk::glib::Propagation::Proceed
}

fn move_rule(window: &Rc<Window>, from: usize, to: usize) {
    {
        let mut settings = window.state().settings.borrow_mut();
        if from < settings.route_rules.len() && to < settings.route_rules.len() {
            settings.route_rules.swap(from, to);
        }
    }
    save(window);
}

fn save(window: &Rc<Window>) {
    if let Err(e) = window.state().save_settings() {
        window.toast(&crate::tr_format!(
            "Не удалось сохранить правила: {e}",
            "Could not save rules: {e}"
        ));
    }
}

/// Single-rule editor. `index` is a rule from the list; `None` is a new rule.
/// A rule may have multiple conditions: an exclusion list commonly mixes domains,
/// addresses, and processes, and separate rules would repeat the same action.
fn edit_rule(
    window: &Rc<Window>,
    parent: &adw::Dialog,
    index: Option<usize>,
    rebuild: Rc<dyn Fn()>,
) {
    let rule = index
        .and_then(|index| {
            window
                .state()
                .settings
                .borrow()
                .route_rules
                .get(index)
                .cloned()
        })
        .unwrap_or_default();

    let dialog = adw::Dialog::builder()
        .title(tr("Правило", "Rule"))
        .content_width(520)
        .content_height(620)
        .build();

    let header = adw::HeaderBar::new();
    let save_button = gtk::Button::builder()
        .label(tr("Сохранить", "Save"))
        .css_classes(["suggested-action"])
        .build();
    header.pack_end(&save_button);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();

    let name = adw::EntryRow::builder()
        .title(tr("Название", "Name"))
        .text(&rule.name)
        .build();
    group.add(&name);

    let actions: Vec<&str> = RouteAction::all()
        .iter()
        .map(|a| action_title(*a))
        .collect();
    let action = adw::ComboRow::builder()
        .title(tr("Действие", "Action"))
        .model(&gtk::StringList::new(&actions))
        .selected(
            RouteAction::all()
                .iter()
                .position(|a| *a == rule.action)
                .unwrap_or(0) as u32,
        )
        .build();
    group.add(&action);

    let modes: Vec<&str> = MatchMode::all().iter().map(|m| mode_title(*m)).collect();
    let mode = adw::ComboRow::builder()
        .title(tr("Когда срабатывает", "Match mode"))
        .subtitle(tr(
            "«Любое» — список исключений, «все» — совпадение по всем признакам сразу",
            "“Any” matches an exception list; “all” requires every condition to match",
        ))
        .model(&gtk::StringList::new(&modes))
        .selected(
            MatchMode::all()
                .iter()
                .position(|m| *m == rule.match_mode)
                .unwrap_or(0) as u32,
        )
        .build();
    group.add(&mode);
    page.add(&group);

    let add_condition = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(tr("Добавить условие", "Add condition"))
        .css_classes(["flat"])
        .build();
    let conditions_group = adw::PreferencesGroup::builder()
        .title(tr("Условия", "Conditions"))
        .description(tr(
            "Правило узнаёт соединение по этим признакам",
            "The rule identifies connections using these criteria",
        ))
        .header_suffix(&add_condition)
        .build();
    page.add(&conditions_group);

    let rows: Rc<RefCell<Vec<ConditionRow>>> = Rc::new(RefCell::new(Vec::new()));
    // A rule without conditions would catch all traffic, so a new rule starts
    // with something to work with.
    let initial = match rule.conditions.is_empty() {
        true => vec![Condition::default()],
        false => rule.conditions.clone(),
    };
    for condition in &initial {
        add_condition_row(&conditions_group, &rows, condition);
    }

    // Choosing between “any” and “all” makes sense only with multiple conditions.
    let sync_mode = {
        let mode = mode.clone();
        let rows = rows.clone();
        Rc::new(move || mode.set_visible(rows.borrow().len() > 1))
    };
    sync_mode();

    add_condition.connect_clicked({
        let conditions_group = conditions_group.clone();
        let rows = rows.clone();
        let sync_mode = sync_mode.clone();
        move |_| {
            add_condition_row(&conditions_group, &rows, &Condition::default());
            sync_mode();
        }
    });

    let toolbar = adw::ToolbarView::builder().content(&page).build();
    toolbar.add_top_bar(&header);
    dialog.set_child(Some(&toolbar));

    save_button.connect_clicked({
        let window = window.clone();
        let dialog = dialog.clone();
        let rows = rows.clone();
        let name = name.clone();
        let action = action.clone();
        let mode = mode.clone();
        move |_| {
            let conditions: Vec<Condition> = rows
                .borrow()
                .iter()
                .map(|row| row.read())
                .filter(|condition| !condition.values.is_empty())
                .collect();

            if conditions.is_empty() {
                window.toast(tr(
                    "Впишите хотя бы одно значение",
                    "Enter at least one value",
                ));
                return;
            }

            let edited = RouteRule {
                name: name.text().trim().to_string(),
                enabled: true,
                conditions,
                match_mode: MatchMode::all()[mode.selected() as usize],
                action: RouteAction::all()[action.selected() as usize],
            };
            {
                let mut settings = window.state().settings.borrow_mut();
                match index {
                    Some(index) if index < settings.route_rules.len() => {
                        // The switch state controls the list, not the editor.
                        let enabled = settings.route_rules[index].enabled;
                        settings.route_rules[index] = RouteRule { enabled, ..edited };
                    }
                    _ => settings.route_rules.push(edited),
                }
            }
            save(&window);
            rebuild();
            dialog.close();

            if window.state().is_connected() {
                window.toast(tr(
                    "Правило сохранено — применится при следующем подключении",
                    "Rule saved — applies on the next connection",
                ));
            }
        }
    });

    dialog.present(Some(parent));
}

/// One editor condition: its type and values.
struct ConditionRow {
    row: adw::ExpanderRow,
    kind: adw::ComboRow,
    view: gtk::TextView,
}

impl ConditionRow {
    /// Condition in the form currently shown by the editor.
    fn read(&self) -> Condition {
        let buffer = self.view.buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        let values = text
            .lines()
            // Accept values both line by line and comma-separated: they are often
            // copied from other configs with different formatting.
            .flat_map(|line| line.split(','))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect();
        Condition::new(MatchKind::all()[self.kind.selected() as usize], values)
    }
}

fn add_condition_row(
    group: &adw::PreferencesGroup,
    rows: &Rc<RefCell<Vec<ConditionRow>>>,
    condition: &Condition,
) {
    let row = adw::ExpanderRow::builder()
        .title(kind_title(condition.kind))
        .subtitle(kind_hint(condition.kind))
        .expanded(true)
        .build();

    let kinds: Vec<&str> = MatchKind::all().iter().map(|k| kind_title(*k)).collect();
    let kind = adw::ComboRow::builder()
        .title(tr("Условие", "Condition"))
        .model(&gtk::StringList::new(&kinds))
        .selected(
            MatchKind::all()
                .iter()
                .position(|k| *k == condition.kind)
                .unwrap_or(0) as u32,
        )
        .build();
    row.add_row(&kind);

    let view = gtk::TextView::builder()
        .monospace(true)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(10)
        .right_margin(10)
        .build();
    view.buffer().set_text(&condition.values.join("\n"));
    let scroller = gtk::ScrolledWindow::builder()
        .height_request(140)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .child(&view)
        .css_classes(["card"])
        .build();
    row.add_row(&scroller);

    let remove = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .valign(gtk::Align::Center)
        .tooltip_text(tr("Убрать условие", "Remove condition"))
        .css_classes(["flat"])
        .build();
    row.add_suffix(&remove);

    // The title must follow the selected condition: process and subnet values
    // are completely different.
    kind.connect_selected_notify({
        let row = row.clone();
        move |combo| {
            let selected = MatchKind::all()[combo.selected() as usize];
            row.set_title(kind_title(selected));
            row.set_subtitle(kind_hint(selected));
        }
    });

    remove.connect_clicked({
        let group = group.clone();
        let rows = rows.clone();
        let row = row.clone();
        move |_| {
            // Do not remove the last condition: a rule without conditions would
            // catch all traffic, while an empty form explains nothing.
            if rows.borrow().len() < 2 {
                return;
            }
            if let Some(at) = rows.borrow().iter().position(|item| item.row == row) {
                rows.borrow_mut().remove(at);
            }
            group.remove(&row);
        }
    });

    group.add(&row);
    rows.borrow_mut().push(ConditionRow { row, kind, view });
}
