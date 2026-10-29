//! Putting the text where the cursor is. The window that had focus when the dictation
//! started is remembered and gets focus back first; then the text is pasted (clipboard +
//! Ctrl+V) or typed (Unicode keystrokes).

use amber_core::settings::{InsertMethod, InsertSettings};

/// The window the text is meant for.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: isize,
}

#[derive(Debug)]
pub enum Outcome {
    Pasted,
    Typed,
    /// Windows would swallow the keystrokes (an elevated window); the text is on the
    /// clipboard instead.
    OnClipboard(String),
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{
        AttachThreadInput, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, OpenProcessToken,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL,
        VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_V,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
    };

    pub fn remember() -> Option<Target> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        // Our own windows (settings, bubble) are no target - except the settings window's test
        // field, which is why the settings window is a valid target after all.
        if pid == unsafe { GetCurrentProcessId() } && !is_settings_window(hwnd) {
            return None;
        }
        Some(Target { hwnd: hwnd.0 as isize })
    }

    fn is_settings_window(hwnd: HWND) -> bool {
        crate::bubble::settings_hwnd().map(|h| h == hwnd.0 as isize).unwrap_or(false)
    }

    /// Gives focus back to [target] if something else took it. Windows only lets the
    /// foreground process hand out focus; attaching to its input queue for a moment counts.
    fn refocus(target: Target) {
        let hwnd = HWND(target.hwnd as *mut _);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return;
            }
            let fg = GetForegroundWindow();
            if fg == hwnd {
                return;
            }
            let fg_thread = GetWindowThreadProcessId(fg, None);
            let me = GetCurrentThreadId();
            let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
            let _ = SetForegroundWindow(hwnd);
            if attached {
                let _ = AttachThreadInput(me, fg_thread, false);
            }
        }
        std::thread::sleep(Duration::from_millis(60));
    }

    const MODIFIERS: [VIRTUAL_KEY; 11] =
        [VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_MENU, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN];

    fn down(vk: VIRTUAL_KEY) -> bool {
        (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000) != 0
    }

    /// The hotkey's modifiers may still be held - Ctrl+Alt+V is not a paste, and Alt+letters
    /// open menus. Waits for them to come up; past the deadline they are released for the user.
    fn wait_for_modifiers() {
        let deadline = Instant::now() + Duration::from_millis(1500);
        while MODIFIERS.iter().any(|&vk| down(vk)) {
            if Instant::now() > deadline {
                let held: Vec<INPUT> = MODIFIERS.iter().filter(|&&vk| down(vk)).map(|&vk| key(vk, true)).collect();
                crate::diag::warn(format!("{} modifier key(s) still held, released for the insert", held.len()));
                send(&held);
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    fn unicode(unit: u16, up: bool) -> INPUT {
        let flags = if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE };
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        }
    }

    fn send(inputs: &[INPUT]) -> u32 {
        if inputs.is_empty() {
            return 0;
        }
        unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) }
    }

    fn elevated(process: HANDLE) -> Option<bool> {
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
            let mut elevation = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            );
            let _ = CloseHandle(token);
            ok.ok()?;
            Some(elevation.TokenIsElevated != 0)
        }
    }

    /// Windows drops synthetic input aimed at a process running with more rights (UIPI) and
    /// reports success anyway. Checked up front instead.
    fn blocked_by_uipi(target: Option<Target>) -> bool {
        let hwnd = match target {
            Some(t) => HWND(t.hwnd as *mut _),
            None => unsafe { GetForegroundWindow() },
        };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 {
            return false;
        }
        let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return false };
        let theirs = elevated(process);
        unsafe {
            let _ = CloseHandle(process);
        }
        let ours = elevated(unsafe { GetCurrentProcess() });
        theirs == Some(true) && ours == Some(false)
    }

    fn set_clipboard(cb: &mut arboard::Clipboard, text: &str) -> Result<(), String> {
        use arboard::SetExtWindows;
        // Kept out of Windows' clipboard history (Win+V) and cloud clipboard sync.
        cb.set().exclude_from_history().text(text.to_string()).map_err(|e| e.to_string())
    }

    fn paste(text: &str, settings: &InsertSettings) -> Result<(), String> {
        let mut cb = arboard::Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?;
        let previous = if settings.restore_clipboard { cb.get_text().ok() } else { None };
        set_clipboard(&mut cb, text)?;
        std::thread::sleep(Duration::from_millis(30));
        send(&[key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)]);
        if let Some(previous) = previous {
            // The target reads the clipboard asynchronously after Ctrl+V; restoring too early
            // would paste the old content.
            std::thread::sleep(Duration::from_millis(400));
            let _ = set_clipboard(&mut cb, &previous);
        }
        Ok(())
    }

    fn type_text(text: &str) {
        let units: Vec<u16> = text.encode_utf16().collect();
        // In batches, so a slow target's input queue keeps up.
        for chunk in units.chunks(64) {
            let inputs: Vec<INPUT> = chunk.iter().flat_map(|&u| [unicode(u, false), unicode(u, true)]).collect();
            send(&inputs);
            std::thread::sleep(Duration::from_millis(8));
        }
    }

    pub fn insert(text: &str, settings: &InsertSettings, target: Option<Target>) -> Result<Outcome, String> {
        if blocked_by_uipi(target) {
            let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            set_clipboard(&mut cb, text)?;
            return Ok(Outcome::OnClipboard(
                "This window runs as administrator, so Windows blocks typing into it. The text is on the clipboard - press Ctrl+V.".into(),
            ));
        }
        if let Some(t) = target {
            refocus(t);
        }
        wait_for_modifiers();
        match settings.method {
            InsertMethod::Paste => paste(text, settings).map(|_| Outcome::Pasted),
            InsertMethod::Type => {
                type_text(text);
                Ok(Outcome::Typed)
            }
        }
    }
}

#[cfg(windows)]
pub use win::{insert, remember};

#[cfg(not(windows))]
pub fn remember() -> Option<Target> {
    None
}

#[cfg(not(windows))]
pub fn insert(text: &str, _settings: &InsertSettings, _target: Option<Target>) -> Result<Outcome, String> {
    let _ = text;
    let _ = InsertMethod::Paste;
    Err("Inserting is only implemented on Windows.".into())
}
