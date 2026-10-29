// What happens to the text between recognition and inserting: fillers out (rules, always),
// and optionally a chat model that resolves self-corrections and false starts.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ChatProvider, CleanupPreview } from "../../shared/types";
import type { PageProps } from "../App";
import { DownloadItem, StatusLabel, useLocal } from "../downloads";
import { Row, Section, Segmented, Switch } from "../ui";

const EXAMPLE = "Ähm, lass uns morgen um drei Uhr treffen, nee stopp, um vier Uhr, äh, im Café weil es da ruhig ist.";

export function CleanupPage(p: PageProps) {
  const { s, save } = p;
  const c = s.cleanup;
  return (
    <>
      <h1>Cleanup</h1>
      <p className="lead">What happens to your words before they are inserted. Hesitation sounds go out by rule; a language model can also untangle what you corrected while speaking.</p>
      <Section>
        <Row label="Remove filler words" hint="äh, ähm, öhm, hm … - by fixed rules, on this PC, instantly. Real words like “um” stay.">
          <Switch on={c.fillers} onChange={(v) => save((x) => (x.cleanup.fillers = v))} />
        </Row>
        <Row label="German commas" hint="Before “dass”, “weil”, “wenn”, “um … zu”, “sondern” - only where the comma is nearly certain, only in German text.">
          <Switch on={c.germanCommas} onChange={(v) => save((x) => (x.cleanup.germanCommas = v))} />
        </Row>
        <Row
          label="Smart cleanup"
          hint="A language model resolves self-corrections (“at three, no wait, at four” → “at four”), false starts and punctuation. If it fails or changes too much, the text goes in as recognized."
        >
          <Switch on={c.smart} onChange={(v) => save((x) => (x.cleanup.smart = v))} />
        </Row>
      </Section>
      {c.smart && (
        <>
          <div className="cards">
            <button className={`card ${c.engine === "local" ? "on" : ""}`} onClick={() => save((x) => (x.cleanup.engine = "local"))}>
              <div className="title">On this PC</div>
              <div className="hint">llama.cpp with Qwen3 4B on your GPU. Private, offline, free.</div>
            </button>
            <button className={`card ${c.engine === "server" ? "on" : ""}`} onClick={() => save((x) => (x.cleanup.engine = "server"))}>
              <div className="title">Server with API key</div>
              <div className="hint">Groq, OpenAI or your own server. The text is sent there.</div>
            </button>
          </div>
          {c.engine === "local" ? <LocalModel {...p} /> : <ServerModel {...p} />}
        </>
      )}
      <TryIt {...p} />
    </>
  );
}

function LocalModel({ s, save, toast }: PageProps) {
  const local = useLocal();
  const info = local.info;
  if (!info) return null;
  const st = info.llmStatus;
  return (
    <>
      <Section title="Runs on">
        {info.llmRuntimes.map((r) => (
          <DownloadItem
            key={r.id}
            local={local}
            kind="llmRuntime"
            item={r}
            selected={s.cleanup.localRuntime === r.id}
            onSelect={() => save((x) => (x.cleanup.localRuntime = r.id))}
            recommended={(info.hasNvidia ? "cuda" : "cpu") === r.id}
            warning={r.wantsGpu && !info.hasNvidia ? "no NVIDIA driver found on this PC" : undefined}
            toast={toast}
          />
        ))}
      </Section>
      <Section title="Model">
        {info.llmModels.map((m) => (
          <DownloadItem
            key={m.id}
            local={local}
            kind="llmModel"
            item={m}
            selected={s.cleanup.localModel === m.id}
            onSelect={() => save((x) => (x.cleanup.localModel = m.id))}
            toast={toast}
          />
        ))}
        <Row label={<StatusLabel status={st} />} hint={st.message ?? "Loaded with Amber while the smart cleanup is on; uses about 3 GB of graphics memory."}>
          {st.state !== "ready" && (
            <button className="btn" disabled={st.state === "starting"} onClick={() => invoke("start_llm")}>
              Load now
            </button>
          )}
        </Row>
      </Section>
    </>
  );
}

function ServerModel({ s, save, go }: PageProps) {
  const [providers, setProviders] = useState<ChatProvider[]>([]);
  useEffect(() => {
    void invoke<ChatProvider[]>("chat_providers").then(setProviders);
  }, []);
  const c = s.cleanup;
  const provider = providers.find((x) => x.id === c.provider);
  if (!provider) return null;
  return (
    <Section title="Server">
      <Row label="Provider">
        <Segmented
          value={provider.id}
          options={providers.map((x) => [x.id, x.name] as [string, string])}
          onChange={(id) =>
            save((x) => {
              x.cleanup.provider = id;
              x.cleanup.model = providers.find((q) => q.id === id)?.models[0]?.id ?? "";
            })
          }
        />
      </Row>
      {provider.id === "custom" && (
        <Row label="Address" hint="Empty: the speech engine's own-server address.">
          <input
            type="text"
            style={{ width: 300 }}
            defaultValue={c.baseUrl}
            placeholder={s.server.baseUrl || "http://…/v1"}
            onBlur={(e) => e.target.value !== c.baseUrl && save((x) => (x.cleanup.baseUrl = e.target.value.trim()))}
          />
        </Row>
      )}
      <Row label="Chat model" hint={provider.models.find((m) => m.id === c.model)?.note}>
        {provider.models.length > 0 ? (
          <select value={c.model} onChange={(e) => save((x) => (x.cleanup.model = e.target.value))}>
            {provider.models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
          </select>
        ) : (
          <input type="text" defaultValue={c.model} placeholder="llama3.2" onBlur={(e) => e.target.value !== c.model && save((x) => (x.cleanup.model = e.target.value.trim()))} />
        )}
      </Row>
      <Row
        label="API key"
        hint={provider.hasKey ? `Uses the ${provider.name} key saved on the Engine page.` : provider.id === "custom" ? "Optional for your own server." : `No ${provider.name} key yet.`}
      >
        <button className="btn" onClick={() => go("engine")}>
          {provider.hasKey ? "Change" : "Add key"}
        </button>
      </Row>
    </Section>
  );
}

function TryIt({ s }: PageProps) {
  const [text, setText] = useState(EXAMPLE);
  const [result, setResult] = useState<CleanupPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    setResult(await invoke<CleanupPreview>("cleanup_preview", { text }));
    setBusy(false);
  };
  return (
    <Section title="Try it">
      <div className="row column">
        <div className="note">Type or paste what a dictation might sound like:</div>
        <textarea value={text} onChange={(e) => setText(e.target.value)} />
        <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
          <button className="btn primary" disabled={busy || !text.trim()} onClick={run}>
            {busy ? "Cleaning…" : "Clean up"}
          </button>
          {result && s.cleanup.smart && <span className="note">{result.error ? "" : `${result.ms} ms`}</span>}
        </div>
        {result && (
          <div className="preview">
            <div className="note">By rule (fillers, commas)</div>
            <div>{result.fillers || <i>(nothing left)</i>}</div>
            {s.cleanup.smart && (
              <>
                <div className="note" style={{ marginTop: 10 }}>
                  Smart cleanup
                </div>
                {result.smart ? <div>{result.smart}</div> : <div className="note error">{result.error}</div>}
              </>
            )}
          </div>
        )}
      </div>
    </Section>
  );
}
