//! Muted in Discord while Amber listens: Amber flips Discord's own microphone switch, the one
//! at the bottom left, through UI Automation - the interface screen readers use. Discord shows
//! the user muted as usual, and nothing needs setting up there.
//!
//! The ways that did not work: Discord's API needs an application of the user's own in its
//! developer portal; a Push to Mute keybind fed with keys from Amber proved unreliable; and
//! muting Discord's recording session in Windows silences the microphone for every program on
//! it, Amber included (measured).
//!
//! The work runs on a thread of its own, in order: COM wants its own apartment, and a mute
//! must never overtake the unmute that follows it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use amber_core::settings::DiscordSettings;

/// Whether a mute was asked for and not taken back yet.
static MUTED: AtomicBool = AtomicBool::new(false);

/// Mute or unmute, and whom to tell when it is done.
type Job = (bool, Option<Sender<()>>);

fn worker() -> &'static Mutex<Sender<Job>> {
    static WORKER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::spawn(move || {
            let mut discord = win::Discord::new();
            for (mute, done) in rx {
                match discord.set(mute) {
                    Ok(win::Outcome::Changed) => crate::diag::info(format!("Discord microphone {}", if mute { "off" } else { "on again" })),
                    Ok(win::Outcome::NoSwitch) => crate::diag::warn("Discord runs, but its microphone switch was not found"),
                    Ok(_) => {}
                    Err(e) => crate::diag::warn(format!("could not switch Discord's microphone: {e}")),
                }
                if let Some(done) = done {
                    let _ = done.send(());
                }
            }
        });
        Mutex::new(tx)
    })
}

fn send(mute: bool, done: Option<Sender<()>>) {
    let _ = worker().lock().unwrap().send((mute, done));
}

/// Mutes the user in Discord, if the switch is on.
pub fn hold(s: &DiscordSettings) {
    if s.mute_while_dictating && !MUTED.swap(true, Ordering::SeqCst) {
        send(true, None);
    }
}

/// Unmutes the user in Discord - only if Amber muted them, and they did not unmute meanwhile.
pub fn release() {
    if MUTED.swap(false, Ordering::SeqCst) {
        send(false, None);
    }
}

/// Amber is quitting: unmutes and waits a moment for it to happen.
pub fn release_now() {
    if MUTED.swap(false, Ordering::SeqCst) {
        let (tx, rx) = channel();
        send(false, Some(tx));
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
}

#[cfg(windows)]
mod win {
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::Foundation::{HWND, LPARAM, TRUE};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationTogglePattern, ToggleState_On, TreeScope_Descendants, UIA_ButtonControlTypeId,
        UIA_ControlTypePropertyId, UIA_TogglePatternId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowThreadProcessId};

    /// Discord, its test builds included.
    const PROGRAMS: &[&str] = &["discord.exe", "discordptb.exe", "discordcanary.exe", "discorddevelopment.exe"];
    /// The microphone switch's name in Discord in English and German, muted or not.
    const LABELS: &[&str] = &["Mute", "Unmute", "Stummschalten", "Stummschaltung aufheben"];

    pub enum Outcome {
        Changed,
        /// Already as wanted - muted by the user before, or unmuted by them meanwhile.
        Unchanged,
        /// Discord is not running.
        NoDiscord,
        /// Discord runs, but the switch is not where it was.
        NoSwitch,
    }

    pub struct Discord {
        uia: Option<IUIAutomation>,
        switch: Option<IUIAutomationTogglePattern>,
        /// Amber switched the microphone off; only then it switches it back on.
        muted_by_amber: bool,
    }

    impl Discord {
        pub fn new() -> Self {
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let uia = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok();
            Discord { uia, switch: None, muted_by_amber: false }
        }

        pub fn set(&mut self, mute: bool) -> Result<Outcome, String> {
            if !mute && !self.muted_by_amber {
                return Ok(Outcome::Unchanged);
            }
            let switch = match self.switch()? {
                Ok(s) => s,
                Err(outcome) => {
                    self.muted_by_amber = false;
                    return Ok(outcome);
                }
            };
            let muted = unsafe { switch.CurrentToggleState() }.map_err(|e| e.to_string())? == ToggleState_On;
            if muted == mute {
                self.muted_by_amber = false;
                return Ok(Outcome::Unchanged);
            }
            unsafe { switch.Toggle() }.map_err(|e| e.to_string())?;
            self.muted_by_amber = mute;
            Ok(Outcome::Changed)
        }

        /// The switch found last time if it still answers, else a fresh search (some 20 ms).
        fn switch(&mut self) -> Result<Result<IUIAutomationTogglePattern, Outcome>, String> {
            if let Some(s) = &self.switch {
                if unsafe { s.CurrentToggleState() }.is_ok() {
                    return Ok(Ok(s.clone()));
                }
            }
            self.switch = None;
            let windows = discord_windows();
            if windows.is_empty() {
                return Ok(Err(Outcome::NoDiscord));
            }
            let uia = self.uia.as_ref().ok_or("UI Automation is not available")?;
            let buttons = unsafe { uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ButtonControlTypeId.0)) }.map_err(|e| e.to_string())?;
            for hwnd in windows {
                let Ok(found) = (unsafe { uia.ElementFromHandle(hwnd).and_then(|root| root.FindAll(TreeScope_Descendants, &buttons)) }) else { continue };
                for i in 0..unsafe { found.Length() }.unwrap_or(0) {
                    let Ok(button) = (unsafe { found.GetElement(i) }) else { continue };
                    if !unsafe { button.CurrentName() }.is_ok_and(|n| LABELS.contains(&n.to_string().as_str())) {
                        continue;
                    }
                    // The bar of a call has a "Mute" too, without a state; the switch has one.
                    if let Ok(toggle) = unsafe { button.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) } {
                        self.switch = Some(toggle.clone());
                        return Ok(Ok(toggle));
                    }
                }
            }
            Ok(Err(Outcome::NoSwitch))
        }
    }

    /// Discord's windows: Chromium frames with a title, of a Discord process.
    fn discord_windows() -> Vec<HWND> {
        unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
            let list = &mut *(data.0 as *mut Vec<isize>);
            let mut class = [0u16; 64];
            let n = GetClassNameW(hwnd, &mut class);
            if GetWindowTextLengthW(hwnd) > 0 && String::from_utf16_lossy(&class[..n.max(0) as usize]) == "Chrome_WidgetWin_1" {
                list.push(hwnd.0 as isize);
            }
            TRUE
        }
        let mut all: Vec<isize> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut all as *mut _ as isize));
        }
        all.into_iter()
            .map(|h| HWND(h as *mut _))
            .filter(|&h| {
                let mut pid = 0u32;
                unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
                crate::insert::exe_of_pid(pid).is_some_and(|exe| PROGRAMS.contains(&exe.as_str()))
            })
            .collect()
    }
}

#[cfg(not(windows))]
mod win {
    pub enum Outcome {
        Changed,
        NoSwitch,
        Unchanged,
    }

    pub struct Discord;

    impl Discord {
        pub fn new() -> Self {
            Discord
        }

        pub fn set(&mut self, _mute: bool) -> Result<Outcome, String> {
            let _ = (Outcome::Changed, Outcome::NoSwitch);
            Ok(Outcome::Unchanged)
        }
    }
}
