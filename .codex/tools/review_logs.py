"""Summarize selected Jarvis sessions directly from ZIP, with secret redaction."""

import argparse
from datetime import datetime
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import statistics
import tomllib
import zipfile

MAX_ARCHIVE_BYTES = 64 * 1024 * 1024
ALLOWED = {"log.txt", "gui-log.txt", "voice-server.log", "assistant.toml", "configure.log", "update-install.log", "voice-install.log", "voice-update.log"}
SENSITIVE = re.compile(r"key|token|secret|password|authorization|cookie|signature", re.I)
PREFIX = re.compile(r"\b(?:sk-[\w-]{8,}|AIza[\w-]{12,}|eyJ[\w.-]{12,}|AQ\.[\w.-]{12,})")
ASSIGNMENT = re.compile(r"((?:api[_-]?key|keys?|token|secret|password|authorization|cookie|thought_signature)\s*[\"']?\s*[:=]\s*)(\[[^\]]*\]|\"[^\"]*\"|'[^']*'|[^\s,;]+)", re.I)
START = re.compile(r"Starting Jarvis v([^\s]+) \(build ([^)]*)\)")
LINE = re.compile(r"^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+)\s+\[(\w+)\s*\].*?>:(.*)$")


def secret_values(value, sensitive=False):
    if isinstance(value, dict):
        return [s for k, v in value.items() for s in secret_values(v, sensitive or bool(SENSITIVE.search(k))) ]
    if isinstance(value, list):
        return [s for v in value for s in secret_values(v, sensitive)]
    return [value] if sensitive and isinstance(value, str) and len(value) >= 6 else []


def redact(text, secrets=()):
    for value in sorted(set(secrets), key=len, reverse=True):
        text = text.replace(value, "[скрыто]")
    text = re.sub(r"(authorization\s*[:=]\s*[\"']?(?:Bearer\s+|Basic\s+))[^\s\"',;]+", r"\1[скрыто]", text, flags=re.I)
    return ASSIGNMENT.sub(lambda m: m[1] + '"[скрыто]"', PREFIX.sub("[скрыто]", text))


def read_archive(path):
    contents = {}
    with zipfile.ZipFile(path) as archive:
        if sum(i.file_size for i in archive.infolist()) > MAX_ARCHIVE_BYTES:
            raise ValueError("Архив превышает лимит распакованных данных")
        for info in archive.infolist():
            name = PurePosixPath(info.filename)
            if name.is_absolute() or ".." in name.parts or "\\" in info.filename:
                raise ValueError("Небезопасный путь в архиве")
            if len(name.parts) == 1 and info.filename in ALLOWED:
                if info.filename in contents:
                    raise ValueError("Повторное имя файла в архиве")
                contents[info.filename] = archive.read(info).decode("utf-8-sig", errors="replace")
    if "log.txt" not in contents:
        raise ValueError("В архиве нет log.txt")
    return contents


def analyze(contents, date=None, version=None):
    try:
        config = tomllib.loads(contents.get("assistant.toml", ""))
    except tomllib.TOMLDecodeError:
        config = {}
    secrets = secret_values(config)
    sessions = []
    current = None
    for number, raw in enumerate(contents["log.txt"].splitlines(), 1):
        match = LINE.match(raw)
        if not match:
            continue
        stamp, level, message = match.groups()
        start = START.search(message)
        if start:
            current = {"started": stamp, "version": start[1], "build": start[2], "start_line": number, "errors": [], "events": [], "stt_ms": [], "tts_to_playback_seconds": []}
            sessions.append(current)
        if current is None or (date and not stamp.startswith(date)) or (version and current["version"] != version):
            continue
        text = redact(message, secrets)
        if level in ("WARN", "ERROR"):
            current["errors"].append({"time": stamp, "line": number, "level": level, "message": text})
        # Usage objects, prompts and arbitrary debug data are deliberately not exported.
        if any(marker in message for marker in ("Recognized voice:", "Intent recognized:", "LLM tool call:", "LLM reply:", "Opening:", "Action ")):
            current["events"].append({"time": stamp, "line": number, "message": text})
        whisper = re.search(r"(?:Whisper|Speech server) \((\d+) ms\)", message)
        if whisper:
            current["stt_ms"].append(int(whisper[1]))
        if "TTS (http):" in message:
            current["_pending_tts"] = datetime.strptime(stamp[:26], "%Y-%m-%d %H:%M:%S.%f")
        elif "TTS failed:" in message:
            current.pop("_pending_tts", None)
        elif "Playing " in message and "jarvis-tts-" in message and "_pending_tts" in current:
            delta = datetime.strptime(stamp[:26], "%Y-%m-%d %H:%M:%S.%f") - current.pop("_pending_tts")
            current["tts_to_playback_seconds"].append(round(delta.total_seconds(), 6))
    filtered = []
    for session in sessions:
        session.pop("_pending_tts", None)
        if (date and not session["started"].startswith(date) and not session["events"] and not session["errors"]) or (version and session["version"] != version):
            continue
        for field in ("stt_ms", "tts_to_playback_seconds"):
            values = session.pop(field)
            session[field] = {"count": len(values), "median": statistics.median(values) if values else None, "min": min(values) if values else None, "max": max(values) if values else None}
        filtered.append(session)
    safe = {
        "backend": config.get("agent", {}).get("backend"),
        "browser_alias": redact(str(config.get("apps", {}).get("браузер", "")), secrets),
    }
    server = contents.get("voice-server.log", "")
    ready = [redact(line, secrets) for line in server.splitlines() if line.startswith("[server] ready")]
    return {"schema_version": 1, "filters": {"date": date, "version": version}, "configuration": safe, "sessions": filtered, "voice_ready_history": ready[-3:], "limits": ["Архивный voice-server.log не всегда содержит время; строки ready не привязаны к выбранной сессии", "TTS-to-playback не включает ожидание LLM и не является отдельным benchmark модели", "Журнал не доказывает содержимое файла или состояние окна после действия"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--date")
    parser.add_argument("--version")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.date:
        datetime.strptime(args.date, "%Y-%m-%d")
    report = analyze(read_archive(args.archive), args.date, args.version)
    report["archive_sha256"] = hashlib.sha256(args.archive.read_bytes()).hexdigest()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"sessions": len(report["sessions"]), "errors": sum(len(s["errors"]) for s in report["sessions"]), "output": str(args.output)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
