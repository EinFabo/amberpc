# Amber for Windows - project rules

- Everything in the project is English: UI text, logs, docs, comments, commits. The chat
  with Fabi follows his language. Exception: Whisper's German subtitle hallucinations in
  `crates/core/src/text.rs` stay as Whisper hears them.
- Logic that can go wrong without a platform API belongs in `crates/core` (pure Rust,
  tested in WSL with `cargo test -p amber-core`); `src-tauri` only holds what needs Windows
  or Tauri.
- Build and run on Windows through `tools/win.sh` (mirrors to C:\Users\fabia\build\amberpc).
  `tools/screenshot.ps1` captures the settings window (or `-Screen`) for checking by eye.
- Version lives in `package.json` and `Cargo.toml` (`[workspace.package]`) - keep both equal.
- The sibling Android app is `~/projects/amethyst` (Amber); its README explains the design.
