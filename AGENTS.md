# jarvis

Rust voice assistant for Windows 11, forked from Priler/jarvis. It opens and
closes applications, launches games, controls audio and files, and converses.
Recorded Jarvis responses cover fixed phrases; arbitrary text uses TTS.

## Configuration

This root file is the shared project contract. Tool-specific configuration
remains in `.codex/` or `.grok/`; setup is documented in their `README.md` files.
Reusable procedures live in `.agents/skills/`; existing shared evidence and
helper scripts may remain under `.codex/`. Native skill entry points forward
to the shared procedures without duplicating them.
The user-facing README and spoken text are Russian. User-facing values in
`assistant.toml` are Russian; configuration keys retain their original spelling.

## Architecture and behavior

| Area | Path |
|---|---|
| Wake word, STT, command dispatch | `crates/jarvis-app/src/app.rs` |
| Startup and tray | `crates/jarvis-app/src/main.rs`, `tray*.rs` |
| Core services | `crates/jarvis-core/src/` |
| Tauri 2 / Svelte settings GUI | `crates/jarvis-gui`, `frontend/` |
| Command packs | `resources/commands/*/command.toml` and Lua |
| Recorded voices | `resources/sound/voices/<voice>/voice.toml`, MP3/WAV |
| Vosk models | `resources/vosk/` |

1. Vosk detects activation. `endpoint.rs` splits speech at pauses; the local
   voice server defaults to GigaAM Multilingual Large CTC int8 ONNX on CPU.
   `whisper.rs` is the generic `/stt` client despite its name; `stt::feed` retains
   background audio. An unavailable server leaves Vosk text in use and is skipped
   for `retry_after_secs`. The intent classifier or fuzzy matching selects a pack.
2. `type = "action"` dispatches to `actions/`: apps, Steam games, volume, media,
   folders, files, power, search, and `input.rs` keyboard/window operations.
   `NAMED_HOTKEYS` defines key names. `key_target` avoids typing into Jarvis;
   `focus_app` raises the target. Pack tests forbid a phrase in two commands.
3. Unmatched commands and `ActionError::NotFound` fall back to `llm.rs`.
   OpenRouter uses `OPENROUTER_PAID_MODELS` with
   `~deepseek/deepseek-v4-flash-latest`; Polza uses `POLZA_MODELS` with
   `deepseek/deepseek-v4-flash`. `gateway_of_key` routes `sk-or-` to OpenRouter,
   rejects legacy Kilo `eyJ` keys, and routes other accepted keys to Polza.
   The selected gateway goes first. HTTP 402 sleeps a key for one hour; two
   consecutive 403 responses cool a provider for two minutes. Free OpenRouter
   models use a separate `openrouter-free` provider so paid-key cooldown does not
   disable them; they still require a key. `free_only` retains free and local
   keyless endpoints. Consult current provider documentation for quotas.
   Parsing migrates `kilo`/`paid` to OpenRouter, drops legacy keys and `kilo-free`,
   and ignores old `gemini` blocks. GUI save and `configure.ps1` remove those
   Gemini blocks and keys. Network failure cools a provider for three seconds;
   rebuild the remote HTTP pool without letting old workers reset the new pool
   or shorten its cooldown. Tools in `llm/tools.rs` expose the same native actions
   and active-window context. `llm-history.json` persists `memory_minutes` of
   conversation and built-in commands via `llm::remember_command`.
   `without_address_rule` strips obsolete address prompts. English reasoning
   instead of a user answer is a model failure and triggers fallback.
4. `Action::is_dangerous` requires spoken confirmation for deletion, shutdown,
   restart, sleep, and emptying the Recycle Bin (`actions/confirm.rs`). Closing
   an app means closing one window normally; multiple windows require selection,
   and success requires the HWND to disappear. Do not kill its processes.
   The simple success response is «Окно закрыто»; background details require a
   question. A known app with no window returns a short refusal without LLM
   fallback to another window. Steam targeting also handles `steamwebhelper`
   and existing aliases. `actions/dialog.rs` and `dialog.ps1` use UI Automation
   buttons bound to HWND/PID/RuntimeId and wait for save decisions. A Save As
   dialog is not confirmed completion. `inspect_window` and `dialog_button` are
   tools; editor shortcuts live in `NAMED_HOTKEYS` and the `keys` pack.
5. The sole cloned voice is Jarvis New, using F5-TTS ESpeech RL-V2, 16 steps,
   and RUAccent. Its sample and exact transcript are `[tts.ru]` in the sole
   `voice.toml`, at most 12 seconds. Synthesis requires CUDA/ROCm; no CPU TTS.
   HTTP returns a complete WAV. Report synthesis failure in `/health` and GUI;
   display the answer as a notification when speech fails.
6. `tools/voice-server/` exposes `/stt`, `/tts`, `/health`: GigaAM by default,
   Whisper only through explicit flags, and F5 synthesis. `requirements.txt`
   covers STT; `requirements-tts.txt` and `f5-tts==1.1.22 --no-deps` cover TTS.
   `tts-engine.txt` is obsolete. `setup.bat` defaults to F5; `-TtsEngine f5`
   remains compatible. Nano, its adapter, weights, and ONNX runtime were removed;
   historical comparisons remain in `.codex/evidence/`.
7. `gpu.py` detects `cuda | rocm | vulkan | cpu`; `gpu-profile.json` configures
   recognition. Explicit Whisper uses faster-whisper CUDA on NVIDIA and
   whisper.cpp Vulkan/CPU otherwise. GigaAM defaults are GPU-independent.
   F5 uses Python 3.12 with CUDA torch 2.8 (cu128 for Blackwell, cu126 for older
   NVIDIA) or AMD's Windows ROCm index. CPU/Intel/AMD without ROCm gets STT and
   a clear TTS-unavailable reason; recorded phrases still work. Supported-GPU
   setup failure must not silently install another synthesizer. Linux/NVIDIA
   timings do not establish Windows/AMD performance.
8. The application directory contains Python 3.12 under
   `tools/voice-server/python`, models, and caches. `-RuntimeOnly` refreshes F5
   and GPU torch while preserving other settings; only after success does it
   remove known Nano paths, including `models/nano`. Failed downloads cannot
   produce a successful runtime marker. Address phrases use the F5 cache
   namespace, never old Nano audio. SAPI and sample selection were removed;
   recorded Jarvis New responses remain independent of synthesis.

`web_search` opens Google, with Russian Google phrases in its pack; explicitly
named mail, music, and map services retain their meaning. Vision is separate
in `llm/vision.rs`: `gemini-flash-lite-latest`, one request with
`reasoning_effort = minimal`, response timeout 25 seconds, connect timeout
5 seconds, cooldown on 429/network/unavailability. Old `[vision].models` is
ignored; quotas are checked in AI Studio rather than guaranteed in code.

`read_window_text` reads visible TextPattern text only on a direct request;
`inspect_window` reads buttons. Terminal `type_text` focuses the text area and
verifies insertion; failure or a changed window blocks subsequent keys in the
same request. Inserted text does not prove command execution.
`tools/voice-server/speech_text.py` prepares English names only for F5 before
RUAccent; STT, displayed text, and action arguments retain their original text.

## Build and verification

The target is Windows x64; local development is Linux. Pass `DOCS_RS=1` to avoid
ort-sys linking/downloads during Linux checks. `.codex/hooks/session-start.sh`
checks Rust, Node 22, language servers, `libasound2-dev`, `libssl-dev`,
`pkg-config`, mingw, and the Windows target. Installation is separate and uses
explicit `--install` authorization.

```sh
DOCS_RS=1 cargo check -p jarvis-core
DOCS_RS=1 cargo check -p jarvis-app --target x86_64-pc-windows-gnu
DOCS_RS=1 cargo test -p jarvis-core --no-default-features --features reqwest --lib
DOCS_RS=1 cargo test -p jarvis-core --no-default-features --features reqwest,lua,ipc --lib
cd tools/voice-server && uv run --no-project --python 3.12 --with pytest --with numpy --with soundfile python -m pytest -q
```

`.github/workflows/windows.yml` builds the executable and `JarvisSetup.exe`
with Inno Setup (`installer/jarvis.iss`, `installer/configure.ps1`). It also
builds whisper.cpp with Vulkan and smoke-tests `WhisperCppRecognizer` on CPU.
The installer runs `install.ps1 -Installer` without a console through
`ExecAndLogOutput`; progress includes `==>` steps and pip `--progress-bar raw`,
with output in `voice-install.log`. Real audio, microphone, and Windows actions
require an actual PC.

GUI changes require `.agents/skills/gui-check/SKILL.md` and viewed Playwright
screenshots using mocked Tauri/jarvis-app. The main orb is
`frontend/src/components/elements/VoiceOrb.svelte`;
`IpcEvent::AudioLevel` supplies level/16 spectrum bands at about 30 Hz and
`IpcEvent::Speaking` supplies speech state. Store check artifacts under `.codex/`.
Parse PowerShell with `[System.Management.Automation.Language.Parser]::ParseFile`;
use UTF-8 BOM and CRLF for Windows PowerShell 5.1 Cyrillic compatibility.

## Runtime, logs, and updates

- `%APPDATA%\com.priler.jarvis\assistant.toml` stores user settings; its template
  is `crates/jarvis-core/assets/assistant.default.toml`, created on first run.
  The same directory holds `log.txt`, `gui-log.txt`, and `voice-server.log`.
  `ui_log` records GUI events; app logs begin with version/build. Start bug
  investigation there. `collect_logs` exports a desktop ZIP; `mask_secrets`
  masks `keys`, `AIza`, `AQ.`, `eyJ`, and `sk-` values.
- CI embeds `JARVIS_VERSION=0.2.<run_number>` and `JARVIS_BUILD=<sha>` via
  `option_env!` and publishes `version.json` in `latest`. `check_update` launches
  `JarvisSetup.exe /SILENT`; silent install uses `-RuntimeOnly`, preserves other
  settings, and restarts the app. Inno `[Run]` precedes `ssPostInstall`; app/GUI
  launch needs `postinstall` to follow voice setup and the last StopJarvis.
  `installer/tests/lifecycle.Tests.ps1` verifies this with real installer flow,
  fake executables, and delayed setup without downloading models.
- `Local\JarvisVoiceAssistantApp` permits one app per session; a new copy waits
  eight seconds. GUI `restart_jarvis_app` / `stop_jarvis_app` terminate the app
  process rather than using IPC, which is off on the settings page. The voice
  server exclusively binds its port before model loading and exits 20 seconds
  after the app disappears with `--exit-with-app`.
- `[assistant] address` reaches the LLM prompt and non-default HTTP-TTS replies.
  `phrases.rs` caches them under `phrases\` using the F5/text FNV namespace;
  `voices::play` prefers them to recordings. Installer `-Address sir|miss`
  feeds `configure.ps1`, which routes keys, migrates old gateway blocks,
  rejects Kilo keys, joins migrated lines, and writes key-free `configure.log`.
- `audio::hold_microphone` suppresses self-recognition through synthesis and
  playback. Cloned speech can allow wake-only interruption through
  `app::wait_for_wake_word`, `audio::stop_speaking`, and `[tts] barge_in`,
  including partial results, synthesis waits, and gaps between sentences.
  A name in an answer does not disable listening; the prompt avoids speaking
  its own name, and barge-in synthesis alone replaces it with «ассистент».
  UI/history retain the original. Partial wake needs 200 ms of matching audio
  samples; finalized wake interrupts immediately. There is no full acoustic
  echo cancellation; speaker-induced interruptions need hardware testing.
  PvRecorder reads continuously on its own thread and keeps only three fresh
  frames (up to 100 ms); flush before command and interruption listening.
  The server is the single automatic path without `stt.engine`; Vosk remains
  activation/fallback. Reject contradictory open/close and save/do-not-save
  substitutions. Reuse HTTP clients; LLM calls share two `timeout_secs` capped
  at 60 seconds. Tool failure is not success; preserve an action's result if
  its final LLM response is lost.
- Runtime DLLs such as `pv_recorder` need explicit paths relative to `APP_DIR`;
  build-time `OUT_DIR` paths do not work on the user's PC.

## Engineering constraints

- Do not run repository-wide `cargo fmt`: upstream formatting is intentionally
  preserved. Add new logic in modules such as `actions`, `llm`, `tts`, and
  `assistant_config`, with narrow changes to upstream files.
- Guard Windows APIs with `#[cfg(windows)]` and Linux fallbacks.
- Pass speech/LLM input as process arguments or environment values through
  `platform::powershell(script, env)`, never interpolate it into shell scripts.
- File actions stay inside `[safety].allowed_dirs`; deletion uses the Recycle Bin.
  `PROTECTED_PROCESSES` forbids closing system processes.
- New behavior needs Linux-runnable unit tests; HTTP mocks use
  `std::net::TcpListener` as in `llm.rs`.
- For mlua, kira, reqwest, tauri, and vosk API questions use Context7 with
  versions from `Cargo.lock`, or the matching official documentation.
- `LICENSE.txt` is CC BY-NC-SA 4.0; preserve upstream attribution and licensing. Do not propose commercial use.

## Git

Author and committer must be the user from `git config user.name` and `git config user.email`.

Work in the fork's `master`; do not send upstream PRs without a request.
Keep one completed task per commit, with a short subject and one or two body
lines. Preserve existing co-authors when replacing a task commit. After push,
follow `.agents/skills/windows-build/SKILL.md` to check CI.

## Procedures and evidence

For completed behavior, integration, or check implementation, invoke the
read-only reviewer in `.codex/agents/reviewer.toml` or
`.grok/agents/reviewer.md` after verification. Pass its instructions if named
roles are unsupported. Fix substantive findings and request review of the
changed portion. Typographical/format-only edits do not require a reviewer.

Multi-step requirements belong in `.codex/plans/` with IDs, observable outcomes,
status, and evidence. `.codex/tools/check_evidence.py` validates manifests;
the example is `.codex/plans/development-2026-10-05/requirements.json`.
`verified` needs an artifact, SHA256, method, conclusion, `result: pass`, and
`inputs` hashes; executable checks also need `exit_code: 0`. Independent review
needs a different reviewer and JSON `verdict: approve`. Hardware evidence needs
`provenance: observed`, `result: pass`, and `environment`; unavailable hardware
stays `hardware_pending`. Completion check:
`python3 .codex/tools/check_evidence.py MANIFEST --require-complete`.
Confirmed bugs become request/outcome scenarios under `.codex/scenarios/`,
validated by `.codex/tools/check_scenarios.py`; fixtures validate the mechanism,
not an unfixed historical behavior.

| Task | Canonical skill |
|---|---|
| Voice commands, actions, Lua packs | `.agents/skills/add-voice-command/SKILL.md` |
| LLM providers, keys, tool calling | `.agents/skills/llm-providers/SKILL.md` |
| Windows CI, DLLs, installer | `.agents/skills/windows-build/SKILL.md` |
| GUI, screenshots, console, copy | `.agents/skills/gui-check/SKILL.md` |
| Logs, STT/TTS/IPC, action failures | `.agents/skills/review-logs/SKILL.md` |
| Speech/voice model comparisons | `.agents/skills/benchmark-voice/SKILL.md` |

Read a matching skill even if absent from the session catalog; Grok adapters
reach these shared procedures through the existing compatibility entry points. `.codex/hooks/guard-bash.py` and its Grok
wrapper guard destructive paths and commands, but are not complete shell
parsers. Native hooks cannot request approval with `permissionDecision = "ask"`;
an already authorized operation still needs a solution within its exact scope.
Keep Codex memory/plans/agents under `.codex/`; Grok configuration under `.grok/`.
The root file is loaded by `.codex/config.toml`, its compatibility entry point,
and Grok's native `AGENTS.md` discovery. Grok adapters live in `.grok/rules/`.
