//! Muting yourself in Discord while dictating. Discord offers no API for that to other
//! programs, but it has a "Push to Mute" keybind: Amber holds a key no keyboard has
//! (F13-F24) for as long as it listens, and lets go when the recording ends. Held, not
//! toggled - whoever was muted before stays muted after.

use std::sync::atomic::{AtomicU8, Ordering};

use amber_core::settings::DiscordSettings;

/// The key held down right now (13..=24), 0 for none.
static HELD: AtomicU8 = AtomicU8::new(0);

/// Holds the Push to Mute key, if muting is on.
pub fn hold(s: &DiscordSettings) {
    if !s.mute_while_dictating {
        return;
    }
    let n = s.key_number();
    // Already held: that key stays the one to release.
    if HELD.compare_exchange(0, n, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
        win::key(n, false);
    }
}

/// Lets the key go; nothing happens if none is held.
pub fn release() {
    let n = HELD.swap(0, Ordering::SeqCst);
    if n != 0 {
        win::key(n, true);
    }
}

/// After a crash with the key held, Windows still counts it as down - and Discord keeps the
/// user muted. A stray key-up at start sets that right.
pub fn reset(s: &DiscordSettings) {
    if s.mute_while_dictating && HELD.load(Ordering::SeqCst) == 0 {
        win::key(s.key_number(), true);
    }
}

/// One press of [key] after [delay_ms] - for recording the keybind in Discord, since the key
/// cannot be pressed by hand. Skipped while a dictation holds it.
pub fn press_later(key: u8, delay_ms: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        if HELD.load(Ordering::SeqCst) != 0 {
            return;
        }
        win::key(key, false);
        std::thread::sleep(std::time::Duration::from_millis(120));
        win::key(key, true);
    });
}

#[cfg(windows)]
mod win {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC,
        VIRTUAL_KEY, VK_F13,
    };

    /// F13..F24 down or up, with its scan code for programs that read those.
    pub fn key(n: u8, up: bool) {
        let vk = VIRTUAL_KEY(VK_F13.0 + (n.clamp(13, 24) - 13) as u16);
        let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16;
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 },
            },
        };
        if unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) } == 0 {
            crate::diag::warn(format!("could not send F{n} for Discord"));
        }
    }
}

#[cfg(not(windows))]
mod win {
    pub fn key(_n: u8, _up: bool) {}
}
