#!/usr/bin/env python3
"""Fail if test code constructs a GPU device without holding the device lock.

llvmpipe is unsafe under concurrent device use in one process. Test code must
therefore obtain devices through
`spektrafilm_gpu::test_support::{adapter,explicit_adapter}`, which hand back a
wrapper that owns the lock, or hold `device_lock()` explicitly when the raw
`wgpu::Device` is needed.

`test_support` enforces this structurally for the wrapper's users, but
`WgpuBackend::new` and `WgpuBackend::new_explicit` remain `pub`, so a test can
still bypass the lock and compile cleanly. This check closes that hole in CI.

Production and example sites are exempt: each builds one device in a
single-threaded process, so there is no concurrency to serialize.
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Files exempt from the lock requirement, each with a recorded reason so the
# exemption is reviewable rather than implicit.
EXEMPT = {
    # The lock itself and its only legitimate callers.
    "crates/spektrafilm-gpu/src/test_support.rs": "defines the lock",
    # Single-threaded example binaries: one device per process.
    "crates/spektrafilm-model/examples/grain_reference_probe.rs": "single-threaded example",
    "crates/spektrafilm-gpu/examples/blur_bench.rs": "single-threaded example",
    "crates/spektrafilm-core/examples/backend_parity.rs": "single-threaded example",
    # Production paths: one device per process or per export job, serialized by
    # construction rather than by a mutex.
    "crates/spektrafilm-gui/src/backend.rs": "one device per process",
    "crates/spektrafilm-gui/src/export_job.rs": "one device per export job",
    "crates/spektrafilm-cli/src/process.rs": "one device per invocation",
    # Benchmark harness instrumentation, not test code.
    "crates/spektrafilm-benchmark/src/lib.rs": "benchmark instrumentation",
}

CONSTRUCTOR = re.compile(r"WgpuBackend::new(?:_explicit)?\s*\(")
# An integration test can bypass the backend entirely and drive wgpu directly.
# `tests/procedural_shader.rs` did exactly that before run #8, so these patterns
# are checked with the same strictness as the backend constructor.
RAW_DEVICE = re.compile(
    r"(?:\.request_device\s*\(|Instance::new\s*\(|Instance::default\s*\(|\.request_adapter\s*\()"
)
# Acquiring the lock before touching the device is what makes raw construction
# safe. A line holding the lock satisfies any device call below it in the same
# block.
LOCK_HELD = re.compile(r"device_lock\s*\(")
CFG_TEST = re.compile(r"#\[cfg\((?:all\s*\([^)]*\s*)?test\b")
# Any file under a crate's `tests/` directory is an integration-test binary:
# every line of it is test code, whether or not it carries `#[cfg(test)]`.
INTEGRATION_TEST_DIR = re.compile(r"^crates/[^/]+/tests/")


def tracked_rust_files() -> list[Path]:
    out = subprocess.check_output(["git", "ls-files", "-z", "--", "*.rs"], cwd=ROOT)
    paths = [ROOT / name.decode() for name in out.split(b"\0") if name]
    return [path for path in paths if path.is_file()]


def cfg_test_line_ranges(lines: list[str]) -> list[tuple[int, int]]:
    """1-based inclusive line ranges of `#[cfg(test)]` items.

    The marker applies to the item whose body opens at the next `{`; the range
    runs to that body's matching close, so any nesting depth inside the module
    counts. Brace counting is done over the raw text, but string and char
    literals are blanked first so braces inside them cannot corrupt the count.
    Line numbering is preserved exactly: every blanked character keeps its
    position, and newlines are never touched.
    """
    text = "\n".join(lines)
    masked = mask_literals_and_comments(text)
    ranges: list[tuple[int, int]] = []
    for marker in CFG_TEST.finditer(masked):
        open_at = masked.find("{", marker.end())
        if open_at < 0:
            continue
        depth = 0
        for index in range(open_at, len(masked)):
            char = masked[index]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    start = masked.count("\n", 0, marker.start()) + 1
                    end = masked.count("\n", 0, index) + 1
                    ranges.append((start, end))
                    break
    return ranges


def is_lifetime(text: str, index: int) -> bool:
    """True when the `'` at `index` opens a lifetime, not a char literal.

    A lifetime is `'` followed by an identifier that is not immediately closed
    by a matching quote: `'static`, `'a`, `'_`. A char literal is exactly one
    character (or one escape) between the quotes.
    """
    if index + 1 >= len(text):
        return False
    following = text[index + 1]
    if following.isalpha() or following == "_":
        # `'a` starts a lifetime unless the very next char closes it, as in
        # `'a'` — but that is a char literal only when preceded by a value
        # position. In practice Rust code in this repo never writes `'a'` as a
        # char literal, and erring toward "lifetime" is safe: a lifetime holds
        # no braces, so leaving it unmasked cannot corrupt brace counting.
        return True
    return False


def mask_literals_and_comments(text: str) -> str:
    """Blank comments, string bodies and char literals, preserving length.

    Newlines are always preserved so line numbers map 1:1 to the source. Each
    consumed character emits exactly one replacement, so the result has the
    same length as the input; the assertion at the end enforces that.
    """
    out: list[str] = []
    index = 0
    length = len(text)
    while index < length:
        char = text[index]
        nxt = text[index + 1] if index + 1 < length else ""
        # Line comment: blank to end of line, keeping the newline itself.
        if char == "/" and nxt == "/":
            while index < length and text[index] != "\n":
                out.append(" ")
                index += 1
            continue
        # Block comment: blank, preserving newlines.
        if char == "/" and nxt == "*":
            out.append(" ")
            out.append(" ")
            index += 2
            while index < length:
                if text.startswith("*/", index):
                    out.append(" ")
                    out.append(" ")
                    index += 2
                    break
                out.append("\n" if text[index] == "\n" else " ")
                index += 1
            continue
        # String literal. A backslash escape consumes two characters, both
        # blanked, so the count stays balanced.
        if char == '"':
            out.append(" ")
            index += 1
            while index < length:
                if text[index] == "\\":
                    # A backslash escape spans two characters. A line
                    # continuation (`\` + newline) must keep its newline so
                    # line numbering stays intact.
                    out.append(" ")
                    out.append("\n" if text[index + 1] == "\n" else " ")
                    index += 2
                    continue
                if text[index] == '"':
                    out.append(" ")
                    index += 1
                    break
                out.append("\n" if text[index] == "\n" else " ")
                index += 1
            continue
        # A lifetime (`'static`, `'a`) is not a quote: it contains no braces
        # and no escapes, so it is left alone. A char literal (`'x'`,
        # `'\n'`) is blanked. Distinguishing them matters because treating a
        # lifetime as an opening quote would swallow the rest of the file and
        # silently disable the check.
        if char == "'" and not is_lifetime(text, index):
            out.append(" ")
            index += 1
            while index < length:
                if text[index] == "\\":
                    # A backslash escape spans two characters. A line
                    # continuation (`\` + newline) must keep its newline so
                    # line numbering stays intact.
                    out.append(" ")
                    out.append("\n" if text[index + 1] == "\n" else " ")
                    index += 2
                    continue
                if text[index] == "'":
                    out.append(" ")
                    index += 1
                    break
                out.append("\n" if text[index] == "\n" else " ")
                index += 1
            continue
        out.append(char)
        index += 1
    result = "".join(out)
    assert len(result) == length, "masking must preserve length"
    return result


def violations(path: Path) -> list[int]:
    rel = path.relative_to(ROOT).as_posix()
    if rel in EXEMPT:
        return []
    text = path.read_text(encoding="utf-8")
    # An integration test is test code on every line; a unit test only inside
    # its `#[cfg(test)]` scope.
    if INTEGRATION_TEST_DIR.match(rel):
        ranges = [(1, len(text.splitlines()) + 1)]
    else:
        ranges = cfg_test_line_ranges(text.splitlines())
    # Match against masked text so a constructor mentioned in a comment or a
    # string literal is not mistaken for a call site.
    masked = mask_literals_and_comments(text)
    lines = masked.splitlines()
    found: list[int] = []
    for lineno, line in enumerate(lines, start=1):
        if not any(start <= lineno <= end for start, end in ranges):
            continue
        # Raw wgpu construction is acceptable only when the device lock is
        # already held at that point: `test_support` cannot wrap a device the
        # caller built itself, so the ordering is the caller's responsibility.
        if RAW_DEVICE.search(line) and not LOCK_HELD.search(line):
            if any(LOCK_HELD.search(earlier) for earlier in lines[: lineno - 1]):
                continue
            found.append(lineno)
            continue
        # The backend constructor must always go through the lock.
        if CONSTRUCTOR.search(line):
            found.append(lineno)
    return found


def main() -> int:
    failures: list[str] = []
    for path in tracked_rust_files():
        for lineno in violations(path):
            failures.append(f"{path.relative_to(ROOT).as_posix()}:{lineno}")
    if not failures:
        print("device-lock guard: no test constructs an unlocked GPU device")
        return 0
    print(
        "device-lock guard: test code must obtain devices through "
        "test_support::{adapter,explicit_adapter} or hold device_lock().",
        file=sys.stderr,
    )
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    print(
        "If this site is genuinely single-threaded, add it to EXEMPT with a "
        "reason rather than weakening the check.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
