# Terminal mode - plan

**Goal:** in a terminal, what Fabi says is a request, not a dictation: "such mir alle Dateien,
die mit CL anfangen", "geh in den Ordner projects", "verschieb den Ordner X nach Y". The
cleanup's language model turns it into the shell command, and Amber types it at the prompt -
**not executed**: Fabi reads it and presses Enter. The phone app plans the same
(`~/projects/amethyst/docs/plans/2026-10-29-terminal-mode-and-everyday-fixes.md`, item 7);
the PC goes first.

## Fabi's decisions (2026-11-01)

- **Never executed:** typed without Enter; Fabi presses Enter himself.
- **Triggers, all three:** automatically when dictating into a terminal program (its profile,
  switchable off); a hotkey of its own, anywhere; a spoken prefix ("Befehl: ..." / "Command:
  ...") for one dictation.
- **Context:** the terminal window's title only (it often names the folder and the shell:
  "fabi@pc: ~/projects", "PowerShell") - nothing of the terminal's content is read.
- **Shells:** bash (WSL, SSH, Git Bash) and PowerShell. cmd is not supported.

## Design

### `crates/core/src/terminal.rs` (pure, tested)

- `enum Shell { Bash, PowerShell }`.
- `fn shell_for(exe: &str, title: &str, chosen: ShellChoice) -> Option<Shell>`: the profile's
  choice if set; else from the program and title - `wsl.exe`, `bash.exe`, `mintty.exe`
  (Git Bash), `putty.exe`, `ssh.exe`, a title like `user@host: path` -> bash;
  `powershell.exe`, `pwsh.exe`, a title with "PowerShell" -> PowerShell; Windows Terminal
  (`windowsterminal.exe`) by its title, bash when it names a Linux distribution ("Ubuntu") or
  `user@host`; `cmd.exe` -> None (no terminal mode, said once).
- `fn request(spoken: &str, shell: Shell, title: &str) -> Vec<Message>`: the system prompt -
  the dictation is a request; reply with exactly one line of `bash`/`PowerShell`, no prose, no
  Markdown; several steps joined (`&&` / `;`); keep paths and names as spoken; the window
  title as the only context ("probably in ~/projects"); never invent files; if the request
  is not a command, reply `# ` and the request.
- `fn command_from(reply: &str) -> Result<String, String>`: drop `<think>`, code fences, a
  leading `$ `, `PS> `, `> `; one line (several lines joined with the shell's separator, a
  line continuation kept out); refuse prose (sentences ending in a full stop with no command
  word) and empty answers.
- `fn risky(command: &str, shell: Shell) -> Option<&'static str>`: `rm -r`/`-rf`, `rmdir`,
  `dd`, `mkfs`, `chmod`/`chown -R`, `> file` (overwrite), `| sh`/`| bash`, `sudo`, `shutdown`,
  `git push --force`, `git reset --hard`; PowerShell `Remove-Item -Recurse`, `Format-*`,
  `Stop-Computer`, `Set-ExecutionPolicy`. A risky command is typed **commented out** (`# rm -rf
  build`): Enter does nothing; Fabi deletes the `#` to run it. Works in bash and PowerShell.
- `fn command_prefix(text: &str, words: &[String]) -> Option<String>`: "Befehl: ...",
  "Befehl ...", "Command, ..." at the start -> the rest.

### Amber PC

- **Settings:** `TerminalSettings { enabled, hotkey, prefixes: ["Befehl", "Command"] }`; a
  profile gets `terminal: Option<ShellChoice>` (None = off; known terminals get Auto as a
  default profile - Windows Terminal, PowerShell, pwsh, Git Bash/mintty, PuTTY, WSL).
- **Flow** (`dictation.rs`): after recognition - the hotkey, the prefix or a terminal profile
  makes it a request: `terminal::request` with the foreground window's title (read like the
  program now) -> the cleanup's model (`llm::cleaner`, local or cloud, its own prompt, no
  length check) -> `command_from` -> `risky` -> typed without a line break and without Enter
  (`insert` with line breaks refused). No cleanup model: the bubble says so, nothing typed.
- **The bubble** shows `>_` while a request runs; a risky command: a short note "commented
  out - remove the # to run".
- **History:** kept as `request -> command`, engine "terminal".
- **Settings UI:** Programs page - per program "Terminal commands" (Off / bash / PowerShell /
  Auto); Hotkeys page - "Command hotkey"; the prefixes on the Programs page.

### Not in this plan

Executing anything; reading the terminal's content; cmd; the phone app (its own plan).

## Phases (each TDD, suites green, then a review)

1. **core `terminal.rs`** - shell detection, the prompt, `command_from`, `risky`,
   `command_prefix`. Tests: "such alle Dateien, die mit CL anfangen" style replies cleaned
   (fences, `$ `, prose refused); risky ones flagged and commented; the shells told apart
   from exe and title (Windows Terminal with "Ubuntu", "PowerShell", `fabi@pc: ~/x`); the
   prefix found only at the start.
2. **Amber PC flow** - triggers, the model call, typing without Enter, the bubble mark,
   history. Checked by hand in Windows Terminal (PowerShell and Ubuntu tabs) and Git Bash:
   "list the five biggest files in my downloads folder" puts a correct, unexecuted command at
   the prompt; "lösch den Ordner build" comes commented out.
3. **Settings UI** - per-program terminal choice, hotkey, prefixes; checked by eye.
4. **Docs, whole review, release.**
