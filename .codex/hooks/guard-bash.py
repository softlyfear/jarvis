"""Codex PreToolUse guard: deny catastrophic and unapproved destructive commands.

`sudo` and `rm` stay allowed in general; only the catastrophic forms are denied
(system and home roots, the project root and its ancestors, `.claude` and `.codex`), and a short
list of data-destroying operations needs explicit user authorization (Codex has no hook ask decision).
"""

import json
import os
import re
import shlex
import sys
from pathlib import Path

PROJECT_ROOT = str(Path(__file__).resolve().parents[2])

SYSTEM_DIRS = {
    "/",
    "/*",
    "/bin",
    "/boot",
    "/dev",
    "/etc",
    "/home",
    "/lib",
    "/lib64",
    "/opt",
    "/proc",
    "/root",
    "/sbin",
    "/srv",
    "/sys",
    "/usr",
    "/var",
}
HOME_TARGETS = {
    "~",
    "~/",
    "~/*",
    "$HOME",
    "$HOME/",
    "$HOME/*",
    "${HOME}",
    "${HOME}/",
    "${HOME}/*",
}
SEGMENT_SPLIT = re.compile(r"\|\||&&|;|\||\n|&(?!>)")
WRAPPERS = {"sudo", "doas", "env", "command", "nice", "nohup", "time", "exec"}
SUDO_ARG_OPTS = {"-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-U", "-T"}
ASSIGNMENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*=.*")
HOME = os.path.expanduser("~").rstrip("/")


def _tokens(segment: str) -> list[str]:
    try:
        words = shlex.split(segment, posix=True)
    except ValueError:
        words = segment.split()
    # Drop leading wrappers with their options (sudo -E -u root, env -i VAR=1, ...).
    while words and (words[0] in WRAPPERS or ASSIGNMENT.fullmatch(words[0])):
        wrapper, words = words[0], words[1:]
        while words and (words[0].startswith("-") or ASSIGNMENT.fullmatch(words[0])):
            takes_arg = wrapper in ("sudo", "doas") and words[0] in SUDO_ARG_OPTS
            words = words[2:] if takes_arg else words[1:]
    return words


def _is_recursive(flags: list[str]) -> bool:
    for f in flags:
        if f in ("--recursive",) or (
            f.startswith("-") and not f.startswith("--") and ("r" in f or "R" in f)
        ):
            return True
    return False


def _norm(path: str) -> str:
    if path == "/*":
        return path
    return path.rstrip("/") or "/"


def _wipes_project(target: str, cwd: str, project: str) -> bool:
    """True when the target is the project root, one of its ancestors or agent directories.

    `.claude` holds unversioned pipeline documents, so it is guarded like the root.
    """
    wipes_contents = target == "*" or target.endswith("/*")
    base = target[:-1] if wipes_contents else target
    path = os.path.normpath(os.path.join(cwd, os.path.expanduser(base or ".")))
    guarded = (project, os.path.join(project, ".claude"), os.path.join(project, ".codex"))
    return path in guarded or project.startswith(path.rstrip("/") + "/")


def check(
    command: str, cwd: str | None = None, project: str | None = None
) -> tuple[str, str] | None:
    """Return (decision, reason) or None when the command is fine."""
    cwd = cwd or os.getcwd()
    project = os.path.normpath(project or PROJECT_ROOT)
    if re.search(r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:", command):
        return "deny", "fork bomb"
    for raw in SEGMENT_SPLIT.split(command):
        words = _tokens(raw.strip())
        if not words:
            continue
        prog, args = words[0].rsplit("/", 1)[-1], words[1:]
        flags = [a for a in args if a.startswith("-")]
        targets = [a for a in args if not a.startswith("-")]

        # Track common directory changes so relative mutations stay protected.
        if prog == "cd" and len(targets) == 1:
            cwd = os.path.normpath(os.path.join(cwd, os.path.expanduser(targets[0])))
            continue
        verdict = _claude_mutation(prog, args, raw, cwd)
        if verdict:
            return verdict

        if prog == "rm":
            if "--no-preserve-root" in flags:
                return "deny", "rm --no-preserve-root"
            for t in targets:
                n = _norm(t)
                if n.startswith("/dev/") or n == "/dev":
                    return "deny", f"rm on device path {t}"
                if _is_recursive(flags) and (
                    n in SYSTEM_DIRS or t in HOME_TARGETS or n in (HOME, HOME + "/*")
                ):
                    return "deny", f"recursive rm of system or home root {t}"
                if (
                    _is_recursive(flags)
                    and re.fullmatch(r"(/[^/]+){1,2}/?\*?", t)
                    and n.split("/")[1]
                    in {d.strip("/") for d in SYSTEM_DIRS if d not in ("/", "/*")}
                    and not n.startswith(("/tmp", "/home/"))
                ):
                    return "deny", f"recursive rm inside system dir {t}"
                if _is_recursive(flags) and _wipes_project(t, cwd, project):
                    return "deny", f"recursive rm of the project or agent configuration {t}"
                if _is_recursive(flags) and re.search(r"(^|/)\.git/?$", t):
                    return "ask", f"recursive rm of git metadata {t}"
        elif prog.startswith("mkfs") or prog in ("wipefs", "fdisk", "sfdisk", "parted"):
            return "deny", f"{prog} rewrites a disk"
        elif prog == "dd" and any(
            a.startswith("of=/dev/") and a != "of=/dev/null" for a in args
        ):
            return "deny", "dd writing to a device"
        elif prog == "shred" and any(t.startswith("/dev/") for t in targets):
            return "deny", "shred on a device"
        elif (
            prog in ("chmod", "chown", "chgrp")
            and _is_recursive(flags)
            and any(_norm(t) in SYSTEM_DIRS for t in targets)
        ):
            return "deny", f"recursive {prog} on a system root"
        elif prog == "docker":
            joined = " ".join(args)
            if re.search(r"\bsystem prune\b|\bvolume (rm|prune)\b", joined):
                return "ask", f"docker {joined}: deletes data"
            if re.search(r"\bcompose\b.*\bdown\b", joined) and re.search(
                r"(^|\s)(-v|--volumes)(\s|$)", joined
            ):
                return "ask", "docker compose down -v deletes database volumes"
        elif prog == "git":
            joined = " ".join(args)
            if re.search(r"\bpush\b", joined) and re.search(
                r"(^|\s)(-f|--force|--force-with-lease\S*)(\s|$)|\s\+\S", joined
            ):
                return "ask", "force push rewrites remote history"
            if re.search(r"\breset\b.*--hard", joined):
                return "ask", "git reset --hard discards uncommitted work"
            if re.search(r"\bclean\b", joined) and re.search(r"(^|\s)-\w*f", joined):
                return "ask", "git clean -f deletes untracked files"
            if re.search(r"\b(checkout|restore)\b.*(\s--\s+\.|\s\.$)", joined):
                return "ask", "discards uncommitted changes in the working tree"
    if re.search(r">\s*/dev/(sd|nvme|vd|hd|mmcblk)", command):
        return "deny", "redirect into a block device"
    return None


def _is_claude_path(target: str, cwd: str) -> bool:
    # Lexical and resolved paths cover ordinary paths and symlink aliases.
    expanded = os.path.expandvars(os.path.expanduser(target))
    candidate = Path(os.path.normpath(os.path.join(cwd, expanded)))
    for path in (candidate, candidate.resolve()):
        if ".claude" in path.parts or path.name == "CLAUDE.md":
            return True
    return False


def _claude_mutation(prog: str, args: list[str], raw: str, cwd: str):
    operands = [a for a in args if not a.startswith("-")]
    if prog in {"rm", "mv", "chmod", "chown", "chgrp", "touch", "truncate", "shred", "rmdir"}:
        write_targets = operands
    elif prog in {"cp", "install", "ln"}:
        write_targets = operands[-1:]
        for i, arg in enumerate(args):
            if arg in {"-t", "--target-directory"} and i + 1 < len(args):
                write_targets.append(args[i + 1])
            elif arg.startswith("--target-directory="):
                write_targets.append(arg.split("=", 1)[1])
            elif arg.startswith("-t") and len(arg) > 2:
                write_targets.append(arg[2:])
    elif prog == "tee":
        write_targets = operands
    elif prog in {"sed", "perl"} and any(a.startswith("-i") or a.startswith("--in-place") for a in args):
        write_targets = operands
    elif prog == "git" and any(a in {"restore", "checkout", "clean", "reset", "apply", "am"} for a in args):
        write_targets = operands
        if any(a in {"reset", "clean"} for a in args) or "." in operands:
            return "deny", "Git operation may overwrite Claude configuration or parallel work"
    else:
        write_targets = []
    # Redirections are inspected independently of the program (including cat).
    for match in re.finditer(r"(?:>{1,2}|<>|>\|)\s*(?:\"([^\"]+)\"|'([^']+)'|([^\s;&|]+))", raw):
        write_targets.append(next(s for s in match.groups() if s is not None))
    if any(_is_claude_path(t, cwd) for t in write_targets):
        return "deny", "Claude configuration (.claude/ and CLAUDE.md) is read-only for GPT"
    return None


def check_patch(patch: str, cwd: str) -> tuple[str, str] | None:
    for match in re.finditer(r"^\*\*\* (?:Add File|Update File|Delete File|Move to): (.+)$", patch, re.MULTILINE):
        if _is_claude_path(match.group(1), cwd):
            return "deny", "Patch would modify read-only Claude configuration"
    return None


def main() -> None:
    data = json.load(sys.stdin)
    tool_input = data.get("tool_input") or {}
    command = tool_input.get("command") or tool_input.get("cmd") or tool_input.get("patch") or ""
    cwd = data.get("cwd") or PROJECT_ROOT
    # exec_command workdir overrides the session cwd.
    workdir = tool_input.get("workdir")
    if workdir:
        cwd = os.path.normpath(os.path.join(cwd, workdir))
    verdict = check_patch(command, cwd) if data.get("tool_name") == "apply_patch" else check(command, cwd)
    if verdict is None:
        return
    decision, reason = verdict
    if decision == "ask":
        reason += "; requires explicit user authorization; Codex hook ask is unsupported"
    sys.stdout.write(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": f"guard-bash: {reason}",
    }}))


if __name__ == "__main__":
    main()
