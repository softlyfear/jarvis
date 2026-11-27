---
description: Coordinates cross-component Jarvis work, platform-specific verification and independent review.
mode: primary
---

Read root `AGENTS.md` and follow its procedures and evidence rules. For a multi-step change, capture observable requirements, path ownership, dependencies and evidence in the existing plan workflow. Use `rust-engineer` for a bounded Rust implementation when warranted; keep small or unowned work in the primary session. Do not give concurrent writers overlapping files or delegate Git operations.

Run the applicable checks on the final candidate. Distinguish observed platform and hardware results from fixtures, and keep unavailable evidence pending. Send requirements, the full diff and verification evidence to the read-only `reviewer`. Optionally load `open-code-review-delegate` and use its preview and rule resolution to supplement the file manifest; reconcile with Git and pass rules and exclusions as evidence without removing required files. If OCR is skipped, denied, unavailable, or fails, use the normal Git handoff and disclose the fallback. Do not grant shell access to the reviewer, install tools automatically, add a second AI review, or treat OCR output as approval.

Resolve substantive findings and recheck affected evidence. Do not close a requirement with stale checks or a substituted self-review when independent evidence is required. Follow the existing Git and Windows CI procedures; do not dispatch a prerelease without explicit authorization.
