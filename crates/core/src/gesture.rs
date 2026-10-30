//! One key or one mouse button, three ways to use it. The machine only turns presses and
//! releases into "start", "stop" or "cancel"; whether a dictation is running comes from
//! outside, because a dictation can also end on its own (error, time limit).

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Press starts, the next press stops.
    Toggle,
    /// Dictates while held (push to talk).
    Hold,
    /// A short tap toggles, a long press dictates while held.
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    /// Throw the recording away: the press turned out to be a drag.
    Cancel,
    Nothing,
}

/// Held at least this long, a press in [Mode::Both] counts as push to talk.
pub const HOLD_MS: u64 = 350;

#[derive(Debug, Clone)]
pub struct Gesture {
    pub mode: Mode,
    pressed_at: Option<u64>,
    /// The current press started the running dictation.
    started_by_press: bool,
    /// The current press will stop the running dictation on release.
    stop_pending: bool,
    /// Stop on release instead of on press. For the bubble: a press may still turn into a
    /// drag, and dragging must not end a running dictation.
    pub defer_stop: bool,
}

impl Gesture {
    pub fn new(mode: Mode) -> Self {
        Gesture { mode, pressed_at: None, started_by_press: false, stop_pending: false, defer_stop: false }
    }

    /// The bubble's variant, see [Gesture::defer_stop].
    pub fn for_pointer(mode: Mode) -> Self {
        Gesture { defer_stop: true, ..Gesture::new(mode) }
    }

    pub fn press(&mut self, now_ms: u64, recording: bool) -> Action {
        // Key repeat sends presses while the key is down; only the first counts.
        if self.pressed_at.is_some() {
            return Action::Nothing;
        }
        self.pressed_at = Some(now_ms);
        self.started_by_press = false;
        self.stop_pending = false;
        if recording {
            return match self.mode {
                Mode::Hold => Action::Nothing,
                Mode::Toggle | Mode::Both if self.defer_stop => {
                    self.stop_pending = true;
                    Action::Nothing
                }
                Mode::Toggle | Mode::Both => Action::Stop,
            };
        }
        self.started_by_press = true;
        Action::Start
    }

    pub fn release(&mut self, now_ms: u64, recording: bool) -> Action {
        let Some(at) = self.pressed_at.take() else { return Action::Nothing };
        let started = std::mem::take(&mut self.started_by_press);
        if std::mem::take(&mut self.stop_pending) {
            return if recording { Action::Stop } else { Action::Nothing };
        }
        if !recording || !started {
            return Action::Nothing;
        }
        match self.mode {
            Mode::Toggle => Action::Nothing,
            Mode::Hold => Action::Stop,
            Mode::Both if now_ms.saturating_sub(at) >= HOLD_MS => Action::Stop,
            Mode::Both => Action::Nothing,
        }
    }

    /// The bubble moved before the button came up: a dictation this press began is
    /// discarded, one that was already running keeps going, and the release does nothing.
    /// None: no drag at all - the press has been talking for a while (hold to talk), and the
    /// mouse only slipped; the bubble stays and the release stops as usual.
    pub fn drag(&mut self, now_ms: u64, recording: bool) -> Option<Action> {
        let Some(at) = self.pressed_at else { return Some(Action::Nothing) };
        if self.started_by_press && recording && self.mode != Mode::Toggle && now_ms.saturating_sub(at) >= HOLD_MS {
            return None;
        }
        let started = std::mem::take(&mut self.started_by_press);
        self.stop_pending = false;
        self.pressed_at = None;
        Some(if started && recording { Action::Cancel } else { Action::Nothing })
    }

    /// Forget a press, e.g. when the hotkey changes mid-press.
    pub fn reset(&mut self) {
        self.pressed_at = None;
        self.started_by_press = false;
        self.stop_pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;

    #[test]
    fn toggle() {
        let mut g = Gesture::new(Mode::Toggle);
        assert_eq!(g.press(0, false), Start);
        assert_eq!(g.release(2_000, true), Nothing);
        assert_eq!(g.press(5_000, true), Stop);
        assert_eq!(g.release(5_050, false), Nothing);
    }

    #[test]
    fn hold() {
        let mut g = Gesture::new(Mode::Hold);
        assert_eq!(g.press(0, false), Start);
        assert_eq!(g.release(80, true), Stop);
        // A dictation started elsewhere (the bubble) is not stopped by pressing.
        assert_eq!(g.press(100, true), Nothing);
        assert_eq!(g.release(200, true), Nothing);
    }

    #[test]
    fn both_tap_toggles_hold_talks() {
        let mut g = Gesture::new(Mode::Both);
        assert_eq!(g.press(0, false), Start);
        assert_eq!(g.release(120, true), Nothing, "a tap keeps recording");
        assert_eq!(g.press(4_000, true), Stop);
        assert_eq!(g.release(4_100, false), Nothing);

        assert_eq!(g.press(10_000, false), Start);
        assert_eq!(g.release(10_000 + HOLD_MS, true), Stop, "a hold stops on release");
    }

    #[test]
    fn key_repeat_is_ignored() {
        let mut g = Gesture::new(Mode::Both);
        assert_eq!(g.press(0, false), Start);
        assert_eq!(g.press(30, true), Nothing);
        assert_eq!(g.press(60, true), Nothing);
        assert_eq!(g.release(900, true), Stop);
    }

    #[test]
    fn dictation_ended_on_its_own() {
        let mut g = Gesture::new(Mode::Hold);
        assert_eq!(g.press(0, false), Start);
        assert_eq!(g.release(5_000, false), Nothing);
        assert_eq!(g.release(5_100, false), Nothing, "release without press");
    }

    #[test]
    fn drag_cancels_only_what_the_press_started() {
        let mut g = Gesture::new(Mode::Both);
        assert_eq!(g.press(0, false), Start);
        assert_eq!(g.drag(100, true), Some(Cancel));
        assert_eq!(g.release(900, false), Nothing);

        // Already dictating (tapped on): the bubble stops on release, so a drag keeps it going.
        let mut g = Gesture::for_pointer(Mode::Both);
        assert_eq!(g.press(0, false), Start);
        assert_eq!(g.release(50, true), Nothing);
        assert_eq!(g.press(1_000, true), Nothing);
        assert_eq!(g.drag(1_100, true), Some(Nothing));
        assert_eq!(g.release(1_400, true), Nothing);
        // Without a drag the release stops it.
        assert_eq!(g.press(2_000, true), Nothing);
        assert_eq!(g.release(2_080, true), Stop);
    }

    #[test]
    fn a_slip_of_the_mouse_while_holding_to_talk_is_no_drag() {
        for mode in [Mode::Hold, Mode::Both] {
            let mut g = Gesture::for_pointer(mode);
            assert_eq!(g.press(0, false), Start);
            assert_eq!(g.drag(30_000, true), None, "30 s of talking are not thrown away");
            assert_eq!(g.release(31_000, true), Stop, "{mode:?}: the release still stops");
        }
        // Toggle has no holding: a drag right after the press still discards.
        let mut g = Gesture::for_pointer(Mode::Toggle);
        assert_eq!(g.press(0, false), Start);
        assert_eq!(g.drag(2_000, true), Some(Cancel));
    }
}
