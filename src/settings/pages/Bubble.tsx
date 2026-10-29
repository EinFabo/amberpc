import type { BubbleVisibility, Edge } from "../../shared/types";
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
        <Row label="Opacity">
          <Slider min={30} max={100} step={5} value={Math.round(s.bubble.opacity * 100)} onCommit={(v) => save((x) => (x.bubble.opacity = v / 100))} />
        </Row>
        <Row label="Sounds" hint="A short tone when a dictation starts and stops.">
          <Switch on={s.sounds} onChange={(v) => save((x) => (x.sounds = v))} />
        </Row>
      </Section>
    </>
  );
}
