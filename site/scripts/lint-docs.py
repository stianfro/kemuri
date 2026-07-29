#!/usr/bin/env python3

from pathlib import Path
import re
import sys


SITE = Path(__file__).resolve().parents[1]
MARKDOWN_FILES = sorted(
    path
    for path in SITE.rglob("*.md")
    if "node_modules" not in path.parts and ".vitepress" not in path.parts
)
FORBIDDEN = {
    "best-in-class",
    "cutting-edge",
    "delve",
    "effortless",
    "empower",
    "enhance",
    "enterprise-grade",
    "foster",
    "game-changing",
    "groundbreaking",
    "leverage",
    "next-generation",
    "powerful",
    "revolutionary",
    "robust",
    "seamless",
    "showcase",
    "state-of-the-art",
    "supercharge",
    "vibrant",
}


def prose_lines(path: Path) -> list[tuple[int, str]]:
    lines: list[tuple[int, str]] = []
    in_fence = False
    in_frontmatter = False

    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        stripped = raw.strip()
        if number == 1 and stripped == "---":
            in_frontmatter = True
            continue
        if in_frontmatter:
            if stripped == "---":
                in_frontmatter = False
            continue
        if stripped.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence or not stripped:
            continue
        if stripped.startswith(("#", "|", "<", ">", "-", "*", "+")):
            continue
        if re.match(r"^\d+\.", stripped):
            continue
        lines.append((number, stripped))

    return lines


errors: list[str] = []
for path in MARKDOWN_FILES:
    relative = path.relative_to(SITE)
    text = path.read_text(encoding="utf-8")

    if "—" in text:
        errors.append(f"{relative}: contains an em dash")

    lower = text.lower()
    for word in sorted(FORBIDDEN):
        if re.search(rf"\b{re.escape(word)}\b", lower):
            errors.append(f"{relative}: contains forbidden wording: {word}")

    for number, line in prose_lines(path):
        plain = re.sub(r"`[^`]*`", "value", line)
        plain = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", plain)
        for sentence in re.split(r"(?<=[.!?])\s+", plain):
            words = re.findall(r"[A-Za-z0-9][A-Za-z0-9'/-]*", sentence)
            if len(words) > 25:
                errors.append(
                    f"{relative}:{number}: sentence has {len(words)} words"
                )

if errors:
    print("\n".join(errors))
    sys.exit(1)

print(f"Checked {len(MARKDOWN_FILES)} documentation files.")
