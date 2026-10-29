//! The tray icon: Amber's home while no window is open.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let dictate = MenuItem::with_id(app, "dictate", "Start / stop dictation", true, None::<&str>)?;
    let bubble = MenuItem::with_id(app, "bubble", "Show / hide bubble", true, None::<&str>)?;
    let copy_last = MenuItem::with_id(app, "copy-last", "Copy last dictation", true, None::<&str>)?;
    let again = MenuItem::with_id(app, "again", "Recognize again", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "Check for updates", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Amber", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&dictate, &bubble, &sep, &copy_last, &again, &sep3, &settings, &update, &sep2, &quit])?;

    let mut tray = TrayIconBuilder::with_id("amber")
        .tooltip("Amber - dictation")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "dictate" => crate::dictation::toggle(app),
            "bubble" => crate::hotkeys::toggle_bubble(app),
            "copy-last" => copy_last_dictation(app),
            "again" => {
                // Recognition may take a while; the menu must not wait for it.
                let app = app.clone();
                std::thread::spawn(move || match crate::dictation::recognize_again(&app) {
                    Ok(_) => crate::notify(&app, "Amber", "Recognized again - the text is on the clipboard."),
                    Err(e) => crate::notify(&app, "Amber", &e),
                });
            }
            "settings" => crate::bubble::open_settings(app, None),
            "update" => crate::bubble::open_settings(app, Some("app")),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                crate::bubble::open_settings(tray.app_handle(), None);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn copy_last_dictation(app: &AppHandle) {
    use tauri::Manager;
    let latest = app.state::<crate::state::Core>().memory.latest();
    match latest {
        Some(entry) => match crate::insert::to_clipboard(&entry.text) {
            Ok(()) => crate::notify(app, "Amber", "The last dictation is on the clipboard."),
            Err(e) => crate::notify(app, "Amber", &format!("Could not copy: {e}")),
        },
        None => crate::notify(app, "Amber", "No dictation in the history yet."),
    }
}
