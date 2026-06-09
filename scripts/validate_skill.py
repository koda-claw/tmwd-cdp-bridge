#!/usr/bin/env python3
"""Minimal repository-local skill validator for CI."""

from __future__ import annotations

import re
import sys
from pathlib import Path


def fail(message: str) -> None:
    print(f"skill validation failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: scripts/validate_skill.py <skill-dir>")
    skill_dir = Path(sys.argv[1])
    skill = skill_dir / "SKILL.md"
    if not skill.is_file():
        fail(f"missing {skill}")
    text = skill.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        fail("SKILL.md must start with YAML frontmatter")
    match = re.match(r"^---\n(.*?)\n---\n", text, flags=re.S)
    if not match:
        fail("frontmatter must be closed by ---")
    frontmatter = match.group(1)
    for field in ("name", "description"):
        if not re.search(rf"^{field}:\s*\S", frontmatter, flags=re.M):
            fail(f"frontmatter missing {field}")
    body = text[match.end():].strip()
    if not body:
        fail("SKILL.md body is empty")
    forbids_link = re.search(
        r"(do not|never)\s+(call|use)\s+`?/link`?", body, flags=re.I
    )
    if "/link" in body and not forbids_link:
        fail("legacy /link mention must be explicitly forbidden")
    print("Skill is valid!")


if __name__ == "__main__":
    main()
