//! Global hotkeys. Presses and releases go through the gesture machine, so one key can
//! toggle, talk while held, or both.

use amber_core::gesture::{Action, Gesture};
use amber_core::settings::{BubbleVisibility, Settings};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::state::Core;

/// Registers the hotkeys from [settings]. Returns what could not be registered, in words.
pub fn register(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    *app.state::<Core>().hotkey_gesture.lock().unwrap() = Gesture::new(settings.hotkey.mode);

    let mut problems = Vec::new();
    let dictate = settings.hotkey.dictate.trim();
    if !dictate.is_empty() {
        match dictate.parse::<Shortcut>() {
            Ok(sc) => {
                if let Err(e) = gs.on_shortcut(sc, |app, _, event| on_dictate(app, event.state)) {
                    problems.push(format!("{dictate} is taken by another app ({e})"));
                }
            }
            Err(e) => problems.push(format!("{dictate} is not a valid hotkey ({e})")),
        }
    }
    let toggle = settings.hotkey.toggle_bubble.trim();
    if !toggle.is_empty() && !toggle.eq_ignore_ascii_case(dictate) {
        match toggle.parse::<Shortcut>() {
            Ok(sc) => {
                let r = gs.on_shortcut(sc, |app, _, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_bubble(app);
                    }
                });
                if let Err(e) = r {
                    problems.push(format!("{toggle} is taken by another app ({e})"));
                }
            }
            Err(e) => problems.push(format!("{toggle} is not a valid hotkey ({e})")),
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

fn on_dictate(app: &AppHandle, state: ShortcutState) {
    let core = app.state::<Core>();
    let now = core.now_ms();
    let recording = core.dictation.is_recording();
    let action = {
        let mut g = core.hotkey_gesture.lock().unwrap();
        match state {
            ShortcutState::Pressed => g.press(now, recording),
            ShortcutState::Released => g.release(now, recording),
        }
    };
    apply(app, action);
}

pub fn apply(app: &AppHandle, action: Action) {
    match action {
        Action::Start => crate::dictation::start(app),
        Action::Stop => crate::dictation::stop(app),
        Action::Cancel => crate::dictation::cancel(app),
        Action::Nothing => {}
    }
}

/// Hidden <-> shown, kept in the settings so it survives a restart.
pub fn toggle_bubble(app: &AppHandle) {
    let core = app.state::<Core>();
    core.update_settings(|s| {
        s.bubble.visibility = match s.bubble.visibility {
            BubbleVisibility::Hidden => BubbleVisibility::Always,
            _ => BubbleVisibility::Hidden,
        }
    });
    crate::bubble::sync_visibility(app, core.dictation.is_active());
    let _ = tauri::Emitter::emit(app, "settings-changed", core.settings());
}
