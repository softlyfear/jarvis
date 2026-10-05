"""Validate action postconditions without treating fixtures as hardware evidence."""

import argparse
import hashlib
import json
from pathlib import Path, PureWindowsPath
import re

ROOT = Path(__file__).resolve().parents[2]
CASES = ROOT / ".codex/scenarios/cases.json"


def observation_ok(event, case, trace):
    kind = event.get("kind")
    if kind in ("file_saved", "file_opened"):
        inputs = trace.get("inputs", {})
        desktop = inputs.get("desktop_dir", "") if isinstance(inputs, dict) else ""
        if not isinstance(desktop, str) or not PureWindowsPath(desktop).is_absolute() or not isinstance(event.get("path"), str):
            return False
        expected_path = PureWindowsPath(desktop) / case["target_filename"]
        if PureWindowsPath(event.get("path", "")) != expected_path:
            return False
    if kind == "file_saved":
        digest = event.get("content_sha256", "")
        return event.get("changed") is True and isinstance(digest, str) and bool(re.fullmatch(r"[0-9a-f]{64}", digest)) and digest == inputs.get("expected_content_sha256")
    if kind == "file_opened":
        return event.get("opened") is True
    if kind == "browser_focused":
        return event.get("focused") is True and event.get("opened_urls") == []
    if kind == "text_replaced":
        return event.get("changed") is True and event.get("file_deleted") is False
    if kind == "voice_ready":
        return event.get("stt_ready") is True and event.get("tts_ready") is True
    if kind == "ipc_ready":
        return event.get("connected") is True and event.get("port") == 9712
    return False


def verify_source(trace, root):
    name = trace.get("source", "")
    if not isinstance(name, str):
        return False
    path = (root / name).resolve()
    if not name or Path(name).is_absolute() or not path.is_relative_to(root.resolve()):
        return False
    if not path.is_file():
        return False
    try:
        data = path.read_bytes()
        source = json.loads(data)
    except (OSError, ValueError):
        return False
    return (
        hashlib.sha256(data).hexdigest() == trace.get("source_sha256")
        and isinstance(source, dict) and source.get("schema_version") == 1
        and source.get("provenance") == "observed" and source.get("case_id") == trace.get("case_id")
        and source.get("events") == trace.get("events")
        and source.get("inputs", {}) == trace.get("inputs", {})
        and isinstance(source.get("environment"), dict) and bool(source["environment"].get("os"))
        and bool(source.get("observer"))
    )


def evaluate(trace, cases=None, root=ROOT):
    cases = cases or json.loads(CASES.read_text(encoding="utf-8"))["cases"]
    case = next((c for c in cases if c["id"] == trace.get("case_id")), None)
    result = {"case_id": trace.get("case_id"), "status": "unknown", "provenance": trace.get("provenance"), "hardware_verified": False, "reasons": []}
    if trace.get("schema_version") != 1 or not case or trace.get("provenance") not in ("fixture", "observed") or not isinstance(trace.get("events"), list):
        result.update(status="fail", reasons=["Некорректная схема трассы"])
        return result
    if trace["provenance"] == "observed" and not verify_source(trace, root):
        result.update(status="fail", reasons=["Нет проверяемого источника наблюдения"])
        return result
    verified = False
    action_seen = False
    action_failed = False
    failures = []
    for event in trace["events"]:
        if not isinstance(event, dict):
            failures.append("Некорректное событие")
            continue
        if event.get("type") == "tool":
            if not isinstance(event.get("name"), str) or not event["name"] or not isinstance(event.get("arguments"), dict) or not isinstance(event.get("ok"), bool):
                failures.append("Некорректное событие инструмента")
                continue
            if event.get("name") not in ("inspect_window", "find_files", "list_apps", "system_info"):
                action_seen = True
                verified = False
                action_failed = event.get("ok") is False
            if event.get("name") in case["forbidden_tools"]:
                failures.append("Выбрано действие, не соответствующее просьбе: " + event["name"])
        elif event.get("type") == "observation" and event.get("kind") == case["required_observation"]:
            if not action_seen:
                continue
            verified = observation_ok(event, case, trace)
            if verified:
                action_failed = False
            if not verified:
                failures.append("Наблюдаемое состояние не соответствует требованию")
        elif event.get("type") == "claim" and event.get("success") is True and not verified:
            failures.append("Объявлен успех без подтверждения результата после действия")
    if action_failed:
        failures.append("Последнее действие завершилось ошибкой без подтверждённого восстановления")
    if failures:
        result.update(status="fail", reasons=failures)
    elif verified:
        result.update(status="pass", hardware_verified=trace["provenance"] == "observed")
    else:
        result["reasons"] = ["Недостаточно наблюдений конечного состояния"]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trace", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    data = json.loads(args.trace.read_text(encoding="utf-8"))
    traces = data.get("traces", [data]) if isinstance(data, dict) else []
    if not isinstance(traces, list) or not traces:
        print(json.dumps({"schema_version": 1, "results": [], "error": "Нет сценариев для проверки"}, ensure_ascii=False))
        return 1
    results = [evaluate(t) if isinstance(t, dict) else {"status": "fail", "reasons": ["Некорректная трасса"]} for t in traces]
    report = {"schema_version": 1, "results": results}
    text = json.dumps(report, ensure_ascii=False, indent=2)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text + "\n", encoding="utf-8")
    print(text)
    return 0 if all(r["status"] == "pass" for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
