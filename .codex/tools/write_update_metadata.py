"""Generate release metadata using the repository baked into the updater."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re


def release_metadata(installer, environment):
    repository = environment.get("JARVIS_UPDATE_REPOSITORY", "")
    parts = repository.split("/")
    if len(parts) != 2 or any(
        part in ("", ".", "..") or re.fullmatch(r"[A-Za-z0-9._-]+", part) is None
        for part in parts
    ):
        raise ValueError("JARVIS_UPDATE_REPOSITORY must contain owner/repository")
    version = environment.get("JARVIS_VERSION", "")
    build = environment.get("JARVIS_BUILD", "")
    if not version or not build:
        raise ValueError("JARVIS_VERSION and JARVIS_BUILD are required")
    with installer.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    return {
        "version": version,
        "build": build,
        "setup": f"https://github.com/{repository}/releases/download/latest/JarvisSetup.exe",
        "sha256": digest,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("installer", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        metadata = release_metadata(args.installer, os.environ)
    except (ValueError, OSError) as error:
        parser.exit(1, f"Cannot generate update metadata: {error}\n")
    args.output.write_text(json.dumps(metadata, indent=2) + "\n", encoding="ascii")


if __name__ == "__main__":
    main()
