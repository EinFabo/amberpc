//! Amber for Windows: dictation into any window, locally or through a server.

mod autostart;
mod bubble;
mod capture;
mod commands;
mod diag;
mod dictation;
mod hotkeys;
mod insert;
mod secrets;
mod state;
mod tray;
mod updates;
mod whisper;

use amber_core::local::Layout;
use amber_core::settings::EngineKind;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use state::{Core, Paths};

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let config = app.path().app_config_dir()?;
    let data = app.path().app_local_data_dir()?;
    let paths = Paths {
        settings: config.join("settings.json"),
        secrets: config.join("secrets.bin"),
        logs: data.join("logs"),
        layout: Layout { root: data },
    };
    diag::init(&paths.logs);
    diag::info(format!("Amber {} starting", env!("CARGO_PKG_VERSION")));
    app.manage(Core::new(paths));

    let core = app.state::<Core>();
    let settings = core.settings();
    bubble::create(app)?;
    tray::create(app)?;
    if let Err(e) = hotkeys::register(app, &settings) {
        diag::warn(format!("hotkey: {e}"));
        notify(app, "Amber: hotkey unavailable", &e);
    }
    autostart::sync_at_start(app);

    if settings.engine == EngineKind::Local && settings.local.keep_loaded {
        let app = app.clone();
        std::thread::spawn(move || {
            let _ = app.state::<Core>().whisper.ensure(&app);
        });
    }

    // Started by Windows at login: stay in the tray.
    let autostarted = std::env::args().any(|a| a == amber_core::autostart::ARG);
    if !settings.onboarded {
        bubble::open_settings(app, Some("welcome"));
    } else if !autostarted {
        bubble::open_settings(app, None);
    }
    updates::spawn(app);
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        // First: a second start only brings the running Amber forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| bubble::open_settings(app, None)))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| setup(app.handle()))
        .invoke_handler(tauri::generate_handler![
            commands::get_overview,
            commands::save_settings,
            commands::set_api_key,
            commands::test_server,
            commands::list_microphones,
            commands::mic_test,
            commands::local_info,
            commands::download,
            commands::cancel_download,
            commands::remove_download,
            commands::start_local,
            commands::bubble_pointer,
            commands::bubble_config,
            commands::open_settings,
            commands::toggle_dictation,
            commands::get_log,
            commands::check_update,
            commands::install_update,
            commands::open_url,
        ])
        .build(tauri::generate_context!())
        .expect("Amber failed to start")
        .run(|_app, event| {
            // Closing the settings window keeps Amber running in the tray; only "Quit" ends it.
            if let tauri::RunEvent::ExitRequested { api, code: None, .. } = event {
                api.prevent_exit();
            }
        });
}
