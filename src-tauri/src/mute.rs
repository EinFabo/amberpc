//! Muted in Discord while Amber listens: Amber flips Discord's own microphone switch, the one
//! at the bottom left. Discord shows the user muted as usual, and nothing needs setting up.
//!
//! The switch is found through UI Automation, the interface screen readers use - afresh each
//! time, since Discord replaces it when it flips - and pressed with a click sent to Discord's
//! window as a message. UI Automation's own Toggle would let
//! Discord take the focus - a game loses it, may leave full screen or pause - while a sent
//! click cannot bring a window to the front. So there is no fallback that takes the focus:
//! with Discord minimized the click does not land, and Amber leaves the microphone as it is.
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
                let state = if mute { "off" } else { "on again" };
                match discord.set(mute) {
                    Ok(win::Outcome::Clicked) => crate::diag::info(format!("Discord microphone {state}")),
                    Ok(win::Outcome::NotTaken) => crate::diag::warn(format!("Discord microphone should be {state}, but the switch did not change")),
                    Ok(win::Outcome::Minimized) => crate::diag::warn("Discord is minimized - its microphone switch cannot be pressed without taking the focus"),
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
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{HWND, LPARAM, POINT, TRUE, WPARAM};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTogglePattern, ToggleState_On, TreeScope_Descendants,
        UIA_ButtonControlTypeId, UIA_ControlTypePropertyId, UIA_TogglePatternId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, PostMessageW, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MOUSEMOVE,
    };

    /// Discord, its test builds included.
    const PROGRAMS: &[&str] = &["discord.exe", "discordptb.exe", "discordcanary.exe", "discorddevelopment.exe"];
    /// The microphone switch's name in Discord in English and German, muted or not.
    const LABELS: &[&str] = &["Mute", "Unmute", "Stummschalten", "Stummschaltung aufheben"];
    /// Not in the windows crate's WindowsAndMessaging module.
    const WM_MOUSELEAVE: u32 = 0x02A3;
    /// How long a sent click gets to show in the switch, for the log.
    const CLICK_TAKES: Duration = Duration::from_millis(600);

    pub enum Outcome {
        Clicked,
        /// Sent, but the switch did not change in time.
        NotTaken,
        Minimized,
        /// Already as wanted - muted by the user before, or unmuted by them meanwhile.
        Unchanged,
        /// Discord is not running.
        NoDiscord,
        /// Discord runs, but the switch is not where it was.
        NoSwitch,
    }

    struct Switch {
        window: HWND,
        element: IUIAutomationElement,
        toggle: IUIAutomationTogglePattern,
    }

    impl Switch {
        fn muted(&self) -> windows::core::Result<bool> {
            Ok(unsafe { self.toggle.CurrentToggleState() }? == ToggleState_On)
        }

        /// A click into the middle of the switch, as messages to Discord's window; false if
        /// there is nowhere to click. Ends with the mouse leaving: without that Discord takes
        /// the mouse for still hovering there, ignores the next click (the unmute) and stays
        /// awake as if in use until it is focused once.
        fn click(&self) -> bool {
            let Ok(r) = (unsafe { self.element.CurrentBoundingRectangle() }) else { return false };
            if r.right <= r.left || r.bottom <= r.top {
                return false;
            }
            let mut p = POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 };
            if !unsafe { ScreenToClient(self.window, &mut p) }.as_bool() {
                return false;
            }
            let at = LPARAM(((p.y as u16 as isize) << 16) | p.x as u16 as isize);
            unsafe {
                PostMessageW(Some(self.window), WM_MOUSEMOVE, WPARAM(0), at).is_ok()
                    && PostMessageW(Some(self.window), WM_LBUTTONDOWN, WPARAM(1), at).is_ok()
                    && PostMessageW(Some(self.window), WM_LBUTTONUP, WPARAM(0), at).is_ok()
                    && PostMessageW(Some(self.window), WM_MOUSELEAVE, WPARAM(0), LPARAM(0)).is_ok()
            }
        }
    }

    /// The switch is looked up afresh every time: Discord replaces it with a new one when it
    /// flips, and the old one goes on answering "not muted".
    pub struct Discord {
        uia: Option<IUIAutomation>,
        /// Amber switched the microphone off; only then it switches it back on.
        muted_by_amber: bool,
    }

    impl Discord {
        pub fn new() -> Self {
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let uia = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok();
            Discord { uia, muted_by_amber: false }
        }

        pub fn set(&mut self, mute: bool) -> Result<Outcome, String> {
            if !mute && !self.muted_by_amber {
                return Ok(Outcome::Unchanged);
            }
            let switch = match self.find()? {
                Ok(switch) => switch,
                Err(outcome) => {
                    self.muted_by_amber = false;
                    return Ok(outcome);
                }
            };
            if switch.muted().map_err(|e| e.to_string())? == mute {
                self.muted_by_amber = false;
                return Ok(Outcome::Unchanged);
            }
            if unsafe { IsIconic(switch.window) }.as_bool() {
                return Ok(Outcome::Minimized);
            }
            if !switch.click() {
                return Ok(Outcome::NoSwitch);
            }
            // Sent: the switch is Amber's to switch back, even if the click shows only later -
            // the unmute looks at the switch itself before clicking again.
            self.muted_by_amber = mute;
            Ok(if self.shows(mute, CLICK_TAKES) { Outcome::Clicked } else { Outcome::NotTaken })
        }

        /// Waits up to [limit] for the switch to show [muted].
        fn shows(&self, muted: bool, limit: Duration) -> bool {
            let until = Instant::now() + limit;
            loop {
                if self.find().is_ok_and(|f| f.is_ok_and(|s| s.muted().is_ok_and(|m| m == muted))) {
                    return true;
                }
                if Instant::now() > until {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        /// The switch as Discord shows it now (some 20 ms).
        fn find(&self) -> Result<Result<Switch, Outcome>, String> {
            let windows = discord_windows();
            if windows.is_empty() {
                return Ok(Err(Outcome::NoDiscord));
            }
            let uia = self.uia.as_ref().ok_or("UI Automation is not available")?;
            let buttons = unsafe { uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ButtonControlTypeId.0)) }.map_err(|e| e.to_string())?;
            for window in windows {
                let Ok(found) = (unsafe { uia.ElementFromHandle(window).and_then(|root| root.FindAll(TreeScope_Descendants, &buttons)) }) else { continue };
                for i in 0..unsafe { found.Length() }.unwrap_or(0) {
                    let Ok(element) = (unsafe { found.GetElement(i) }) else { continue };
                    if !unsafe { element.CurrentName() }.is_ok_and(|n| LABELS.contains(&n.to_string().as_str())) {
                        continue;
                    }
                    // The bar of a call has a "Mute" too, without a state; the switch has one.
                    if let Ok(toggle) = unsafe { element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) } {
                        return Ok(Ok(Switch { window, element, toggle }));
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
        Clicked,
        NotTaken,
        Minimized,
        NoSwitch,
        Unchanged,
    }

    pub struct Discord;

    impl Discord {
        pub fn new() -> Self {
            Discord
        }

        pub fn set(&mut self, _mute: bool) -> Result<Outcome, String> {
            let _ = (Outcome::Clicked, Outcome::NotTaken, Outcome::Minimized, Outcome::NoSwitch);
            Ok(Outcome::Unchanged)
        }
    }
}
