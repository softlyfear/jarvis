---
name: windows-build
description: "Build, package, and diagnose Jarvis Windows CI, DLLs, and the Inno Setup installer; check CI after an authorized push."
---

# Windows build and packaging

Read root `AGENTS.md` and `.github/workflows/windows.yml`.
Linux/Windows unit tests and frontend `npm ci`, `npm test`, and `npm run build`
are CI checks. Frontend `npm run check` generates Routify routes before type
checking; retain the custom `[...404].svelte` fallback.
Build `jarvis-app` and `jarvis-gui` separately with MSVC: combined invocations
unify core features and incorrectly link GUI to Vosk.

`crates/jarvis-app/build.rs` selects the Vosk import library;
`lib/windows/amd64/*.dll` belongs beside executables. ONNX Runtime is downloaded
by `ort` in CI. Package executables, required DLLs, resources (Russian Vosk
only), tools, and `ИНСТРУКЦИЯ.md`. Inno Setup uses `installer/jarvis.iss` and
`ISCC /DAppVersion=...`; `installer/configure.ps1` receives keys through a
temporary file rather than command arguments. The workflow publishes a fixed
`latest` prerelease for the authorized release path; do not infer publication
permission from this skill.

Use an available GitHub connector or `gh run list` and
`gh run view <run-id> --log-failed` in this checkout. Do not hardcode a personal
repository URL. Linux `DOCS_RS=1 cargo check -p jarvis-app --target
x86_64-pc-windows-gnu` checks types/Windows guards; MSVC linking, DLL loading,
and packaging need CI or the target machine. Retry a presumed transient failure
only with evidence that it happened before compilation, such as checkout/download.

Do not commit executables, `target/`, or `frontend/dist`; retain required DLLs.
Do not add unnecessary en/uk models to the archive.

After an authorized push find the run for its SHA with
`gh run list --commit <sha> -L 5`, then monitor using
`gh run watch <run-id> --exit-status` without blocking communication.
Inspect failures, fix only within scope, and verify the replacement run.
Report the commit, run link, actual result, or the precise unverified limitation.
