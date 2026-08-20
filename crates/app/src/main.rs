//! Throne GTK — клиент прокси на sing-box и Xray с интерфейсом на GTK4.

mod engine;
mod format;
mod logbridge;
mod paths;
mod state;
mod tray;
mod ui;

use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
use gtk::glib;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use engine::{Command, Engine, Status};
use state::State;
use throne_store::Store;

const APP_ID: &str = "dev.nalart.ThroneGtk";

fn main() -> glib::ExitCode {
    // Канал событий заводим до подписчика логов: строки, записанные при
    // старте, должны попасть в журнал окна, а не в никуда.
    let (events_tx, events_rx) = async_channel::unbounded::<engine::Event>();

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(logbridge::LogBridge::new(events_tx.clone()))
        .init();

    // Значок в трее заводим до окна: он живёт в своём потоке и к моменту
    // первого закрытия окна уже успевает зарегистрироваться в панели.
    let (actions_tx, actions_rx) = async_channel::unbounded::<tray::Action>();
    let tray = tray::start(actions_tx);

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::default())
        .build();

    app.connect_startup(|_| load_styles());
    app.connect_activate({
        let events_tx = events_tx.clone();
        let events_rx = events_rx.clone();
        let actions_rx = actions_rx.clone();
        let tray = tray.clone();
        move |app| {
            // Второй вызов activate (запуск из списка программ при уже
            // работающей программе) не должен поднимать второе окно и второе
            // ядро. Окно ищем среди всех, а не только активных: спрятанное в
            // трей окно активным не считается.
            if let Some(window) = app.windows().first() {
                window.set_visible(true);
                window.present();
                return;
            }
            if let Err(e) = build(
                app,
                events_tx.clone(),
                events_rx.clone(),
                actions_rx.clone(),
                tray.clone(),
            ) {
                tracing::error!("не удалось запустить приложение: {e:#}");
                show_fatal(app, &format!("{e:#}"));
            }
        }
    });

    app.run()
}

/// Выключение целиком. Ядро завершаем сами, не полагаясь на то, что оно
/// заметит смерть родителя: иначе TUN-интерфейс переживёт закрытие программы.
fn shutdown(state: &Rc<State>) {
    let _ = state.save_settings();
    state.engine.send(Command::Shutdown);
}

fn build(
    app: &adw::Application,
    events_tx: async_channel::Sender<engine::Event>,
    events_rx: async_channel::Receiver<engine::Event>,
    actions_rx: async_channel::Receiver<tray::Action>,
    tray: tray::Tray,
) -> anyhow::Result<()> {
    let store = Store::open(&paths::database())?;
    let core_bin = paths::core_binary();
    let engine = Engine::start(core_bin.clone(), paths::runtime_dir(), events_tx);
    let cache_file = paths::config_dir().join("cache.db");
    let state = State::new(store, engine, cache_file.to_string_lossy().into_owned())?;

    let window = ui::Window::new(app, state.clone());
    register_actions(app, &window);

    if !core_bin.exists() {
        window.toast("Ядро не найдено — соберите его через just build");
        tracing::error!("ядро не найдено: {}", core_bin.display());
    }

    // Единственный потребитель событий: интерфейс обновляется только здесь.
    glib::spawn_future_local({
        let window = window.clone();
        let tray = tray.clone();
        async move {
            while let Ok(event) = events_rx.recv().await {
                // Значок показывает состояние в подсказке: со спрятанным окном
                // это единственный способ увидеть, работает ли соединение.
                if let engine::Event::Status(status) = &event {
                    tray.set_connected(match status {
                        Status::Connected { name, .. } => Some(name.clone()),
                        _ => None,
                    });
                }
                window.apply_event(event);
            }
        }
    });

    // Щелчки по значку приходят из его потока сюда.
    glib::spawn_future_local({
        let window = window.clone();
        let state = state.clone();
        let app = app.clone();
        async move {
            while let Ok(action) = actions_rx.recv().await {
                match action {
                    tray::Action::Show => {
                        window.root.set_visible(true);
                        window.root.present();
                    }
                    tray::Action::Quit => {
                        shutdown(&state);
                        app.quit();
                    }
                }
            }
        }
    });

    window.root.connect_close_request({
        let state = state.clone();
        let tray = tray.clone();
        move |window| {
            // Значок на месте — прячемся в него: соединение продолжает
            // работать, окно возвращается щелчком по значку.
            if state.settings.borrow().close_to_tray && tray.is_alive() {
                let _ = state.save_settings();
                window.set_visible(false);
                return glib::Propagation::Stop;
            }
            shutdown(&state);
            glib::Propagation::Proceed
        }
    });

    // Автовыбор — тоже «прошлый выбор», хотя профиля за ним не стоит.
    let has_selection =
        state.selected_profile().is_some() || state.auto_selected().is_some();
    if state.settings.borrow().connect_last_on_start && has_selection {
        state.connect_selected();
    }

    // Первый запуск при уже установленном Throne: предлагаем перенести
    // серверы сразу, а не оставляем человека перед пустым списком с меню,
    // в которое ещё надо догадаться заглянуть.
    let empty = state
        .store
        .borrow()
        .profiles(None)
        .map(|p| p.is_empty())
        .unwrap_or(false);
    if empty && paths::throne_database().exists() {
        ui::dialogs::import_dialog(&window);
    }

    if state.settings.borrow().start_minimized {
        glib::spawn_future_local({
            let window = window.clone();
            let tray = tray.clone();
            async move {
                // Значок регистрируется в панели не мгновенно. «Свёрнутым»
                // значит «в значок», поэтому без значка окно всё же надо
                // показать — иначе программу неоткуда достать.
                glib::timeout_future(std::time::Duration::from_millis(1500)).await;
                if !tray.is_alive() {
                    window.root.present();
                }
            }
        });
    } else {
        window.root.present();
    }

    Ok(())
}

fn register_actions(app: &adw::Application, window: &Rc<ui::Window>) {
    let add = |name: &str, accels: &[&str], callback: Box<dyn Fn(&Rc<ui::Window>)>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate({
            let window = window.clone();
            move |_, _| callback(&window)
        });
        app.add_action(&action);
        if !accels.is_empty() {
            app.set_accels_for_action(&format!("app.{name}"), accels);
        }
    };

    add("add", &["<Control>n"], Box::new(ui::dialogs::add_dialog));
    add("test", &["<Control>t"], Box::new(|w| w.start_test()));
    add(
        "speed-test",
        &["<Control><Shift>t"],
        Box::new(|w| {
            let id = w.state().selected_id.get();
            if id == 0 {
                w.toast("Сначала выберите сервер");
            } else {
                w.start_speed_test(id);
            }
        }),
    );
    add(
        "update-subscription",
        &["<Control>r"],
        Box::new(ui::dialogs::update_subscription),
    );
    add("auto-recheck", &[], Box::new(|w| w.recheck_auto()));
    add("routes", &["<Control>m"], Box::new(ui::routes::open));
    add("import", &[], Box::new(ui::dialogs::import_dialog));
    add("check-config", &[], Box::new(ui::dialogs::check_config));
    add(
        "preferences",
        &["<Control>comma"],
        Box::new(|w| w.open_preferences()),
    );
    add("about", &[], Box::new(ui::dialogs::about));

    let quit = gio::SimpleAction::new("quit", None);
    quit.connect_activate({
        let app = app.clone();
        let window = window.clone();
        // Выход по Ctrl+Q не проходит через закрытие окна, так что ядро надо
        // погасить здесь же — иначе туннель переживёт программу.
        move |_, _| {
            shutdown(window.state());
            app.quit()
        }
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Control>q"]);

    // Кнопки на странице серверов делают то же, что пункты меню.
    window.servers_page().test_button().connect_clicked({
        let window = window.clone();
        move |_| window.start_test()
    });
    window.servers_page().update_button().connect_clicked({
        let window = window.clone();
        move |_| ui::dialogs::update_subscription(&window)
    });
}

fn load_styles() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };

    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../resources/style.css"));
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Форма кнопок идёт выше пользовательской темы: та задаёт капсулы всем
    // подряд, а сетка этого окна держится на прямоугольниках. Всё остальное
    // оформление тема по-прежнему перекрывает — этот провайдер трогает только
    // скругления.
    let shape = gtk::CssProvider::new();
    shape.load_from_string(include_str!("../resources/shape.css"));
    gtk::style_context_add_provider_for_display(
        &display,
        &shape,
        gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
    );
}

/// Если не поднялось даже хранилище, показать окно с причиной: молчаливый
/// выход не оставляет человеку ничего, кроме пустого экрана.
fn show_fatal(app: &adw::Application, message: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading("Не удалось запустить Throne GTK")
        .body(message)
        .build();
    dialog.add_response("quit", "Закрыть");

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(480)
        .default_height(240)
        .build();
    window.set_content(Some(&adw::ToolbarView::new()));
    window.present();

    dialog.connect_response(None, {
        let app = app.clone();
        move |_, _| app.quit()
    });
    dialog.present(Some(&window));
}
