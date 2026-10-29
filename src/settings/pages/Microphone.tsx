import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PageProps } from "../App";
import { Row, Section, Switch } from "../ui";

interface Microphones {
  devices: string[];
  default: string | null;
}

export function MicrophonePage({ s, save, toast, phase }: PageProps) {
  const [mics, setMics] = useState<Microphones | null>(null);
  const [level, setLevel] = useState(0);
  const [testing, setTesting] = useState(false);

  const load = async () => setMics(await invoke<Microphones>("list_microphones"));

  useEffect(() => {
    void load();
    const un = listen<number>("mic-level", (e) => setLevel(e.payload));
    return () => {
      void un.then((f) => f());
      void invoke("mic_test", { device: null });
    };
  }, []);

  // A dictation takes the microphone over; the meter stops with it.
  useEffect(() => {
    if (phase === "recording") {
      setTesting(false);
      setLevel(0);
    }
  }, [phase]);

  const startTest = async (device: string) => {
    try {
      await invoke("mic_test", { device });
      setTesting(true);
    } catch (e) {
      toast(String(e));
      setTesting(false);
    }
  };

  const choose = async (device: string) => {
    await save((x) => (x.microphone = device));
    if (testing) await startTest(device);
  };

  const missing = s.microphone && mics && !mics.devices.includes(s.microphone);

  return (
    <>
      <h1>Microphone</h1>
      <p className="lead">The microphone Amber listens to. Speak and watch the meter - it should move clearly while you talk and rest when you are quiet.</p>
      <Section>
        <Row label="Microphone" hint={missing ? `"${s.microphone}" is not connected - the default microphone is used.` : undefined}>
          <select value={s.microphone} onChange={(e) => choose(e.target.value)}>
            <option value="">System default{mics?.default ? ` (${mics.default})` : ""}</option>
            {mics?.devices.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
          <button className="btn" onClick={load}>
            Refresh
          </button>
        </Row>
        <div className="row column">
          <div className="meter">
            <div style={{ width: `${Math.round(level * 100)}%` }} />
          </div>
          <div>
            {testing ? (
              <button
                className="btn"
                onClick={async () => {
                  await invoke("mic_test", { device: null });
                  setTesting(false);
                  setLevel(0);
                }}
              >
                Stop test
              </button>
            ) : (
              <button className="btn primary" onClick={() => startTest(s.microphone)}>
                Test microphone
              </button>
            )}
          </div>
        </div>
      </Section>
      <Section title="Discord">
        <Row
          label="Mute me in Discord while dictating"
          hint="While Amber listens it switches your microphone off in Discord - the switch at the bottom left - and on again at the stop. Nothing to set up; if you were muted already, you stay muted. Works with the hotkey, the bubble and the Stream Deck."
        >
          <Switch on={s.discord.muteWhileDictating} onChange={(v) => save((x) => (x.discord.muteWhileDictating = v))} />
        </Row>
      </Section>
      <p className="note">
        If no microphone shows up or the meter stays flat: Windows Settings → Privacy &amp; security → Microphone → "Let desktop apps access your microphone".
      </p>
    </>
  );
}
