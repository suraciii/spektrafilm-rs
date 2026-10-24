#!/usr/bin/env python3
"""Self-check for scripts/check_specs.py.

The checker is a gate: a parsing regression must fail loudly, not silently pass
a dirty corpus. Each case below is a synthetic fixture with one known defect.

Run with: python3 scripts/test_check_specs.py
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECKER = Path(__file__).resolve().with_name("check_specs.py")


def write(root: Path, relative: str, content: str) -> None:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def run(root: Path, *arguments: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(CHECKER), "--root", str(root), *arguments],
        capture_output=True,
        text=True,
        check=False,
    )


def clean_fixture(root: Path) -> None:
    write(root, "CONTEXT.md", "# Context\n\n## Language\n\n**Term**: definition.\n")
    write(root, "specs/topic/spec.md", "# Topic\n\n## Contract\n\n- One rule.\n")


def test_clean() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        result = run(root)
        assert result.returncode == 0, result.stdout + result.stderr
        assert "checked" in result.stdout


def test_broken_link() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        write(root, "docs/page.md", "# Page\n\n[Missing](../specs/topic/missing.md)\n")
        result = run(root)
        assert result.returncode == 1, result.stdout
        assert "missing link target" in result.stdout


def test_missing_anchor() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        write(root, "docs/page.md", "# Page\n\n[Contract](../specs/topic/spec.md#nope)\n")
        result = run(root)
        assert result.returncode == 1, result.stdout
        assert "missing anchor" in result.stdout


def test_lowercase_modal_in_specs() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        write(root, "specs/topic/spec.md", "# Topic\n\n## Contract\n\nThe command must fail.\n")
        result = run(root)
        assert result.returncode == 1, result.stdout
        assert "use uppercase MUST/MAY/SHOULD in specs" in result.stdout


def test_uppercase_modal_in_specs() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        write(root, "specs/topic/spec.md", "# Topic\n\n## Contract\n\nThe command MUST fail.\n")
        result = run(root)
        assert result.returncode == 0, result.stdout + result.stderr


def test_uppercase_modal_in_docs() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        write(root, "docs/page.md", "# Page\n\nThe command MUST fail.\n")
        result = run(root)
        assert result.returncode == 1, result.stdout
        assert "use lowercase must/may/should outside specs" in result.stdout


def test_heading_structure() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        write(root, "specs/topic/spec.md", "# One\n\n# Two\n")
        result = run(root)
        assert result.returncode == 1, result.stdout
        assert "expected one level-1 heading" in result.stdout


def test_presentation_warning() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        write(
            root,
            "specs/topic/spec.md",
            "# Topic\n\n## Contract\n\nOne rule. Two rule. Three rule. Four rule.\n",
        )
        result = run(root)
        assert result.returncode == 0, result.stdout
        assert "presentation warning" in result.stdout
        strict = run(root, "--strict")
        assert strict.returncode == 1, strict.stdout


def test_only_filter() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        write(
            root,
            "specs/long/spec.md",
            "# Long\n\n## Contract\n\nOne rule. Two rule. Three rule. Four rule.\n",
        )
        selected = run(root, "--only", "specs/topic/spec.md")
        assert selected.returncode == 0, selected.stdout
        assert "presentation warning" not in selected.stdout
        filtered = run(root, "--only", "specs/long/spec.md")
        assert filtered.returncode == 0, filtered.stdout
        assert "presentation warning" in filtered.stdout


def test_only_normalizes_paths_and_rejects_empty_selection() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        write(root, "specs/topic/spec.md", "# Topic\n\nThe command must fail.\n")
        for selection in ("./specs/topic/spec.md", "specs/../specs/topic/spec.md"):
            result = run(root, "--strict", "--only", selection)
            assert result.returncode == 1, (selection, result.stdout, result.stderr)
            assert "uppercase" in result.stdout
        (root / "empty").mkdir()
        for selection in ("missing.md", "empty"):
            result = run(root, "--only", selection)
            assert result.returncode == 1, (selection, result.stdout, result.stderr)
            assert "matches --only" in result.stderr
        result = run(root, "--only", "specs")
        assert result.returncode == 1, (result.stdout, result.stderr)
        assert "uppercase" in result.stdout
        assert "checked" not in result.stdout


def test_fenced_and_inline_code_is_not_prose() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        # A plain corpus: `tracked_markdown` falls back to the directory walk
        # because the fixture is not a Git repository.
        write(
            root,
            "specs/topic/spec.md",
            "# Topic\n\n  ```text\n# Phantom\nThe code must stay literal.\n  ```\n\n"
            "## Contract\n\nThe command MUST fail. Use ``the `must` token``.\n",
        )
        result = run(root, "--strict")
        assert result.returncode == 0, result.stdout + result.stderr
        write(root, "docs/page.md", "# Page\n\n[Phantom](../specs/topic/spec.md#phantom)\n")
        result = run(root)
        assert result.returncode == 1, result.stdout
        assert "missing anchor" in result.stdout
        write(root, "docs/page.md", "# Page\n\n[Contract](../specs/topic/spec.md#contract)\n")
        result = run(root)
        assert result.returncode == 0, result.stdout + result.stderr
        (root / "docs/page.md").unlink()
        write(
            root,
            "specs/topic/spec.md",
            "# Topic\n\nUse ``the `must` token``. The command must fail.\n",
        )
        result = run(root)
        assert result.returncode == 1, result.stdout
        assert "uppercase" in result.stdout
        write(
            root,
            "docs/page.md",
            "# Page\n\nUse ``MUST``. The command must fail.\n",
        )
        result = run(root)
        assert result.returncode == 1, result.stdout
        assert "lowercase" in result.stdout


def test_tracked_corpus_excludes_untracked_files() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        clean_fixture(root)
        subprocess.run(["git", "init", "--quiet", str(root)], check=True)
        subprocess.run(["git", "add", "."], cwd=root, check=True)
        write(root, "specs/scratch.md", "# Scratch\n\nOne rule. Two rule. Three rule. Four rule.\n")
        result = run(root, "--strict")
        assert result.returncode == 0, result.stdout + result.stderr
        assert "scratch.md" not in result.stdout
        subprocess.run(["git", "add", "specs/scratch.md"], cwd=root, check=True)
        result = run(root, "--strict")
        assert result.returncode == 1, result.stdout
        assert "presentation warning" in result.stdout


def main() -> int:
    tests = (
        test_clean,
        test_broken_link,
        test_missing_anchor,
        test_lowercase_modal_in_specs,
        test_uppercase_modal_in_specs,
        test_uppercase_modal_in_docs,
        test_heading_structure,
        test_presentation_warning,
        test_only_filter,
        test_only_normalizes_paths_and_rejects_empty_selection,
        test_fenced_and_inline_code_is_not_prose,
        test_tracked_corpus_excludes_untracked_files,
    )
    for test in tests:
        test()
    print(f"ok: {len(tests)} cases")
    return 0


if __name__ == "__main__":
    sys.exit(main())
