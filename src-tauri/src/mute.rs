//! Discord hears silence while Amber listens. Windows keeps an audio session per program and
//! device, for recording as for playback; muting Discord's recording sessions - what the
//! volume mixer does for playback - hands Discord silence and leaves every other program's
//! microphone alone, Amber's own included. Nothing to set up in Discord - its API would need
//! an application of the user's own in Discord's developer portal, a Push to Mute keybind fed
//! with keys from Amber proved unreliable.
//!
//! The work runs on a thread of its own, in order: COM wants its own apartment, and a mute
//! must never overtake the unmute that follows it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use amber_core::settings::DiscordSettings;

/// Whether Amber muted Discord and has not given the microphone back yet.
static MUTED: AtomicBool = AtomicBool::new(false);

/// Mute or unmute, and whom to tell when it is done.
type Job = (bool, Option<Sender<()>>);

fn worker() -> &'static Mutex<Sender<Job>> {
    static WORKER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::spawn(move || {
            win::init();
            for (mute, done) in rx {
                let what = if mute { "mute" } else { "unmute" };
                match win::set(mute) {
                    Ok(0) => {}
                    Ok(n) => crate::diag::info(format!("Discord: {what}d {n} recording session{}", if n == 1 { "" } else { "s" })),
                    Err(e) => crate::diag::warn(format!("could not {what} Discord: {e}")),
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

/// Mutes Discord's microphone, if the switch is on.
pub fn hold(s: &DiscordSettings) {
    if s.mute_while_dictating && !MUTED.swap(true, Ordering::SeqCst) {
        send(true, None);
    }
}

/// Gives Discord its microphone back, if Amber took it. Every Discord recording session is
/// unmuted, not only the ones muted at the start: Windows can carry a mute over to a session
/// opened later, and no Windows settings page mutes a program's recording - a muted one is
/// always Amber's doing.
pub fn release() {
    if MUTED.swap(false, Ordering::SeqCst) {
        send(false, None);
    }
}

/// Amber is quitting: gives the microphone back and waits a moment for it to happen.
pub fn release_now() {
    if MUTED.swap(false, Ordering::SeqCst) {
        let (tx, rx) = channel();
        send(false, Some(tx));
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
}

/// After a crash while dictating, Discord would stay muted: unmuted at start.
pub fn reset(s: &DiscordSettings) {
    if s.mute_while_dictating {
        send(false, None);
    }
}

#[cfg(windows)]
mod win {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        eCapture, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
        DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

    /// Discord, its test builds included.
    const PROGRAMS: &[&str] = &["discord.exe", "discordptb.exe", "discordcanary.exe", "discorddevelopment.exe"];

    pub fn init() {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    }

    /// Mutes or unmutes Discord's recording sessions on every microphone; how many changed.
    pub fn set(mute: bool) -> windows::core::Result<usize> {
        let mut changed = 0;
        unsafe {
            let devices = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)?.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
            for i in 0..devices.GetCount()? {
                let Ok(manager) = devices.Item(i).and_then(|d| d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)) else { continue };
                let sessions = manager.GetSessionEnumerator()?;
                for j in 0..sessions.GetCount()? {
                    let Ok(session) = sessions.GetSession(j) else { continue };
                    // Sessions shared by several processes have no single one: not Discord's.
                    let Ok(pid) = session.cast::<IAudioSessionControl2>().and_then(|c| c.GetProcessId()) else { continue };
                    if !crate::insert::exe_of_pid(pid).is_some_and(|exe| PROGRAMS.contains(&exe.as_str())) {
                        continue;
                    }
                    let volume: ISimpleAudioVolume = session.cast()?;
                    if volume.GetMute()?.as_bool() != mute {
                        volume.SetMute(mute, std::ptr::null())?;
                        changed += 1;
                    }
                }
            }
        }
        Ok(changed)
    }
}

#[cfg(not(windows))]
mod win {
    pub fn init() {}

    pub fn set(_mute: bool) -> Result<usize, String> {
        Ok(0)
    }
}
