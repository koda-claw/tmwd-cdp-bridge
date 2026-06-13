#!/usr/bin/env python3
"""Minimal repository-local skill validator for CI."""

from __future__ import annotations

import re
import sys
from pathlib import Path


def fail(message: str) -> None:
    print(f"skill validation failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def validate_no_unforbidden_link(path: Path, text: str) -> None:
    forbids_link = re.search(
        r"(do not|never)\s+(call|use)\s+`?/link`?", text, flags=re.I
    )
    if "/link" in text and not forbids_link:
        fail(f"legacy /link mention in {path} must be explicitly forbidden")


def validate_local_markdown_links(skill_dir: Path, path: Path, text: str) -> None:
    for target in re.findall(r"\[[^\]]+\]\(([^):#][^):#]*\.md)(?:#[^)]+)?\)", text):
        linked = (path.parent / target).resolve()
        try:
            linked.relative_to(skill_dir.resolve())
        except ValueError:
            continue
        if not linked.is_file():
            fail(f"broken local markdown link in {path}: {target}")


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
    validate_no_unforbidden_link(skill, body)
    validate_local_markdown_links(skill_dir, skill, body)
    for markdown in skill_dir.rglob("*.md"):
        if markdown == skill:
            continue
        markdown_text = markdown.read_text(encoding="utf-8")
        validate_no_unforbidden_link(markdown, markdown_text)
        validate_local_markdown_links(skill_dir, markdown, markdown_text)
    print("Skill is valid!")


if __name__ == "__main__":
    main()
