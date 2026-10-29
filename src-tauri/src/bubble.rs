//! The two windows: the bubble at the screen edge and the settings window.
//!
//! The bubble must never take focus - a click on it would otherwise move the cursor out of
//! the text field the dictation is meant for. It is a tool window with `WS_EX_NOACTIVATE`,
//! shown with `SW_SHOWNOACTIVATE` and moved with `SWP_NOACTIVATE`, all through Win32
//! directly, in physical pixels, so DPI and several monitors line up.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use amber_core::settings::{BubbleVisibility, Edge};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::state::Core;

pub const BUBBLE: &str = "bubble";
pub const SETTINGS: &str = "settings";

/// Room around the disc for the fuse and the sparks, in logical pixels.
const PAD: u32 = 14;

static SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);
/// Out of the way for the program in front (full screen, or on the "hidden in" list).
static SUPPRESSED: AtomicBool = AtomicBool::new(false);
static DRAGGING: AtomicBool = AtomicBool::new(false);

pub fn settings_hwnd() -> Option<isize> {
    Some(SETTINGS_HWND.load(Ordering::Relaxed)).filter(|&h| h != 0)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BubbleConfig {
    pub opacity: f64,
    pub sounds: bool,
}

pub fn config(core: &Core) -> BubbleConfig {
    let s = core.settings();
    BubbleConfig { opacity: s.bubble.opacity, sounds: s.sounds }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let size = app.state::<Core>().settings().bubble.size + PAD * 2;
    let window = WebviewWindowBuilder::new(app, BUBBLE, WebviewUrl::App("bubble.html".into()))
        .title("Amber bubble")
        .inner_size(size as f64, size as f64)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        win::make_unfocusable(hwnd.0 as isize);
    }
    // Moved onto a monitor with another scale, the window is resized after the move; placing
    // it again keeps it from hanging over the right edge.
    let handle = app.clone();
    window.on_window_event(move |e| {
        if let tauri::WindowEvent::ScaleFactorChanged { .. } = e {
            if !DRAGGING.load(Ordering::SeqCst) {
                place(&handle);
            }
        }
    });
    place(app);
    sync_visibility(app, false);
    watch_foreground(app);
    Ok(())
}

/// Keeps the bubble away from full-screen programs and from the programs it is hidden in -
/// the hotkey and the Stream Deck keep working there.
fn watch_foreground(app: &AppHandle) {
    #[cfg(windows)]
    {
        let app = app.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(400));
            let core = app.state::<Core>();
            let b = core.settings().bubble;
            let fg = win::foreground();
            let suppress = fg.is_some_and(|fg| {
                (b.hide_fullscreen && win::is_fullscreen(fg))
                    || (!b.hidden_in.is_empty()
                        && crate::insert::program_of_window(fg).is_some_and(|exe| b.hidden_in.contains(&exe)))
            });
            if SUPPRESSED.swap(suppress, Ordering::Relaxed) != suppress {
                sync_visibility(&app, core.dictation.is_active());
            }
        });
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// Applies size, look and position after a settings change.
pub fn apply(app: &AppHandle) {
    let core = app.state::<Core>();
    if let Some(w) = app.get_webview_window(BUBBLE) {
        let size = core.settings().bubble.size + PAD * 2;
        let _ = w.set_size(tauri::LogicalSize::new(size as f64, size as f64));
    }
    let _ = app.emit_to(BUBBLE, "bubble-config", config(&core));
    place(app);
    sync_visibility(app, core.dictation.is_active());
}

/// Shows or hides the bubble for the current setting and whether a dictation runs.
pub fn sync_visibility(app: &AppHandle, dictating: bool) {
    let Some(w) = app.get_webview_window(BUBBLE) else { return };
    let show = match app.state::<Core>().settings().bubble.visibility {
        BubbleVisibility::Always => true,
        BubbleVisibility::WhileDictating => dictating,
        BubbleVisibility::Hidden => false,
    } && !SUPPRESSED.load(Ordering::Relaxed);
    #[cfg(windows)]
    if let Ok(hwnd) = w.hwnd() {
        win::show(hwnd.0 as isize, show);
        return;
    }
    let _ = if show { w.show() } else { w.hide() };
}

/// Puts the bubble at its edge, on its monitor, at its height.
pub fn place(app: &AppHandle) {
    #[cfg(windows)]
    {
        let Some(w) = app.get_webview_window(BUBBLE) else { return };
        let Ok(hwnd) = w.hwnd() else { return };
        let b = app.state::<Core>().settings().bubble;
        win::place(hwnd.0 as isize, &b.monitor, b.edge, b.y);
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// The press turned into a drag: follow the mouse until the button comes up, then snap to
/// the nearer edge and remember the place.
pub fn start_drag(app: &AppHandle) {
    #[cfg(windows)]
    {
        if DRAGGING.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(w) = app.get_webview_window(BUBBLE) else {
            DRAGGING.store(false, Ordering::SeqCst);
            return;
        };
        let Ok(hwnd) = w.hwnd() else {
            DRAGGING.store(false, Ordering::SeqCst);
            return;
        };
        let hwnd = hwnd.0 as isize;
        let app = app.clone();
        std::thread::spawn(move || {
            win::follow_mouse(hwnd);
            let (monitor, edge, y) = win::snap(hwnd);
            let core = app.state::<Core>();
            let after = core.update_settings(|s| {
                s.bubble.monitor = monitor;
                s.bubble.edge = edge;
                s.bubble.y = y;
            });
            DRAGGING.store(false, Ordering::SeqCst);
            place(&app);
            // The settings window holds a copy; without this its next save would put the
            // bubble back where it was.
            let _ = app.emit("settings-changed", after);
        });
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// Opens (or brings forward) the settings window, optionally on a page. A new window is
/// built on its own thread: building it inside a command or an event handler (tray menu,
/// second start) deadlocks WebView2 on Windows.
pub fn open_settings(app: &AppHandle, page: Option<&str>) {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if let Some(page) = page {
            let _ = app.emit_to(SETTINGS, "navigate", page);
        }
        return;
    }
    let url = match page {
        Some(p) => format!("settings.html?page={p}"),
        None => "settings.html".into(),
    };
    let app = app.clone();
    std::thread::spawn(move || build_settings(&app, url));
}

fn build_settings(app: &AppHandle, url: String) {
    static BUILDING: AtomicBool = AtomicBool::new(false);
    // Two quick clicks would otherwise race to create the same window.
    if BUILDING.swap(true, Ordering::SeqCst) {
        return;
    }
    if app.get_webview_window(SETTINGS).is_some() {
        BUILDING.store(false, Ordering::SeqCst);
        return;
    }
    let built = WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App(url.into()))
        .title("Amber")
        .inner_size(900.0, 660.0)
        .min_inner_size(760.0, 540.0)
        .center()
        .build();
    match built {
        Ok(w) => {
            #[cfg(windows)]
            if let Ok(hwnd) = w.hwnd() {
                SETTINGS_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
            }
            w.on_window_event(|e| {
                if let tauri::WindowEvent::Destroyed = e {
                    SETTINGS_HWND.store(0, Ordering::Relaxed);
                }
            });
        }
        Err(e) => crate::diag::error(format!("settings window failed: {e}")),
    }
    BUILDING.store(false, Ordering::SeqCst);
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::time::Duration;
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
        MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SM_SWAPBUTTON, SetWindowPos, ShowWindow, GWL_EXSTYLE,
        HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WS_EX_APPWINDOW,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    fn hwnd(h: isize) -> HWND {
        HWND(h as *mut _)
    }

    /// The window in front, unless it is the desktop or Amber's own.
    pub fn foreground() -> Option<isize> {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetShellWindow, GetWindowThreadProcessId};
        let fg = unsafe { GetForegroundWindow() };
        if fg.is_invalid() || fg == unsafe { GetShellWindow() } {
            return None;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
        (pid != std::process::id()).then_some(fg.0 as isize)
    }

    /// A window covering its whole monitor - a game, a video. The desktop's own windows
    /// (Progman, WorkerW) cover it too and do not count.
    pub fn is_fullscreen(h: isize) -> bool {
        use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
        use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
        let mut class = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd(h), &mut class) };
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        if class == "Progman" || class == "WorkerW" {
            return false;
        }
        let r = rect(h);
        let Some(m) = (unsafe { info_full(MonitorFromWindow(hwnd(h), MONITOR_DEFAULTTONEAREST)) }) else { return false };
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }

    /// The whole monitor, taskbar included (rcMonitor, not the work area).
    unsafe fn info_full(m: HMONITOR) -> Option<RECT> {
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(m, &mut info).as_bool().then_some(info.rcMonitor)
    }

    pub fn make_unfocusable(h: isize) {
        unsafe {
            let ex = GetWindowLongPtrW(hwnd(h), GWL_EXSTYLE);
            let ex = (ex | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
            SetWindowLongPtrW(hwnd(h), GWL_EXSTYLE, ex);
        }
    }

    pub fn show(h: isize, show: bool) {
        unsafe {
            if show {
                let _ = ShowWindow(hwnd(h), SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(hwnd(h), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE);
            } else {
                let _ = ShowWindow(hwnd(h), SW_HIDE);
            }
        }
    }

    struct Monitor {
        name: String,
        work: RECT,
    }

    fn info(m: HMONITOR) -> Option<Monitor> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        unsafe { GetMonitorInfoW(m, &mut info as *mut _ as *mut MONITORINFO) }.as_bool().then(|| {
            let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
            Monitor { name: String::from_utf16_lossy(&info.szDevice[..len]), work: info.monitorInfo.rcWork }
        })
    }

    fn monitors() -> Vec<Monitor> {
        unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> windows::core::BOOL {
            let list = &mut *(data.0 as *mut Vec<Monitor>);
            if let Some(i) = info(m) {
                list.push(i);
            }
            true.into()
        }
        let mut list: Vec<Monitor> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut list as *mut _ as isize));
        }
        list
    }

    fn primary() -> Option<Monitor> {
        info(unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) })
    }

    fn rect(h: isize) -> RECT {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd(h), &mut r);
        }
        r
    }

    fn scale(h: isize) -> f64 {
        let dpi = unsafe { GetDpiForWindow(hwnd(h)) };
        if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }
    }

    pub fn place(h: isize, monitor: &str, edge: Edge, y: f64) {
        let Some(m) = monitors().into_iter().find(|m| !monitor.is_empty() && m.name == monitor).or_else(primary) else { return };
        let r = rect(h);
        let (w, ht) = (r.right - r.left, r.bottom - r.top);
        let margin = (2.0 * scale(h)).round() as i32;
        let x = match edge {
            Edge::Left => m.work.left + margin,
            Edge::Right => m.work.right - w - margin,
        };
        let free = (m.work.bottom - m.work.top - ht).max(0);
        let top = m.work.top + (free as f64 * y.clamp(0.0, 1.0)).round() as i32;
        unsafe {
            let _ = SetWindowPos(hwnd(h), Some(HWND_TOPMOST), x, top, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    fn cursor() -> POINT {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        p
    }

    /// Moves the window with the mouse while the left button is down.
    pub fn follow_mouse(h: isize) {
        let start = cursor();
        let r = rect(h);
        let (dx, dy) = (start.x - r.left, start.y - r.top);
        // GetAsyncKeyState reads physical buttons; for left-handed mice the primary one is the
        // physical right button.
        let primary = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
        // The button was already down when the drag began; a missed release ends it anyway
        // after a minute.
        for _ in 0..6_000 {
            if (unsafe { GetAsyncKeyState(primary.0 as i32) } as u16 & 0x8000) == 0 {
                break;
            }
            let p = cursor();
            unsafe {
                let _ = SetWindowPos(hwnd(h), None, p.x - dx, p.y - dy, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Which monitor, which edge and how high the bubble was dropped.
    pub fn snap(h: isize) -> (String, Edge, f64) {
        let r = rect(h);
        let center = POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 };
        let Some(m) = info(unsafe { MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST) }) else {
            return (String::new(), Edge::Right, 0.5);
        };
        let edge = if center.x < (m.work.left + m.work.right) / 2 { Edge::Left } else { Edge::Right };
        let free = (m.work.bottom - m.work.top - (r.bottom - r.top)).max(1);
        let y = ((r.top - m.work.top) as f64 / free as f64).clamp(0.0, 1.0);
        (m.name, edge, y)
    }
}
