---
name: review-logs
description: "Analyze Jarvis log archives, separate versions and sessions, and produce sanitized evidence for startup, STT/TTS, IPC, and action failures."
---

# Review logs

From the root run `python3 .agents/tools/review_logs.py ARCHIVE --output
.opencode/.tmp/log-review.json`; add `--date YYYY-MM-DD` or `--version 0.2.N`
when useful. The analyzer reads ZIP content without extracting it and emits
allowlisted fields.

Establish version/build and agent mode; separate app launches and old-version
failures from current failures. Compare the latest failure with later successes
and current execution code. For each finding retain the phrase, time/line,
observed action, expected result, confidence, and fix criterion.
A keypress does not prove saving; an old file is not a new result; a user note
does not prove changed settings. Without audio/screen evidence, do not establish
false activations or window contents.

Inspect analyzer output before publication; keep secrets and complete dialogs
out of commits. Do not make LLM/network requests just to reproduce an archived
failure. Use `.agents/scenarios/README.md` for scenarios and root `AGENTS.md`
with `check_evidence.py` for requirement completion.
