---
name: benchmark-voice
description: "Research and compare Jarvis ASR and voice-cloning models for Russian quality, latency, and Windows/CUDA/ROCm compatibility."
---

# Compare speech models

Use available model/dataset/paper search and developer repositories. Record
queries, model IDs/revisions, language, license, size, streaming mode,
dependencies, and actually supported hardware. Distinguish author benchmarks
from measurements on the target Windows/AMD system.

Use `.codex/scenarios/voice-corpus.json`: numbers, paths, negation, questions,
and short/long answers. TTS candidates share the same Jarvis New reference
and exact transcript; ASR candidates share WAVs and reference transcriptions.
Popularity alone does not justify changing the production synthesizer.

For an existing Jarvis `/health` and `/tts` server, inspect
`python3 .codex/tools/benchmark_voice.py --help`. Check consistent F5 profile,
GPU, and synthesis-device reporting. Historical Nano results record CPU,
revision, and sample hash separately from the recognition hardware profile.
Keep the full WAV, HTTP elapsed time, audio duration, RTF, and separate warmup.
`observed_http` establishes the HTTP measurement; model/sample identity is
reported, and `hardware_verified: false` does not verify the GPU. Test fixtures
use `fixture` provenance. First HTTP byte is not first audible sound; streaming
needs a separate adapter and first-playable-block measurement.

Measure VRAM, voice similarity, pronunciation errors, and subjective quality
with their methods; mark missing measurements explicitly. Test dependencies
apart from the running server. Paid cloud jobs need separate authorization.
Return candidates, evidence for claimed benefits, measured/unknown metrics,
the next test, and replacement criteria. This skill does not authorize an
automatic production model change.
