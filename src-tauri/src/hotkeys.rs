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
    let command = settings.terminal.hotkey.trim();
    if settings.terminal.enabled && !command.is_empty() && (command.eq_ignore_ascii_case(dictate) || command.eq_ignore_ascii_case(toggle)) {
        problems.push(format!("{command} is already one of Amber's hotkeys"));
    } else if settings.terminal.enabled && !command.is_empty() {
        match command.parse::<Shortcut>() {
            Ok(sc) => {
                if let Err(e) = gs.on_shortcut(sc, |app, _, event| on_command(app, event.state)) {
                    problems.push(format!("{command} is taken by another app ({e})"));
                }
            }
            Err(e) => problems.push(format!("{command} is not a valid hotkey ({e})")),
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// The command hotkey: like the dictation hotkey (hold or toggle), but what is said becomes a
/// terminal command wherever it goes.
fn on_command(app: &AppHandle, state: ShortcutState) {
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
    match action {
        Action::Start => crate::dictation::start_as(app, true),
        other => apply(app, other),
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
