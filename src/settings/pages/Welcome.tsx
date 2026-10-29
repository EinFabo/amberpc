// First start: one decision (local or server), the downloads it needs, and how to dictate.

import { useEffect } from "react";
import type { PageProps } from "../App";
import { Section, Row, Progress, size } from "../ui";
import { useLocal } from "./Engine";
import { pretty } from "./Hotkeys";

export function WelcomePage(p: PageProps) {
  const { s, save, go } = p;
  const { info, progress, download } = useLocal();

  // Seeing this page once is enough; next start opens the normal settings.
  useEffect(() => {
    if (!s.onboarded) void save((x) => (x.onboarded = true));
  }, [s.onboarded, save]);

  if (!info) return null;
  const [rtId, modelId] = info.recommended;
  const rt = info.runtimes.find((r) => r.id === rtId)!;
  const model = info.models.find((m) => m.id === modelId)!;
  const ready = rt.installed && model.installed;
  const rtDl = progress[`runtime:${rtId}`];
  const modelDl = progress[`model:${modelId}`];
  const downloading = !!(rtDl || modelDl) || info.downloads.length > 0;

  const setupLocal = async () => {
    await save((x) => {
      x.engine = "local";
      x.local.runtime = rtId;
      x.local.model = modelId;
    });
    if (!rt.installed) await download("runtime", rtId);
    if (!model.installed) await download("model", modelId);
  };

  return (
    <>
      <h1>Welcome to Amber</h1>
      <p className="lead">
        Speak, and the text appears where your cursor is - in any app. Pick where your voice is recognized; you can change everything later.
      </p>
      <div className="welcome-steps">
        <Section title="1 · Engine">
          <Row
            label={`On this PC: ${model.label} on ${rt.label}`}
            hint={
              info.hasNvidia
                ? `Your NVIDIA GPU makes this fast. Download: ${size((rt.installed ? 0 : rt.sizeMb) + (model.installed ? 0 : model.sizeMb))}.`
                : `Runs on your processor. Download: ${size((rt.installed ? 0 : rt.sizeMb) + (model.installed ? 0 : model.sizeMb))}.`
            }
          >
            {ready && s.engine === "local" ? (
              <span className="tag ok">ready</span>
            ) : downloading ? (
              <Progress done={(rtDl?.done ?? 0) + (modelDl?.done ?? 0)} total={(rtDl?.total ?? 0) + (modelDl?.total ?? 0) || null} />
            ) : (
              <button className="btn primary" onClick={setupLocal}>
                {ready ? "Use it" : "Set up"}
              </button>
            )}
          </Row>
          <Row label="Or a server with your API key" hint="Groq is fast and costs about 4 cents per hour of speech.">
            <button
              className="btn"
              onClick={async () => {
                await save((x) => (x.engine = "server"));
                go("engine");
              }}
            >
              Use a server
            </button>
          </Row>
        </Section>
        <Section title="2 · Microphone">
          <Row label={s.microphone || "System default microphone"}>
            <button className="btn" onClick={() => go("microphone")}>
              Test
            </button>
          </Row>
        </Section>
        <Section title="3 · Dictate">
          <Row
            label={
              s.hotkey.dictate ? (
                <>
                  Press <b>{pretty(s.hotkey.dictate)}</b> - tap to start and stop, or hold while you speak.
                </>
              ) : (
                "Set a hotkey to dictate from anywhere."
              )
            }
            hint="Or click the amber bubble at the edge of your screen."
          >
            <button className="btn" onClick={() => go("insert")}>
              Try it
            </button>
          </Row>
        </Section>
      </div>
    </>
  );
}
