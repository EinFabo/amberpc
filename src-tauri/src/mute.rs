//! Muted in Discord while Amber listens: Amber flips Discord's own microphone switch, the one
//! at the bottom left. Discord shows the user muted as usual, and nothing needs setting up.
//!
//! The switch is found through UI Automation, the interface screen readers use - afresh each
//! time, since Discord replaces it when it flips - and pressed with a click sent to Discord's
//! window as a message, ending with the mouse leaving (else Discord takes the mouse for still
//! hovering, ignores the next click and stays awake). UI Automation's own Toggle would let
//! Discord take the focus - a game loses it, may leave full screen or pause - while a sent
//! click cannot bring a window to the front. So there is no fallback that takes the focus:
//! with Discord minimized or in the tray Amber leaves the microphone as it is.
//!
//! Amber only takes back its own mute: not one the user set, not while they are deafened
//! (unmuting would undeafen them). An unmute that does not take - Discord minimized, busy,
//! closed - is tried again every few seconds until it does. Dictations into Discord itself
//! are left alone: pressing the switch could take the message box's focus.
//!
//! The ways that did not work: Discord's API needs an application of the user's own in its
//! developer portal; a Push to Mute keybind fed with keys from Amber proved unreliable; and
//! muting Discord's recording session in Windows silences the microphone for every program on
//! it, Amber included (measured).
//!
//! The work runs on a thread of its own, in order: COM wants its own apartment, and a mute
//! must never overtake the unmute that follows it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use amber_core::settings::DiscordSettings;

/// Discord, its test builds included.
const PROGRAMS: &[&str] = &["discord.exe", "discordptb.exe", "discordcanary.exe", "discorddevelopment.exe"];
/// How often an unmute that did not take yet is tried again.
const RETRY: Duration = Duration::from_secs(3);

/// Whether Amber wants the user muted right now. Changed only under the worker's lock, together
/// with queueing the job, so flag and queue never disagree.
static MUTED: AtomicBool = AtomicBool::new(false);
static WORKER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();

/// Mute, unmute, or only report back (None), and whom to tell when it is done.
type Job = (Option<bool>, Option<Sender<()>>);

fn worker() -> &'static Mutex<Sender<Job>> {
    WORKER.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::spawn(move || {
            let mut discord = win::Discord::new();
            loop {
                // With an unmute still to do, the queue is only waited on for a while.
                let (job, done, retry) = if discord.unmute_pending() {
                    match rx.recv_timeout(RETRY) {
                        Ok((job, done)) => (job, done, false),
                        Err(RecvTimeoutError::Timeout) => (Some(false), None, true),
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                } else {
                    match rx.recv() {
                        Ok((job, done)) => (job, done, false),
                        Err(_) => break,
                    }
                };
                // A report-back while an unmute is pending (Amber quitting): one more try.
                let job = job.or_else(|| discord.unmute_pending().then_some(false));
                if let Some(mute) = job {
                    let was_pending = discord.unmute_pending();
                    // A bug here must not end the thread, and with it every later unmute.
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| discord.set(mute))) {
                        Ok(outcome) => report(mute, outcome, retry || was_pending),
                        Err(_) => crate::diag::error("the Discord switch helper failed"),
                    }
                }
                if let Some(done) = done {
                    let _ = done.send(());
                }
            }
        });
        Mutex::new(tx)
    })
}

fn report(mute: bool, outcome: Result<win::Outcome, String>, again: bool) {
    use win::Outcome::*;
    let state = if mute { "off" } else { "on again" };
    match outcome {
        Ok(Clicked) => crate::diag::info(format!("Discord microphone {state}")),
        Ok(NotTaken) => crate::diag::warn(format!("Discord microphone should be {state}, but the switch did not change (yet)")),
        Ok(Hidden) if mute => crate::diag::warn("Discord is minimized or in the tray - its microphone switch cannot be pressed without taking the focus"),
        Ok(NoSwitch) if mute => crate::diag::warn("Discord runs, but its microphone switch was not found"),
        // Said once; the tries after it stay quiet until one works.
        Ok(Waiting(why)) if !again => crate::diag::warn(format!("Discord microphone not on again yet ({why}) - Amber keeps trying")),
        Ok(_) => {}
        Err(e) if !again => crate::diag::warn(format!("could not switch Discord's microphone: {e}")),
        Err(_) => {}
    }
}

pub fn is_discord(exe: &str) -> bool {
    PROGRAMS.contains(&exe)
}

/// Muted exactly while [recording] says Amber listens. Asked again under the lock, so calls
/// racing in from different threads end in the state of the latest phase. [program]: where
/// the dictation goes - into Discord itself, nothing is pressed.
pub fn follow(s: &DiscordSettings, program: Option<&str>, recording: impl Fn() -> bool) {
    let into_discord = program.is_some_and(is_discord);
    if s.mute_while_dictating && into_discord && recording() {
        crate::diag::info("dictating into Discord itself: not muting there");
    }
    let wanted = s.mute_while_dictating && !into_discord;
    if !wanted && WORKER.get().is_none() {
        // Never muted anything: no helper thread needed.
        return;
    }
    let queue = worker().lock().unwrap();
    let want = wanted && recording();
    if MUTED.swap(want, Ordering::SeqCst) != want && queue.send((Some(want), None)).is_err() {
        crate::diag::error("the Discord switch helper is gone");
    }
}

/// Amber is quitting: whatever is still to do for Discord gets a moment to happen.
pub fn release_now() {
    let Some(worker) = WORKER.get() else { return };
    let (tx, rx) = channel();
    {
        let queue = worker.lock().unwrap();
        let job = MUTED.swap(false, Ordering::SeqCst).then_some(false);
        let _ = queue.send((job, Some(tx)));
    }
    let _ = rx.recv_timeout(Duration::from_secs(2));
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
        EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    };

    /// The microphone switch's name in Discord in English and German, muted or not.
    const MUTE: &[&str] = &["Mute", "Unmute", "Stummschalten", "Stummschaltung aufheben"];
    /// The deafen switch's, likewise.
    const DEAFEN: &[&str] = &["Deafen", "Undeafen", "Ein- und Ausgabe deaktivieren", "Ein- und Ausgabe aktivieren"];
    /// Not in the windows crate's WindowsAndMessaging module.
    const WM_MOUSELEAVE: u32 = 0x02A3;
    /// How long a sent click gets to show in the switch.
    const CLICK_TAKES: Duration = Duration::from_millis(600);
    /// How long a mute click that did not show may still land: the unmute waits that long.
    const LATE: Duration = Duration::from_secs(4);

    pub enum Outcome {
        Clicked,
        /// A mute click was sent, but the switch did not change in time.
        NotTaken,
        /// An unmute that has to wait, and why: tried again later.
        Waiting(&'static str),
        /// Already as wanted - muted by the user, or unmuted by them meanwhile.
        Unchanged,
        /// Discord is not running.
        NoDiscord,
        /// Discord's windows are minimized or in the tray: a sent click would not land.
        Hidden,
        /// Discord runs, but the switch is not where it was.
        NoSwitch,
    }

    struct Switch {
        window: HWND,
        element: IUIAutomationElement,
        toggle: IUIAutomationTogglePattern,
        deafen: Option<IUIAutomationTogglePattern>,
    }

    impl Switch {
        fn muted(&self) -> Result<bool, String> {
            Ok(unsafe { self.toggle.CurrentToggleState() }.map_err(|e| e.to_string())? == ToggleState_On)
        }

        fn deafened(&self) -> bool {
            self.deafen.as_ref().is_some_and(|d| unsafe { d.CurrentToggleState() }.is_ok_and(|s| s == ToggleState_On))
        }

        /// A click into the middle of the switch, as messages to Discord's window; false if
        /// there is nowhere to click.
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

    pub struct Discord {
        uia: Option<IUIAutomation>,
        /// Amber pressed the switch to mute: that mute is Amber's to take back.
        owned: bool,
        /// When that press was sent without Discord showing it - it may still land.
        unconfirmed: Option<Instant>,
        /// The unmute has not taken yet: tried again every few seconds.
        pending: bool,
        /// The window pressed last, looked at first.
        window: Option<HWND>,
    }

    impl Discord {
        pub fn new() -> Self {
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let uia = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok();
            Discord { uia, owned: false, unconfirmed: None, pending: false, window: None }
        }

        pub fn unmute_pending(&self) -> bool {
            self.pending
        }

        pub fn set(&mut self, mute: bool) -> Result<Outcome, String> {
            if mute {
                self.pending = false;
                self.mute()
            } else {
                self.unmute()
            }
        }

        fn mute(&mut self) -> Result<Outcome, String> {
            let switch = match self.find()? {
                Ok(s) => s,
                Err(outcome) => return Ok(outcome),
            };
            if switch.muted()? {
                // Muted by the user: theirs, left alone at the stop. Still muted by Amber (an
                // unmute that had not taken yet): still Amber's to take back.
                if !self.owned {
                    self.unconfirmed = None;
                }
                return Ok(Outcome::Unchanged);
            }
            if !switch.click() {
                return Ok(Outcome::NoSwitch);
            }
            self.owned = true;
            self.window = Some(switch.window);
            if self.shows(true) {
                self.unconfirmed = None;
                Ok(Outcome::Clicked)
            } else {
                self.unconfirmed = Some(Instant::now());
                Ok(Outcome::NotTaken)
            }
        }

        fn unmute(&mut self) -> Result<Outcome, String> {
            if !self.owned {
                self.pending = false;
                return Ok(Outcome::Unchanged);
            }
            self.pending = true;
            let switch = match self.find()? {
                Ok(s) => s,
                // Discord keeps a mute across restarts: waited for, too.
                Err(Outcome::NoDiscord) => return Ok(Outcome::Waiting("Discord is not running")),
                Err(Outcome::Hidden) => return Ok(Outcome::Waiting("Discord is minimized or in the tray")),
                Err(_) => return Ok(Outcome::Waiting("its microphone switch was not found")),
            };
            if !switch.muted()? {
                if self.unconfirmed.is_some_and(|t| t.elapsed() < LATE) {
                    return Ok(Outcome::Waiting("the mute click has not shown yet"));
                }
                self.release();
                return Ok(Outcome::Unchanged);
            }
            if switch.deafened() {
                return Ok(Outcome::Waiting("Discord is deafened"));
            }
            if !switch.click() {
                return Ok(Outcome::Waiting("nowhere to click"));
            }
            if self.shows(false) {
                self.release();
                Ok(Outcome::Clicked)
            } else {
                Ok(Outcome::Waiting("the click did not take"))
            }
        }

        fn release(&mut self) {
            self.owned = false;
            self.pending = false;
            self.unconfirmed = None;
        }

        /// Waits a moment for the switch to show [muted].
        fn shows(&self, muted: bool) -> bool {
            let until = Instant::now() + CLICK_TAKES;
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

        /// The switch as Discord shows it now (some 20 ms), in the window pressed last if it
        /// still has one. Windows that are minimized or hidden are not searched: a sent click
        /// would not land there.
        fn find(&self) -> Result<Result<Switch, Outcome>, String> {
            let mut windows = discord_windows();
            if windows.is_empty() {
                return Ok(Err(Outcome::NoDiscord));
            }
            if let Some(last) = self.window.and_then(|w| windows.iter().position(|&h| h == w)) {
                windows.swap(0, last);
            }
            windows.retain(|&h| unsafe { IsWindowVisible(h).as_bool() && !IsIconic(h).as_bool() });
            if windows.is_empty() {
                return Ok(Err(Outcome::Hidden));
            }
            let uia = self.uia.as_ref().ok_or("UI Automation is not available")?;
            let buttons = unsafe { uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ButtonControlTypeId.0)) }.map_err(|e| e.to_string())?;
            for window in windows {
                let Ok(found) = (unsafe { uia.ElementFromHandle(window).and_then(|root| root.FindAll(TreeScope_Descendants, &buttons)) }) else { continue };
                let (mut mute, mut deafen) = (None, None);
                for i in 0..unsafe { found.Length() }.unwrap_or(0) {
                    let Ok(element) = (unsafe { found.GetElement(i) }) else { continue };
                    let Ok(name) = (unsafe { element.CurrentName() }).map(|n| n.to_string()) else { continue };
                    let (is_mute, is_deafen) = (MUTE.contains(&name.as_str()), DEAFEN.contains(&name.as_str()));
                    if !is_mute && !is_deafen {
                        continue;
                    }
                    // The bar of a call has a "Mute" too, without a state; the switches have one.
                    let Ok(toggle) = (unsafe { element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) }) else { continue };
                    if is_mute && mute.is_none() {
                        mute = Some((element, toggle));
                    } else if is_deafen && deafen.is_none() {
                        deafen = Some(toggle);
                    }
                    if mute.is_some() && deafen.is_some() {
                        break;
                    }
                }
                if let Some((element, toggle)) = mute {
                    return Ok(Ok(Switch { window, element, toggle, deafen }));
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
                crate::insert::exe_of_pid(pid).is_some_and(|exe| super::is_discord(&exe))
            })
            .collect()
    }
}

#[cfg(not(windows))]
mod win {
    pub enum Outcome {
        Clicked,
        NotTaken,
        Waiting(&'static str),
        Unchanged,
        Hidden,
        NoSwitch,
    }

    pub struct Discord;

    impl Discord {
        pub fn new() -> Self {
            Discord
        }

        pub fn unmute_pending(&self) -> bool {
            false
        }

        pub fn set(&mut self, _mute: bool) -> Result<Outcome, String> {
            let _ = (Outcome::Clicked, Outcome::NotTaken, Outcome::Waiting(""), Outcome::Hidden, Outcome::NoSwitch);
            Ok(Outcome::Unchanged)
        }
    }
}
