#!/usr/bin/env python3
"""Self-check for scripts/check_device_lock.py.

The guard is the only thing standing between this repository and a silent
reintroduction of the llvmpipe concurrency bug. Its masker hand-rolls a Rust
lexer, and a bug in that lexer fails *open*: a swallowed `#[cfg(test)]` span
makes the guard report "no violations" on a tree that is in fact dirty. That
failure mode produced three real bugs during development (a short lookback
window, line-continuation backslashes, and lifetimes parsed as char literals),
each found only because a manual negative control happened to run.

Those controls were throwaway. This file makes them permanent: every case below
is a synthetic fixture the guard must classify correctly, so a future edit to
the masker cannot silently disable the check.

Run with: python3 scripts/test_check_device_lock.py
"""
from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GUARD = ROOT / "scripts" / "check_device_lock.py"

_spec = importlib.util.spec_from_file_location("check_device_lock", GUARD)
assert _spec is not None and _spec.loader is not None
guard = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(guard)

SRC = "crates/x/src/t.rs"
TESTS = "crates/x/tests/t.rs"


def violations_for(source: str, rel: str = SRC) -> list[int]:
    """Classify `source` as if it were a tracked Rust file at `rel`.

    The guard resolves paths relative to the repository root and decides scope
    partly from the path, so fixtures must live inside the tree rather than in
    a system temp directory.
    """
    path = ROOT / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source, encoding="utf-8")
    try:
        return guard.violations(path)
    finally:
        path.unlink()


def line_of(source: str, needle: str) -> int:
    """1-based line number of the single line containing `needle`."""
    hits = [i for i, line in enumerate(source.splitlines(), start=1) if needle in line]
    assert len(hits) == 1, f"expected exactly one line with {needle!r}, got {hits}"
    return hits[0]


def expect(source: str, needle: str, rel: str = SRC) -> None:
    """The line containing `needle` must be the only reported violation."""
    assert violations_for(source, rel) == [line_of(source, needle)]


def expect_clean(source: str, rel: str = SRC) -> None:
    assert violations_for(source, rel) == []


class UnitTestScopeDetection(unittest.TestCase):
    def test_plain_cfg_test(self) -> None:
        expect(
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    #[test]\n"
            "    fn a() {\n"
            "        let b = spektrafilm_gpu::wgpu_backend::WgpuBackend::new();\n"
            "    }\n"
            "}\n",
            "WgpuBackend::new()",
        )

    def test_deeply_nested_inside_cfg_test(self) -> None:
        # The marker sits ~120 lines above the call site. A fixed lookback
        # window missed this, which is what defeated the first implementation.
        filler = "\n".join(f"    fn filler_{i}() {{ let _ = {i}; }}" for i in range(120))
        source = (
            "#[cfg(test)]\n"
            "mod tests {\n"
            f"{filler}\n"
            "    #[test]\n"
            "    fn a() {\n"
            "        let b = WgpuBackend::new_explicit();\n"
            "    }\n"
            "}\n"
        )
        expect(source, "WgpuBackend::new_explicit()")
        self.assertGreater(line_of(source, "WgpuBackend::new_explicit()"), 120)

    def test_production_code_is_not_a_violation(self) -> None:
        expect_clean(
            "pub fn make() -> WgpuBackend {\n"
            "    WgpuBackend::new()\n"
            "}\n"
        )

    def test_cfg_all_test_feature(self) -> None:
        expect(
            '#[cfg(all(test, feature = "wgpu-backend"))]\n'
            "mod tests {\n"
            "    fn a() { let _ = WgpuBackend::new(); }\n"
            "}\n",
            "WgpuBackend::new()",
        )


class IntegrationTestScope(unittest.TestCase):
    def test_tests_directory_is_checked_without_a_marker(self) -> None:
        # `tests/grain_v2_shader.rs` carries no `#[cfg(test)]` at all, which is
        # why the guard was blind to integration tests before run #15.
        expect(
            "#![cfg(feature = \"wgpu-backend\")]\n"
            "#[test]\n"
            "fn grain_v2_shader_compiles() {\n"
            "    let instance = wgpu::Instance::default();\n"
            "}\n",
            "Instance::default",
            TESTS,
        )


class MaskingCorrectness(unittest.TestCase):
    def test_constructor_in_a_comment_is_ignored(self) -> None:
        expect_clean(
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    // WgpuBackend::new() is forbidden here.\n"
            "    fn a() {}\n"
            "}\n"
        )

    def test_constructor_in_a_string_is_ignored(self) -> None:
        expect_clean(
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    fn a() {\n"
            "        let doc = \"WgpuBackend::new() is forbidden\";\n"
            "    }\n"
            "}\n"
        )

    def test_braces_inside_strings_do_not_break_scoping(self) -> None:
        expect(
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    fn a() {\n"
            "        let s = \"} } } {\";\n"
            "        let b = WgpuBackend::new();\n"
            "    }\n"
            "}\n",
            "WgpuBackend::new()",
        )

    def test_lifetime_is_not_a_char_literal(self) -> None:
        # `&'static str` was parsed as an opening quote by an earlier masker,
        # which swallowed the rest of the file and disabled the check silently.
        expect(
            "pub fn note() -> &'static str {\n"
            "    \"x\"\n"
            "}\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    fn a() { let _ = WgpuBackend::new(); }\n"
            "}\n",
            "WgpuBackend::new()",
        )

    def test_line_continuation_preserves_line_numbers(self) -> None:
        # A `\` + newline inside a string literal used to become a space,
        # shifting every later line number and emptying all spans.
        expect(
            "const S: &str = \"a \\\n"
            "    b\";\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    fn a() { let _ = WgpuBackend::new(); }\n"
            "}\n",
            "WgpuBackend::new()",
        )

    def test_masking_preserves_length_and_newlines(self) -> None:
        source = (
            "// comment with } braces\n"
            "const S: &str = \"line \\\n"
            "    continuation { }\";\n"
            "pub fn f() -> &'static str { S }\n"
        )
        masked = guard.mask_literals_and_comments(source)
        self.assertEqual(len(masked), len(source))
        self.assertEqual(masked.count("\n"), source.count("\n"))


class RawDeviceConstruction(unittest.TestCase):
    def test_instance_without_lock_is_a_violation(self) -> None:
        expect(
            "#[test]\n"
            "fn a() {\n"
            "    let instance = wgpu::Instance::default();\n"
            "}\n",
            "Instance::default",
            TESTS,
        )

    def test_instance_after_lock_is_allowed(self) -> None:
        # The shape of the correctly-fixed `tests/grain_v2_shader.rs`.
        expect_clean(
            "#[test]\n"
            "fn a() {\n"
            "    let _guard = spektrafilm_gpu::test_support::device_lock();\n"
            "    let instance = wgpu::Instance::default();\n"
            "}\n",
            TESTS,
        )

    def test_request_device_without_lock_is_a_violation(self) -> None:
        expect(
            "#[test]\n"
            "fn a() {\n"
            "    pollster::block_on(adapter.request_device(&desc, None));\n"
            "}\n",
            "request_device",
            TESTS,
        )


class Exemptions(unittest.TestCase):
    def test_lock_definer_is_exempt(self) -> None:
        self.assertIn("crates/spektrafilm-gpu/src/test_support.rs", guard.EXEMPT)


if __name__ == "__main__":
    unittest.main(verbosity=2)
