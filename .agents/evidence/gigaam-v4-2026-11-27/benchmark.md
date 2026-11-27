# GigaAM v4 CPU comparison

Ubuntu 26.04.1 x64, Intel Core i5-13420H, Python 3.12.13,
ONNX Runtime 1.32.0, onnx-asr 0.12.0. Recognition was evaluated on
the same 16 kHz mono PCM16 utterances for each model.

- Baseline: `istupakov/gigaam-multilingual-large-ctc-onnx`,
  `07665ab5e54371dd1ac7b8b10f06478003723573`, INT8.
- Candidate: `ai-babai/gigaam-v4-onnx`,
  `868283a297ad4a85b73ad06a080950d0e0aec07e`, CTC E2E INT8.
  Files total 242,537,846 bytes versus 591,646,507 for the baseline.
- Alternate: `Altushkins/GigaAM-v4-onnx`,
  `87a0325922b703c1ab54bea525163f90880d3358`, RNN-T INT8.

| Corpus | Utterances | Baseline WER | v4 CTC WER | v4 RNN-T WER |
|---|---:|---:|---:|---:|
| FLEURS Russian test | 100 | 149/1943 (7.67%) | 83/1943 (4.27%) | 77/1943 (3.96%) |
| FLEURS English test | 100 | 271/2167 (12.51%) | 141/2167 (6.51%) | 129/2167 (5.95%) |
| Golos Farfield test, up to 8 words | 100 | 20/411 (4.87%) | 18/411 (4.38%) | 13/411 (3.16%) |

FLEURS uses a seeded sample with distinct transcript IDs (seed 470);
Golos uses a seeded sample of short queries to smart devices (seed 471).
Paired-bootstrap WER difference, v4 CTC minus baseline: Russian FLEURS
−5.10 to −1.90 percentage points, English FLEURS −7.81 to −4.23 pp,
Golos −1.93 to +0.78 pp (95% intervals). Golos exact text: 83/100 vs
86/100. In a separate run, four pinned performance cores, four inference
threads, four warmups, sequential models: Golos median 142.5 vs 163.1 ms,
p95 266.4 vs 281.2 ms. These timings cover WAV decoding and recognition,
not endpointing, HTTP, dispatch, or user-perceived latency.

Two Piper voices provided 52 short command utterances and 18 utterances
from `.agents/scenarios/voice-corpus.json`. On identical cached WAVs,
the baseline and v4 CTC matched 37/52 commands after normalizing case,
punctuation, selected numeric equivalents, and known Latin/Cyrillic brand
spellings. Without brand normalization, exact matches were 37/52 and 30/52.
Both kept all 10 expected negations. Some v4 brand outputs mixed scripts;
this was a transcript check, with no action execution.

v4 CTC was selected for the clear FLEURS accuracy improvement, comparable
short-command results and smaller bundle, accepting the modest Golos
latency increase. Linux HTTP smoke tests used the real pinned files and
both ONNX Runtime 1.32.0 and 1.23.2: offline `/health` and `/stt`,
bilingual speech and digital silence, retention and use of the old
bundle when v4 was absent, and removal of the old bundle after v4 warmed
up through both normal startup and `--download-only`. A separate online
test replaced a corrupted local vocabulary with its pinned original and
then warmed up the model. The Python server suite passed 85 tests,
including simulated cleanup permission errors and clean-install warmup
failure. The same three v4 CTC asset hashes appear in the
[exporter's Windows smoke-tested bundle][windows]. These checks do not
measure the complete Jarvis application or microphone flow on Windows.

[windows]: https://github.com/ai-babai/gigaam-onnx/blob/main/benchmarks/compatibility/precision-platforms.md
