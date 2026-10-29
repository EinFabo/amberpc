// Mirrors of the Rust types the windows exchange (serde, camelCase).

export type Phase = "idle" | "recording" | "transcribing" | "inserting" | "done" | "error";

export interface DictationEvent {
  state: Phase;
  message?: string | null;
}

export interface BubbleConfig {
  opacity: number;
  sounds: boolean;
}

export type EngineKind = "local" | "server";
export type HotkeyMode = "toggle" | "hold" | "both";
export type BubbleVisibility = "always" | "whileDictating" | "hidden";
export type Edge = "left" | "right";
export type InsertMethod = "paste" | "type";

export interface Settings {
  engine: EngineKind;
  local: { runtime: string; model: string; threads: number; keepLoaded: boolean };
  server: { provider: string; baseUrl: string; model: string };
  language: string;
  vocabulary: string;
  microphone: string;
  hotkey: { dictate: string; mode: HotkeyMode; toggleBubble: string };
  bubble: { visibility: BubbleVisibility; edge: Edge; y: number; monitor: string; size: number; opacity: number };
  insert: { method: InsertMethod; restoreClipboard: boolean; trailingSpace: boolean };
  sounds: boolean;
  autostart: boolean;
  autoUpdate: boolean;
  logText: boolean;
  onboarded: boolean;
  phone: { enabled: boolean; port: number };
  cleanup: {
    fillers: boolean;
    smart: boolean;
    engine: EngineKind;
    localRuntime: string;
    localModel: string;
    provider: string;
    baseUrl: string;
    model: string;
  };
}

export interface ProviderInfo {
  id: string;
  name: string;
  baseUrl: string;
  models: { id: string; label: string; note: string }[];
  keyUrl: string;
  needsKey: boolean;
  hasKey: boolean;
}

export interface Overview {
  settings: Settings;
  version: string;
  providers: ProviderInfo[];
  phase: Phase;
}

export interface LocalItem {
  id: string;
  label: string;
  note: string;
  sizeMb: number;
  installed: boolean;
  wantsGpu: boolean;
}

export interface LocalStatus {
  state: "stopped" | "starting" | "ready" | "failed";
  message?: string | null;
}

export interface LocalInfo {
  runtimes: LocalItem[];
  models: LocalItem[];
  hasNvidia: boolean;
  recommended: [string, string];
  status: LocalStatus;
  downloads: string[];
  folder: string;
  llmRuntimes: LocalItem[];
  llmModels: LocalItem[];
  llmStatus: LocalStatus;
}

export interface DownloadEvent {
  key: string;
  done: number;
  total?: number | null;
  state: "running" | "done" | "failed" | "cancelled";
  message?: string | null;
}

export interface LogEntry {
  time: string;
  level: "info" | "warn" | "error";
  message: string;
}

export interface UpdateInfo {
  version: string;
  notes?: string | null;
}

export interface PhoneInfo {
  running: boolean;
  error?: string | null;
  addresses: { name: string; url: string }[];
  qr?: string | null;
}

export interface CleanupPreview {
  fillers: string;
  smart?: string | null;
  error?: string | null;
  ms: number;
}

export interface ChatProvider {
  id: string;
  name: string;
  models: { id: string; label: string; note: string }[];
  hasKey: boolean;
}
