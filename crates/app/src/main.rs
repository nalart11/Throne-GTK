//! Throne GTK — a proxy client for sing-box and Xray with a GTK4 interface.

mod engine;
mod format;
mod logbridge;
mod paths;
mod state;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(target_os = "macos")]
#[path = "tray_macos.rs"]
mod tray;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[path = "tray_stub.rs"]
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
    // Create the event channel before the log subscriber: lines written during
    // startup must reach the window log rather than disappear.
    let (events_tx, events_rx) = async_channel::unbounded::<engine::Event>();

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(logbridge::LogBridge::new(events_tx.clone()))
        .init();

    // Start tray setup before creating the window. Linux registers it from a
    // worker thread; macOS completes it on the first main-loop iteration.
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
            // A second activate call (launching from the application list while
            // the program is already running) must not create a second window
            // and a second core. Search among all windows, not just active ones:
            // a window hidden in the tray is not considered active.
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

/// Shut down completely. Terminate the core ourselves instead of relying on it
/// to notice its parent’s death: otherwise the TUN interface outlives the app.
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

    // The only event consumer: the interface is updated only here.
    glib::spawn_future_local({
        let window = window.clone();
        let tray = tray.clone();
        async move {
            while let Ok(event) = events_rx.recv().await {
                // The icon shows the state in its tooltip: with the window hidden,
                // this is the only way to see whether the connection is working.
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

    // Tray menu actions arrive here through the platform-independent channel.
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
            // The icon is available, so hide the window in it: the connection
            // keeps working, and a click on the icon restores the window.
            if state.settings.borrow().close_to_tray && tray.is_alive() {
                let _ = state.save_settings();
                window.set_visible(false);
                return glib::Propagation::Stop;
            }
            shutdown(&state);
            glib::Propagation::Proceed
        }
    });

    // Auto-selection is also a “previous selection,” even though it has no profile.
    let has_selection = state.selected_profile().is_some() || state.auto_selected().is_some();
    if state.settings.borrow().connect_last_on_start && has_selection {
        state.connect_selected();
    }

    // On the first run with Throne already installed, offer to import servers
    // immediately instead of leaving the user with an empty list and a menu
    // they would first have to think to open.
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
                // The icon does not register with the panel immediately. “Hidden”
                // means “in the icon,” so without an icon the window must still
                // be shown — otherwise there would be no way to access the app.
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
        // Ctrl+Q does not go through window closing, so stop the core here too
        // — otherwise the tunnel outlives the app.
        move |_, _| {
            shutdown(window.state());
            app.quit()
        }
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Control>q"]);

    // Buttons on the servers page do the same as the menu items.
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

    // Button shapes take precedence over the user theme: it makes everything
    // into capsules, while this window’s grid relies on rectangles. The theme
    // still controls all other styling — this provider only affects rounding.
    let shape = gtk::CssProvider::new();
    shape.load_from_string(include_str!("../resources/shape.css"));
    gtk::style_context_add_provider_for_display(
        &display,
        &shape,
        gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
    );
}

/// If even the storage could not be initialized, show a window explaining why:
/// silently exiting leaves the user with nothing but an empty screen.
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
