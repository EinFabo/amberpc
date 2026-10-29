// Mirrors of the Rust types the windows exchange (serde, camelCase).

export type Phase = "idle" | "recording" | "transcribing" | "inserting" | "done" | "error";

export interface DictationEvent {
  state: Phase;
  message?: string | null;
  /** While recognizing: the expected duration, for the progress ring. */
  expectedMs?: number;
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
export type WritingStyle = "asSpoken" | "casual" | "formal" | "concise";
export type CleanupChoice = "default" | "always" | "never";

export interface Profile {
  exe: string;
  label: string;
  style: WritingStyle;
  instruction: string;
  cleanup: CleanupChoice;
}

export interface Settings {
  engine: EngineKind;
  local: { runtime: string; model: string; threads: number; keepLoaded: boolean; chunking: boolean };
  server: { provider: string; baseUrl: string; model: string; localFallback: boolean };
  language: string;
  vocabulary: string;
  microphone: string;
  hotkey: { dictate: string; mode: HotkeyMode; toggleBubble: string };
  bubble: {
    visibility: BubbleVisibility;
    edge: Edge;
    y: number;
    monitor: string;
    size: number;
    opacity: number;
    hiddenIn: string[];
    hideFullscreen: boolean;
  };
  insert: { method: InsertMethod; restoreClipboard: boolean; trailingSpace: boolean };
  sounds: boolean;
  autostart: boolean;
  autoUpdate: boolean;
  logText: boolean;
  onboarded: boolean;
  phone: { enabled: boolean; port: number; cleanup: boolean };
  profiles: Profile[];
  history: boolean;
  historyDays: number;
  statistics: boolean;
  snippets: Snippet[];
  discord: { muteWhileDictating: boolean };
  cleanup: {
    fillers: boolean;
    spokenPunctuation: boolean;
    germanCommas: boolean;
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
  replaced: boolean;
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
  /** Model id -> measured milliseconds for 10 s of speech on the current runtime. */
  measured: Record<string, number>;
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

export interface Program {
  exe: string;
  title: string;
}

export interface HistoryEntry {
  at: number;
  text: string;
  program: string;
  engine: string;
  inserted: boolean;
}

export interface StatsSummary {
  wordsToday: number;
  dictationsToday: number;
  lastDays: number[];
  totalWords: number;
  totalDictations: number;
  minutesSavedToday: number;
  minutesSavedTotal: number;
  wordsByProgram: [string, number][];
  medianWaitMs: Record<string, number>;
  streakDays: number;
  longestAudioMs: number;
  speakingWpm: number | null;
  firstDay: number | null;
}

export interface Snippet {
  trigger: string;
  text: string;
}
