//! Starting with Windows, written by hand instead of through a plugin: the path is quoted
//! (user folders with spaces), and the switch in Task Manager's "Startup apps" is respected
//! instead of being turned back on at every start.

use amber_core::autostart::{disabled_in_task_manager, run_command, APPROVED_KEY, RUN_KEY, VALUE_NAME};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::Core;

/// Debug builds and release builds run straight from the build folder (`target\release`)
/// must not point Windows' autostart at themselves - only the installed app registers.
pub fn is_installed() -> bool {
    !cfg!(debug_assertions)
        && std::env::current_exe().map(|p| !p.to_string_lossy().to_lowercase().contains(r"\target\")).unwrap_or(false)
}

/// At start: Task Manager's switch wins over the setting; otherwise the registry is made to
/// match the setting (which also moves an old entry to the current install path).
pub fn sync_at_start(app: &AppHandle) {
    if !is_installed() {
        return;
    }
    let core = app.state::<Core>();
    if disabled_in_task_manager(reg::approved().as_deref()) {
        if core.settings().autostart {
            crate::diag::info("autostart was turned off in Task Manager");
            let after = core.update_settings(|s| s.autostart = false);
            let _ = app.emit("settings-changed", after);
        }
        return;
    }
    set(core.settings().autostart);
}

/// The user flipped the switch in Amber's settings.
pub fn set(on: bool) {
    if !is_installed() {
        return;
    }
    let result = if on {
        std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| reg::enable(&run_command(&exe.to_string_lossy())))
    } else {
        reg::disable()
    };
    if let Err(e) = result {
        crate::diag::warn(format!("autostart not changed: {e}"));
    }
}

#[cfg(windows)]
mod reg {
    use super::*;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY,
    };

    fn check(e: WIN32_ERROR) -> Result<(), String> {
        if e == ERROR_SUCCESS || e == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("registry error {}", e.0))
        }
    }

    pub fn approved() -> Option<Vec<u8>> {
        let mut buf = [0u8; 64];
        let mut len = buf.len() as u32;
        let e = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(APPROVED_KEY),
                &HSTRING::from(VALUE_NAME),
                RRF_RT_REG_BINARY,
                None,
                Some(buf.as_mut_ptr() as *mut _),
                Some(&mut len),
            )
        };
        (e == ERROR_SUCCESS).then(|| buf[..len as usize].to_vec())
    }

    pub fn enable(command: &str) -> Result<(), String> {
        let data: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
        check(unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN_KEY),
                &HSTRING::from(VALUE_NAME),
                REG_SZ.0,
                Some(data.as_ptr() as *const _),
                (data.len() * 2) as u32,
            )
        })?;
        // Turning it on in Amber also clears an old "disabled" from Task Manager.
        check(unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(APPROVED_KEY), &HSTRING::from(VALUE_NAME)) })
    }

    pub fn disable() -> Result<(), String> {
        check(unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(VALUE_NAME)) })?;
        check(unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(APPROVED_KEY), &HSTRING::from(VALUE_NAME)) })
    }
}

#[cfg(not(windows))]
mod reg {
    pub fn approved() -> Option<Vec<u8>> {
        None
    }
    pub fn enable(_command: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn disable() -> Result<(), String> {
        Ok(())
    }
}
