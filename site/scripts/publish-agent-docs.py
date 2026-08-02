#!/usr/bin/env python3

"""Publish and verify the raw files used by agents."""

from pathlib import Path
import re
import shutil
import sys


SITE = Path(__file__).resolve().parents[1]
DIST = SITE / ".vitepress" / "dist"
LLMS_SOURCE = SITE / "public" / "llms.txt"
CONTENT = ("concepts", "guide", "operations", "project", "reference")
LINK_RE = re.compile(r"\]\(\s*([^\s)]+)")


def markdown_sources() -> list[Path]:
    """Return all public Markdown source files."""
    sources = [SITE / "index.md"]
    for directory in CONTENT:
        sources.extend(sorted((SITE / directory).rglob("*.md")))
    return sources


def publish() -> list[str]:
    """Copy raw Markdown into the built site and verify the agent index."""
    errors: list[str] = []
    if not DIST.is_dir():
        return [f"build output not found: {DIST}"]

    for source in markdown_sources():
        if source.is_symlink():
            errors.append(f"refusing symlink: {source.relative_to(SITE)}")
            continue
        target = DIST / source.relative_to(SITE)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)

    published_llms = DIST / "llms.txt"
    if not LLMS_SOURCE.is_file() or LLMS_SOURCE.is_symlink():
        errors.append("public/llms.txt is missing or is a symlink")
        return errors
    if not published_llms.is_file():
        errors.append("llms.txt was not published by VitePress")
        return errors
    if published_llms.read_bytes() != LLMS_SOURCE.read_bytes():
        errors.append("published llms.txt differs from public/llms.txt")

    dist_root = DIST.resolve()
    for target in LINK_RE.findall(LLMS_SOURCE.read_text(encoding="utf-8")):
        if "://" in target or target.startswith(("mailto:", "#")):
            continue
        if target.startswith("/"):
            errors.append(f"llms.txt has a root-relative link: {target}")
            continue
        path_part = target.split("#", 1)[0]
        if Path(path_part).suffix != ".md":
            errors.append(f"llms.txt local link is not Markdown: {target}")
            continue
        resolved = (DIST / path_part).resolve()
        if not resolved.is_relative_to(dist_root):
            errors.append(f"llms.txt link escapes the site: {target}")
        elif not resolved.is_file():
            errors.append(f"llms.txt link does not resolve: {target}")

    for source in markdown_sources():
        target = DIST / source.relative_to(SITE)
        if target.is_file() and target.read_bytes() != source.read_bytes():
            errors.append(f"published Markdown differs: {source.relative_to(SITE)}")

    return errors


errors = publish()
if errors:
    print("\n".join(errors))
    sys.exit(1)

print(f"Published {len(markdown_sources())} Markdown files and llms.txt.")
