//! Putting the text where the cursor is. The window that had focus when the dictation
//! started is remembered; if another program is in front by the time the text is ready, the
//! text goes on the clipboard instead of into the wrong place. Otherwise it is pasted
//! (clipboard + Ctrl+V) or typed (Unicode keystrokes).

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
    use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL,
        VK_RETURN, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_V,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow};

    /// The program file of a process, lowercase: "discord.exe".
    fn exe_of_pid(pid: u32) -> Option<String> {
        use windows::Win32::System::Threading::{QueryFullProcessImageNameW, PROCESS_NAME_WIN32};
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len) };
        unsafe {
            let _ = CloseHandle(process);
        }
        ok.ok()?;
        Some(amber_core::profiles::exe_name(&String::from_utf16_lossy(&buf[..len as usize])))
    }

    fn pid_of(hwnd: HWND) -> u32 {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        pid
    }

    /// The program a window belongs to. Store apps (WhatsApp, Mail ...) live in a frame owned
    /// by ApplicationFrameHost.exe; their own process is the child window of another process.
    pub fn program_of_window(hwnd: isize) -> Option<String> {
        use windows::Win32::Foundation::{LPARAM, TRUE};
        use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;
        let hwnd = HWND(hwnd as *mut _);
        let frame_pid = pid_of(hwnd);
        let exe = exe_of_pid(frame_pid)?;
        if exe != "applicationframehost.exe" {
            return Some(exe);
        }
        unsafe extern "system" fn each(child: HWND, data: LPARAM) -> windows::core::BOOL {
            let (frame_pid, found) = &mut *(data.0 as *mut (u32, u32));
            let pid = pid_of(child);
            if pid != *frame_pid && pid != 0 {
                *found = pid;
                return false.into();
            }
            TRUE
        }
        let mut state = (frame_pid, 0u32);
        unsafe {
            let _ = EnumChildWindows(Some(hwnd), Some(each), LPARAM(&mut state as *mut _ as isize));
        }
        if state.1 != 0 {
            exe_of_pid(state.1)
        } else {
            Some(exe)
        }
    }

    /// Programs with a visible window right now, for picking one in the settings.
    pub fn running_programs() -> Vec<(String, String)> {
        use windows::Win32::Foundation::{LPARAM, TRUE};
        use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible};
        unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
            let list = &mut *(data.0 as *mut Vec<isize>);
            if IsWindowVisible(hwnd).as_bool() && GetWindowTextLengthW(hwnd) > 0 {
                list.push(hwnd.0 as isize);
            }
            TRUE
        }
        let mut windows: Vec<isize> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut windows as *mut _ as isize));
        }
        let own = unsafe { GetCurrentProcessId() };
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for h in windows {
            let hwnd = HWND(h as *mut _);
            if pid_of(hwnd) == own {
                continue;
            }
            let Some(exe) = program_of_window(h) else { continue };
            if !seen.insert(exe.clone()) || exe == "explorer.exe" || exe == "textinputhost.exe" {
                continue;
            }
            let mut title = [0u16; 256];
            let n = unsafe { GetWindowTextW(hwnd, &mut title) };
            out.push((exe, String::from_utf16_lossy(&title[..n.max(0) as usize])));
        }
        out
    }

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

    /// Whether the window in front is still the one the dictation was meant for - or another
    /// window of the same program (a popup, a second browser window). If the user moved on to
    /// another program while Amber was recognizing, nothing is typed there.
    fn still_in_front(target: Target) -> bool {
        let hwnd = HWND(target.hwnd as *mut _);
        let mut fg = unsafe { GetForegroundWindow() };
        // Amber itself in front (its tray menu, used to stop the dictation): it may hand the
        // focus back, Windows allows that to the program in front.
        if pid_of(fg) == unsafe { GetCurrentProcessId() } && !is_settings_window(fg) {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            std::thread::sleep(Duration::from_millis(60));
            fg = unsafe { GetForegroundWindow() };
        }
        if fg == hwnd {
            return true;
        }
        let (a, b) = (pid_of(fg), pid_of(hwnd));
        if a == 0 || a != b {
            return false;
        }
        // Store apps all live in ApplicationFrameHost's frames, and Explorer windows, the
        // taskbar and the desktop share one process: there only the same window counts.
        !matches!(exe_of_pid(a).as_deref(), Some("applicationframehost.exe" | "explorer.exe"))
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
    /// reports success anyway. Checked up front instead. An elevated process does not let a
    /// normal one read its token, so "cannot tell" counts as blocked as long as Amber itself
    /// is not elevated.
    fn blocked_by_uipi(target: Option<Target>) -> bool {
        if elevated(unsafe { GetCurrentProcess() }) == Some(true) {
            return false;
        }
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
        theirs != Some(false)
    }

    fn set_clipboard(cb: &mut arboard::Clipboard, text: &str) -> Result<(), String> {
        use arboard::SetExtWindows;
        // Kept out of Windows' clipboard history (Win+V) and cloud clipboard sync.
        cb.set().exclude_from_history().text(text.to_string()).map_err(|e| e.to_string())
    }

    fn paste(text: &str, settings: &InsertSettings) -> Result<(), String> {
        let mut cb = arboard::Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?;
        let previous = if settings.restore_clipboard { cb.get_text().ok() } else { None };
        // Windows' own line ending, which older programs need to see a line break at all.
        set_clipboard(&mut cb, &text.replace("\r\n", "\n").replace('\n', "\r\n"))?;
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
        // A line break as Shift+Enter: a new line in an editor, and no "send" in a chat.
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                send(&[key(VK_SHIFT, false), key(VK_RETURN, false), key(VK_RETURN, true), key(VK_SHIFT, true)]);
                std::thread::sleep(Duration::from_millis(8));
            }
            let units: Vec<u16> = line.encode_utf16().collect();
            // In batches, so a slow target's input queue keeps up.
            for chunk in units.chunks(64) {
                let inputs: Vec<INPUT> = chunk.iter().flat_map(|&u| [unicode(u, false), unicode(u, true)]).collect();
                send(&inputs);
                std::thread::sleep(Duration::from_millis(8));
            }
        }
    }

    /// Puts [text] on the clipboard (kept out of the clipboard history), for "Copy" and
    /// "Recognize again".
    pub fn to_clipboard(text: &str) -> Result<(), String> {
        let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        set_clipboard(&mut cb, &text.replace("\r\n", "\n").replace('\n', "\r\n"))
    }

    pub fn insert(text: &str, settings: &InsertSettings, target: Option<Target>) -> Result<Outcome, String> {
        if blocked_by_uipi(target) {
            let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            set_clipboard(&mut cb, text)?;
            return Ok(Outcome::OnClipboard(
                "Windows blocks typing into this window (it probably runs as administrator). The text is on the clipboard - press Ctrl+V.".into(),
            ));
        }
        if target.is_some_and(|t| !still_in_front(t)) {
            let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            set_clipboard(&mut cb, text)?;
            return Ok(Outcome::OnClipboard("You switched to another window, so the text is on the clipboard - press Ctrl+V where you want it.".into()));
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
pub use win::{insert, program_of_window, remember, running_programs, to_clipboard};

impl Target {
    /// The target's program, lowercase ("discord.exe"), for its profile.
    pub fn program(&self) -> Option<String> {
        #[cfg(windows)]
        return program_of_window(self.hwnd);
        #[cfg(not(windows))]
        None
    }
}

#[cfg(not(windows))]
pub fn running_programs() -> Vec<(String, String)> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn to_clipboard(_text: &str) -> Result<(), String> {
    Err("The clipboard is only implemented on Windows.".into())
}

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
