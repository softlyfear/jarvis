"""Reject requirements marked verified without intact, successful evidence."""

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
KINDS = {"test", "review", "mcp", "research", "hardware"}


def successful_mcp(payload):
    if payload.get("result") != "pass" or payload.get("exit_code") != 0:
        return False
    if payload.get("provenance") == "observed_stdio":
        servers = payload.get("servers")
        if not isinstance(servers, list) or not servers:
            return False
        for server in servers:
            if not isinstance(server, dict) or server.get("result") != "pass":
                return False
            calls = server.get("calls")
            if not isinstance(calls, list) or not calls:
                return False
            for call in calls:
                if not isinstance(call, dict) or call.get("result") != "pass" or not isinstance(call.get("tool"), str) or not call["tool"] or not isinstance(call.get("arguments"), dict):
                    return False
        return True
    if payload.get("provenance") == "observed_http_mcp":
        response = payload.get("response")
        if not (isinstance(payload.get("tool"), str) and payload["tool"] and isinstance(payload.get("arguments"), dict) and isinstance(response, dict) and response.get("isError") is False and isinstance(response.get("content"), list) and response["content"]):
            return False
        if payload["tool"] == "hf_fs":
            operations = payload["arguments"].get("operations")
            structured = response.get("structuredContent")
            results = structured.get("results") if isinstance(structured, dict) else None
            if not isinstance(operations, list) or not operations or not isinstance(results, list) or len(results) != len(operations):
                return False
            return all(isinstance(item, dict) and item.get("index") == index and item.get("status") == "success" and isinstance(item.get("result"), dict) and item["result"] for index, item in enumerate(results))
        return True
    return False


def validate(document, root=ROOT, require_complete=False):
    errors = []
    if not isinstance(document, dict) or document.get("schema_version") != 1 or not isinstance(document.get("tasks"), list) or not document["tasks"] or not document.get("implementer"):
        return ["Некорректная схема требований"]
    seen = set()
    for task in document["tasks"]:
        if not isinstance(task, dict):
            errors.append("Некорректная задача")
            continue
        if task.get("status") not in ("pending", "in_progress", "complete"):
            errors.append(str(task.get("id")) + ": неизвестный статус задачи")
        requirements = task.get("requirements", [])
        if not isinstance(requirements, list) or not requirements:
            errors.append(str(task.get("id")) + ": нет требований")
            continue
        for req in requirements:
            if not isinstance(req, dict):
                errors.append("Некорректное требование")
                continue
            ident = req.get("id", "")
            if not isinstance(ident, str) or not ident:
                errors.append("Пустой или некорректный ID требования")
                continue
            if ident in seen:
                errors.append(ident + ": повторный ID")
            seen.add(ident)
            if req.get("status") not in ("pending", "in_progress", "implemented", "verified", "hardware_pending"):
                errors.append(ident + ": неизвестный статус")
            if req.get("status") != "verified":
                if require_complete:
                    errors.append(ident + ": требование не проверено")
                continue
            evidence = req.get("evidence", [])
            if not isinstance(evidence, list) or not evidence:
                errors.append(ident + ": нет доказательств")
                continue
            for item in evidence:
                if not isinstance(item, dict):
                    errors.append(ident + ": некорректное доказательство")
                    continue
                name = item.get("artifact", "")
                path = (root / name).resolve() if isinstance(name, str) else root
                if not isinstance(name, str) or not name or Path(name).is_absolute() or not path.is_relative_to((root / ".codex").resolve()) or not path.is_file():
                    errors.append(ident + ": недоступный артефакт")
                    continue
                if item.get("kind") not in KINDS or not item.get("summary"):
                    errors.append(ident + ": нет типа/описания проверки")
                if hashlib.sha256(path.read_bytes()).hexdigest() != item.get("sha256"):
                    errors.append(ident + ": артефакт изменился после проверки")
                if item.get("result") != "pass":
                    errors.append(ident + ": отрицательный или неизвестный результат")
                if item.get("kind") in ("test", "mcp", "hardware") and item.get("exit_code") != 0:
                    errors.append(ident + ": проверка не завершилась успешно")
                if item.get("kind") == "review" and (not item.get("reviewer") or item["reviewer"] == document["implementer"]):
                    errors.append(ident + ": ревью не независимо")
                if item.get("kind") in ("review", "hardware", "mcp"):
                    try:
                        payload = json.loads(path.read_text(encoding="utf-8"))
                    except (OSError, ValueError):
                        payload = {}
                    if not isinstance(payload, dict):
                        payload = {}
                    if item["kind"] == "review" and (payload.get("verdict") != "approve" or payload.get("reviewer") != item.get("reviewer")):
                        errors.append(ident + ": независимое ревью не одобрило реализацию")
                    if item["kind"] == "hardware" and (item.get("provenance") != "observed" or payload.get("provenance") != "observed" or payload.get("result") != "pass" or not payload.get("environment")):
                        errors.append(ident + ": нет успешной аппаратной проверки")
                    if item["kind"] == "mcp" and not successful_mcp(payload):
                        errors.append(ident + ": нет успешного вызова инструмента MCP; discovery не является приёмкой")
                inputs = item.get("inputs", [])
                if not isinstance(inputs, list) or not inputs:
                    errors.append(ident + ": нет проверенной области реализации")
                    continue
                for source in inputs:
                    if not isinstance(source, dict):
                        errors.append(ident + ": некорректная область проверки")
                        continue
                    source_name = source.get("path", "")
                    source_path = (root / source_name).resolve() if isinstance(source_name, str) else root
                    if not isinstance(source_name, str) or not source_name or Path(source_name).is_absolute() or not source_path.is_relative_to(root.resolve()) or not source_path.is_file():
                        errors.append(ident + ": недоступный проверенный исходник")
                    elif hashlib.sha256(source_path.read_bytes()).hexdigest() != source.get("sha256"):
                        errors.append(ident + ": реализация изменилась после проверки")
        if task.get("status") == "complete" and any(not isinstance(r, dict) or r.get("status") != "verified" for r in requirements):
            errors.append(str(task.get("id")) + ": задача закрыта при незакрытых требованиях")
        if require_complete and task.get("status") != "complete":
            errors.append(str(task.get("id")) + ": задача не завершена")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--require-complete", action="store_true")
    args = parser.parse_args()
    errors = validate(json.loads(args.manifest.read_text(encoding="utf-8")), require_complete=args.require_complete)
    print(json.dumps({"ok": not errors, "errors": errors}, ensure_ascii=False, indent=2))
    return int(bool(errors))


if __name__ == "__main__":
    raise SystemExit(main())
