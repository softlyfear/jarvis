"""Run the shared Bash guard and protect Grok's project directory.

The shared guard denies catastrophic commands. Codex turns an ask verdict into
deny because its hook cannot ask. Grok can ask, so this wrapper keeps that
verdict and also treats .grok like the other guarded agent directories.
"""

import importlib.util
import json
import os
import sys
from pathlib import Path

HOOKS = Path(__file__).resolve().parents[2] / ".codex" / "hooks"
sys.path.insert(0, str(HOOKS))
SPEC = importlib.util.spec_from_file_location("shared_guard", HOOKS / "guard-bash.py")
assert SPEC and SPEC.loader
guard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guard)
shared_wipes_project = guard._wipes_project


def wipes_project(target: str, cwd: str, project: str) -> bool:
    base = target[:-1] if target == "*" or target.endswith("/*") else target
    path = os.path.normpath(os.path.join(cwd, os.path.expanduser(base or ".")))
    grok_dir = os.path.join(project, ".grok")
    if path == grok_dir or path.startswith(grok_dir + os.sep):
        return True
    return shared_wipes_project(target, cwd, project)


guard._wipes_project = wipes_project


def main() -> None:
    data = json.load(sys.stdin)
    tool_input = data.get("tool_input") or data.get("toolInput") or {}
    command = tool_input.get("command") or tool_input.get("cmd") or ""
    cwd = data.get("cwd") or guard.PROJECT_ROOT
    workdir = tool_input.get("workdir")
    if workdir:
        cwd = os.path.normpath(os.path.join(cwd, workdir))
    verdict = guard.check(command, cwd)
    if verdict is None:
        return
    decision, reason = verdict
    if decision == "ask":
        reason += "; requires explicit user authorization"
    reason = f"guard-bash: {reason}"
    sys.stdout.write(json.dumps({
        "decision": decision,
        "reason": reason,
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": decision,
            "permissionDecisionReason": reason,
        },
    }))


if __name__ == "__main__":
    main()
