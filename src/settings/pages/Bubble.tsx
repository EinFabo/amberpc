import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { BubbleVisibility, Edge, Program } from "../../shared/types";
import type { PageProps } from "../App";
import { Row, Section, Segmented, Slider, Switch } from "../ui";

const VISIBILITY: [BubbleVisibility, string][] = [
  ["always", "Always"],
  ["whileDictating", "While dictating"],
  ["hidden", "Hidden"],
];

export function BubblePage({ s, save }: PageProps) {
  return (
    <>
      <h1>Bubble</h1>
      <p className="lead">The amber disc at the edge of your screen. It never takes the focus away from the window you are writing in. Drag it anywhere - it settles at the nearer edge.</p>
      <Section>
        <Row label="Show the bubble" hint={s.bubble.visibility === "hidden" ? "Dictate with the hotkey; the bubble stays away." : undefined}>
          <Segmented value={s.bubble.visibility} options={VISIBILITY} onChange={(v) => save((x) => (x.bubble.visibility = v))} />
        </Row>
        <Row label="Edge">
          <Segmented<Edge>
            value={s.bubble.edge}
            options={[
              ["left", "Left"],
              ["right", "Right"],
            ]}
            onChange={(v) => save((x) => (x.bubble.edge = v))}
          />
        </Row>
        <Row label="Height on screen">
          <Slider min={0} max={100} value={Math.round(s.bubble.y * 100)} onCommit={(v) => save((x) => (x.bubble.y = v / 100))} />
        </Row>
        <Row label="Size">
          <Slider min={40} max={96} step={4} value={s.bubble.size} onCommit={(v) => save((x) => (x.bubble.size = v))} />
        </Row>
        <Row label="Place" hint="Drag the bubble anywhere; it settles at the nearer edge.">
          <button
            className="btn"
            onClick={() =>
              save((x) => {
                x.bubble.edge = "right";
                x.bubble.y = 0.62;
                x.bubble.monitor = "";
              })
            }
          >
            Back to its place
          </button>
        </Row>
        <Row label="Opacity at rest" hint="While dictating, the bubble is always fully visible.">
          <Slider min={30} max={100} step={5} value={Math.round(s.bubble.opacity * 100)} onCommit={(v) => save((x) => (x.bubble.opacity = v / 100))} />
        </Row>
        <Row label="Sounds" hint="A short tone when a dictation starts and stops.">
          <Switch on={s.sounds} onChange={(v) => save((x) => (x.sounds = v))} />
        </Row>
      </Section>
      <Section title="Out of the way">
        <Row label="Over full-screen programs" hint="Games and videos in full screen keep the screen to themselves. The hotkey and the Stream Deck still work.">
          <Switch on={s.bubble.hideFullscreen} onChange={(v) => save((x) => (x.bubble.hideFullscreen = v))} />
        </Row>
        <HiddenIn list={s.bubble.hiddenIn} onChange={(list) => save((x) => (x.bubble.hiddenIn = list))} />
      </Section>
    </>
  );
}

function HiddenIn({ list, onChange }: { list: string[]; onChange: (list: string[]) => void }) {
  const [running, setRunning] = useState<Program[]>([]);
  const [pick, setPick] = useState("");
  useEffect(() => {
    void invoke<Program[]>("running_programs").then(setRunning);
  }, []);
  return (
    <>
      <Row label="In these programs" hint="Apps with their own voice input, or where the bubble is in the way.">
        <select value={pick} onChange={(e) => setPick(e.target.value)}>
          <option value="">Pick an open program…</option>
          {running
            .filter((r) => !list.includes(r.exe))
            .map((r) => (
              <option key={r.exe} value={r.exe}>
                {r.exe}
              </option>
            ))}
        </select>
        <button
          className="btn"
          disabled={!pick}
          onClick={() => {
            onChange([...list, pick]);
            setPick("");
          }}
        >
          Add
        </button>
      </Row>
      {list.map((exe) => (
        <Row key={exe} label={exe}>
          <button className="btn danger" onClick={() => onChange(list.filter((e) => e !== exe))}>
            Remove
          </button>
        </Row>
      ))}
    </>
  );
}
