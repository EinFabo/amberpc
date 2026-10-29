// Downloadable parts (engine runtimes and models, the cleanup model) and the row that offers,
// tracks, selects and deletes one. Shared by the Engine, Cleanup and setup pages.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { DownloadEvent, LocalInfo, LocalItem, LocalStatus } from "../shared/types";
import { Progress, size } from "./ui";

export type DownloadKind = "runtime" | "model" | "llmRuntime" | "llmModel";

/** What is installed, what is downloading, and the state of both local servers. */
export function useLocal() {
  const [info, setInfo] = useState<LocalInfo | null>(null);
  const [progress, setProgress] = useState<Record<string, DownloadEvent>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});

  const reload = useCallback(async () => setInfo(await invoke<LocalInfo>("local_info")), []);

  useEffect(() => {
    void reload();
    const subs = [
      listen<DownloadEvent>("download", (e) => {
        const d = e.payload;
        setProgress((p) => ({ ...p, [d.key]: d }));
        if (d.state === "failed") setErrors((x) => ({ ...x, [d.key]: d.message ?? "failed" }));
        if (d.state !== "running") {
          setProgress((p) => {
            const { [d.key]: _, ...rest } = p;
            return rest;
          });
          void reload();
        }
      }),
      listen<LocalStatus>("local-status", (e) => setInfo((i) => (i ? { ...i, status: e.payload } : i))),
      listen<LocalStatus>("llm-status", (e) => setInfo((i) => (i ? { ...i, llmStatus: e.payload } : i))),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, [reload]);

  /** [select]: Amber makes it the one in use once it is installed. */
  const download = async (kind: DownloadKind, id: string, select: boolean) => {
    const key = `${kind}:${id}`;
    setErrors(({ [key]: _, ...rest }) => rest);
    setProgress((p) => ({ ...p, [key]: { key, done: 0, total: null, state: "running" } }));
    await invoke("download", { kind, id, select });
  };

  return { info, progress, errors, reload, download };
}

export type Local = ReturnType<typeof useLocal>;

export function DownloadItem({
  local,
  kind,
  item,
  selected,
  onSelect,
  recommended,
  warning,
  measured,
  toast,
}: {
  local: Local;
  kind: DownloadKind;
  item: LocalItem;
  selected: boolean;
  onSelect: () => void;
  recommended?: boolean;
  warning?: string;
  /** Measured milliseconds for 10 s of speech on this PC. */
  measured?: number;
  toast: (message: string) => void;
}) {
  const { info, progress, errors, reload, download } = local;
  const key = `${kind}:${item.id}`;
  const running = progress[key] ?? (info?.downloads.includes(key) ? { key, done: 0, total: null, state: "running" as const } : null);
  return (
    <div className="item">
      <div
        className={`radio ${selected ? "on" : ""} ${!item.installed ? "disabled" : ""}`}
        onClick={() => item.installed && onSelect()}
        title={item.installed ? "" : "Download it first"}
      />
      <div className="name">
        <div>
          {item.label}
          {recommended && <span className="tag amber">recommended</span>}
          {item.replaced && <span className="tag">replaced</span>}
          {item.installed && <span className="tag ok">installed</span>}
        </div>
        <div className="hint">
          {item.note} · {size(item.sizeMb)}
          {measured !== undefined && ` · ${(measured / 1000).toFixed(1)} s for 10 s of speech here`}
          {warning && ` · ${warning}`}
        </div>
        {errors[key] && <div className="note error">{errors[key]}</div>}
      </div>
      {running ? (
        <>
          <Progress done={running.done} total={running.total} />
          <button className="btn" onClick={() => invoke("cancel_download", { kind, id: item.id })}>
            Cancel
          </button>
        </>
      ) : item.installed ? (
        <button
          className="btn danger"
          onClick={async () => {
            try {
              await invoke("remove_download", { kind, id: item.id });
            } catch (e) {
              toast(String(e));
            }
            void reload();
          }}
        >
          Delete
        </button>
      ) : (
        <button className="btn" onClick={() => download(kind, item.id, true)}>
          Download
        </button>
      )}
    </div>
  );
}

/** "Ready", "Loading…" and so on, with a dot in the matching colour. */
export function StatusLabel({ status }: { status: LocalStatus }) {
  return (
    <>
      <span className={`dot ${status.state}`} />
      {{ stopped: "Not loaded", starting: "Loading…", ready: "Ready", failed: "Not available" }[status.state]}
    </>
  );
}
