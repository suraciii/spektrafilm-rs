#!/usr/bin/env python3
"""Check durable documentation links, structure, and spec presentation.

Errors reject broken links or anchors, malformed headings, and modal case
(specs uppercase, other durable docs lowercase). Presentation warnings report violations of the limits in
`specs/AGENTS.md`; `--strict` turns those warnings into errors.
"""

from __future__ import annotations

import argparse
import collections
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

UPPER_MODAL = re.compile(r"\b(?:MUST NOT|MUST|MAY|SHOULD NOT|SHOULD|REQUIRED)\b")
LOWER_MODAL = re.compile(r"\b(?:must not|must|may|should not|should)\b")
LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*$")
FENCE = "```"
PARAGRAPH_SENTENCES = 3
PARAGRAPH_WORDS = 60
BULLET_SENTENCES = 2
BULLET_WORDS = 60
SENTENCE_WORDS = 40

GOVERNED_ROOTS = {"specs", "docs", "eng"}
GOVERNED_FILES = {"CONTEXT.md", "CONTEXT-MAP.md", "CODING_STANDARDS.md"}

TRANSIENT = (
    ("date", re.compile(r"\b20\d{2}-\d{2}-\d{2}\b")),
    ("count", re.compile(r"\b\d{2,4}\s+(?:sites|call sites|lines|occurrences)\b")),
    ("state", re.compile(
        r"\b(?:currently|for now|as of|right now|temporarily|"
        r"not yet implemented|to be implemented|future work|WIP|TBD|TODO)\b",
        re.IGNORECASE,
    )),
)


def tracked_markdown(root: Path) -> list[Path]:
    try:
        output = subprocess.check_output(
            ["git", "ls-files", "-z", "--", "*.md"], cwd=root, stderr=subprocess.DEVNULL
        )
        return [root / name for name in output.decode().split("\0") if name]
    except (OSError, subprocess.CalledProcessError):
        return sorted(root.rglob("*.md"))


def governed(path: Path, root: Path) -> bool:
    relative = path.relative_to(root)
    return relative.parts[0] in GOVERNED_ROOTS or relative.name in GOVERNED_FILES


def heading_slugs(path: Path) -> set[str]:
    seen: collections.Counter[str] = collections.Counter()
    slugs: set[str] = set()
    in_fence = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.lstrip().startswith(FENCE):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        match = HEADING.match(line)
        if not match:
            continue
        text = re.sub(r"<[^>]+>", "", match.group(2))
        text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", text)
        text = re.sub(r"[`*_]", "", text).strip().lower()
        text = re.sub(r"[^\w\u4e00-\u9fff\- ]", "", text)
        slug = text.replace(" ", "-")
        index = seen[slug]
        seen[slug] += 1
        slugs.add(slug if index == 0 else f"{slug}-{index}")
    return slugs


def check_links(root: Path, files: list[Path]) -> list[str]:
    cache: dict[Path, set[str]] = {}
    problems = []
    for path in files:
        if path.suffix != ".md":
            continue
        in_fence = False
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if line.lstrip().startswith(FENCE):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            for target in LINK.findall(line):
                if target.startswith(("http://", "https://", "mailto:")):
                    continue
                path_part, _, anchor = target.partition("#")
                destination = path if not path_part else (path.parent / path_part).resolve()
                if not destination.exists():
                    problems.append(
                        f"{path.relative_to(root)}:{number}: missing link target {target}"
                    )
                    continue
                if anchor and destination.suffix == ".md":
                    if destination not in cache:
                        cache[destination] = heading_slugs(destination)
                    if anchor not in cache[destination]:
                        problems.append(
                            f"{path.relative_to(root)}:{number}: missing anchor {target}"
                        )
    return problems


def check_headings(root: Path, files: list[Path]) -> list[str]:
    problems = []
    for path in files:
        if not governed(path, root) or path.suffix != ".md":
            continue
        headings = []
        in_fence = False
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if line.lstrip().startswith(FENCE):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            match = HEADING.match(line)
            if match:
                headings.append((number, len(match.group(1)), match.group(2)))
        h1 = [heading for heading in headings if heading[1] == 1]
        if len(h1) != 1:
            problems.append(
                f"{path.relative_to(root)}: expected one level-1 heading, found {len(h1)}"
            )
        previous = 0
        for number, level, _ in headings:
            if previous and level > previous + 1:
                problems.append(
                    f"{path.relative_to(root)}:{number}: heading jumps from "
                    f"level {previous} to {level}"
                )
            previous = level
    return problems


def check_modals(root: Path, files: list[Path]) -> list[str]:
    problems = []
    for path in files:
        if not governed(path, root) or path.suffix != ".md":
            continue
        relative = path.relative_to(root)
        spec = relative.parts[0] == "specs"
        pattern = LOWER_MODAL if spec else UPPER_MODAL
        message = (
            "use uppercase MUST/MAY/SHOULD in specs"
            if spec
            else "use lowercase must/may/should outside specs"
        )
        in_fence = False
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if line.lstrip().startswith(FENCE):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            prose = []
            spans = line.split("`")
            for position, segment in enumerate(spans):
                if position % 2 == 0:
                    prose.append(segment)
            if pattern.search("".join(prose)):
                problems.append(f"{relative}:{number}: {message}")
    return problems


def blocks(path: Path):
    """Yield (kind, start_line, text) for prose paragraphs and bullet items."""
    current: list[str] = []
    kind = None
    start = 0
    in_fence = False

    def flush():
        nonlocal current, kind, start
        if current:
            yield kind, start, " ".join(current)
        current = []
        kind = None
        start = 0

    lines = path.read_text(encoding="utf-8").splitlines()
    for number, line in enumerate(lines, 1):
        stripped = line.strip()
        if stripped.startswith(FENCE):
            yield from flush()
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        if (not stripped
                or stripped.startswith(("#", "|", ">", "!["))
                or re.match(r"^(?:\*{3,}|-{3,}|_{3,})$", stripped)):
            yield from flush()
            continue
        item = re.match(r"^(?:[-*]|\d+\.)\s+(.*)$", stripped)
        if item:
            yield from flush()
            kind = "bullet"
            start = number
            current = [item.group(1)]
            continue
        if stripped.startswith(("    ", "\t")):
            if current:
                current.append(stripped)
            else:
                yield from flush()
            continue
        if current and kind == "bullet":
            yield from flush()
        if not current:
            kind = "paragraph"
            start = number
        current.append(stripped)
    yield from flush()


def words(text: str) -> int:
    return len(re.findall(r"[A-Za-z0-9_`'’./-]+", text)) + len(
        re.findall(r"[\u4e00-\u9fff]", text)
    )


def sentences(text: str) -> list[str]:
    for abbreviation in ("e.g.", "i.e.", "vs.", "etc."):
        text = text.replace(abbreviation, abbreviation.replace(".", ""))
    return [part.strip() for part in re.split(r"(?<=[.!?])\s+", text) if part.strip()]


def check_presentation(root: Path, files: list[Path]) -> list[tuple[str, int, str]]:
    problems = []
    for path in files:
        if not governed(path, root) or path.relative_to(root).parts[0] != "specs":
            continue
        for kind, line, text in blocks(path):
            count = words(text)
            parts = sentences(text)
            sentence_count = len(parts)
            if kind == "paragraph":
                if sentence_count > PARAGRAPH_SENTENCES or count > PARAGRAPH_WORDS:
                    problems.append((
                        str(path.relative_to(root)), line,
                        f"paragraph has {sentence_count} sentences / {count} words",
                    ))
            elif sentence_count > BULLET_SENTENCES or count > BULLET_WORDS:
                problems.append((
                    str(path.relative_to(root)), line,
                    f"bullet has {sentence_count} sentences / {count} words",
                ))
            for sentence in parts:
                if words(sentence) > SENTENCE_WORDS:
                    problems.append((
                        str(path.relative_to(root)), line,
                        f"sentence has {words(sentence)} words",
                    ))
    return problems


def check_transient(root: Path, files: list[Path]) -> list[tuple[str, int, str]]:
    problems = []
    for path in files:
        if path.relative_to(root).parts[0] != "specs":
            continue
        text = path.read_text(encoding="utf-8")
        for name, pattern in TRANSIENT:
            for match in pattern.finditer(text):
                line = text.count("\n", 0, match.start()) + 1
                problems.append((
                    str(path.relative_to(root)), line,
                    f"transient {name} marker: {match.group(0)}",
                ))
    return problems


def check_context_terms(root: Path, files: list[Path]) -> list[tuple[str, int, str]]:
    context = root / "CONTEXT.md"
    if not context.exists():
        return []
    terms = re.findall(r"^\*\*(.+?)\*\*:", context.read_text(encoding="utf-8"), re.M)
    problems = []
    for path in files:
        if path.relative_to(root).parts[0] != "specs":
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for term in terms:
                pattern = r"\*\*" + re.escape(term) + r"\*\*\s*(?::|\bis\b|\bare\b|\bmeans\b)"
                if re.search(pattern, line):
                    problems.append((
                        str(path.relative_to(root)), number,
                        f"term also defined in CONTEXT.md: {term}",
                    ))
    return problems


def print_findings(label: str, findings, all_findings: bool, limit: int = 15) -> None:
    if not findings:
        return
    print(f"{len(findings)} {label}:")
    shown = findings if all_findings else findings[:limit]
    for path, line, message in shown:
        print(f"  {path}:{line}: {message}")
    if len(findings) > len(shown):
        print(f"  ... {len(findings) - len(shown)} more; use --all")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(ROOT))
    parser.add_argument("--strict", action="store_true",
                        help="fail on presentation warnings")
    parser.add_argument("--all", action="store_true", help="print every warning")
    parser.add_argument("--only", action="append", default=[], metavar="PATH",
                        help="report only this file or directory; repeatable")
    args = parser.parse_args()
    root = Path(args.root).resolve()
    files = tracked_markdown(root)

    def selected(relative: str) -> bool:
        if not args.only:
            return True
        wanted = []
        for item in args.only:
            candidate = root / item if not Path(item).is_absolute() else Path(item)
            resolved = candidate.resolve()
            owned = [
                file.relative_to(root).as_posix()
                for file in files
                if file == resolved or resolved in file.parents
            ]
            if not owned:
                raise SystemExit(f"no Markdown file or directory matches --only: {item}")
            wanted.extend(owned)
        return relative in wanted or any(
            relative.startswith(item.rstrip("/") + "/") for item in wanted
        )

    errors = []
    errors.extend(check_links(root, files))
    errors.extend(check_headings(root, files))
    errors.extend(check_modals(root, files))
    errors = [problem for problem in errors if selected(problem.split(":", 1)[0])]

    presentation = [item for item in check_presentation(root, files) if selected(item[0])]
    transient = [item for item in check_transient(root, files) if selected(item[0])]
    terms = [item for item in check_context_terms(root, files) if selected(item[0])]

    if errors:
        print(f"{len(errors)} error(s):")
        for problem in errors:
            print(f"  {problem}")

    if presentation:
        print_findings("presentation warning(s)", presentation, args.all)
    if transient:
        print_findings("transient warning(s)", transient, args.all)
    if terms:
        print_findings("term warning(s)", terms, args.all)

    if errors or (args.strict and presentation):
        return 1
    print(f"checked {len(files)} Markdown files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
