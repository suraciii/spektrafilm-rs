# Contributing to SpektraFilm

Thank you for contributing. This guide covers the contribution process. Build
requirements, packaging, and the full command list live in
[Building, packaging, and verification](docs/building.md).

## Before you open a pull request

- Search [open issues](https://github.com/suraciii/spektrafilm-rs/issues) and
  [open pull requests](https://github.com/suraciii/spektrafilm-rs/pulls) for
  duplicates. Link the closest one, or write "none found".

## Commits

- Follow [Conventional Commits](https://www.conventionalcommits.org/) with a
  scope, for example `fix(cli): reject unknown film profiles before decode`.
- Sign every commit with `git commit -s`. The repository commit-msg hook adds
  the `Signed-off-by` trailer automatically.
- Pull requests are squash-merged into `main-0.3.4`; the pull request title
  becomes the commit subject.

## Gates and evidence

- `just ci-pr` is the required pull request gate: formatting, Python cache
  checks, and the fast precision test profile.
- `just ci` runs the full local gate: `fmt`, `check`, `check-f64`, `clippy`,
  `test`, `test-f64`, `benchmark-test`, and `parity-backend`.
- `just verify-pr <dir>` and `just verify <dir>` retain gate output with source
  provenance.

Automated checks are necessary but not sufficient. Exercise the changed
behavior through the real entry point and report the exact scenario and result
in the pull request. The required evidence depends on the change:

| Change | Required evidence |
| --- | --- |
| Numerical or pipeline behavior | Measured before/after results; `just parity-backend` or `just benchmark-test` where applicable |
| GUI | Screenshots, or `just gui-headless`, `just gui-layout`, and `just gui-raw` output |
| LUT export | The exported LUT and a rendered comparison |
| CLI, image I/O, or RAW decoding | A run through the real binary with the affected input |

Numeric agreement is judged against per-scenario budgets, not universal bit
identity. See [parity and backends](docs/parity.md) and
[test architecture](eng/testing.md).

## Standards

- [Coding standards](CODING_STANDARDS.md) — reviewer judgment rules.
- [Test architecture](eng/testing.md) — test requirements and organization.
- [Crate architecture](eng/architecture.md) — workspace layout.
- [specs/](specs/) — product contracts; [docs/](docs/README.md) — user guides.

## Language

Issues, pull requests, commits, and documentation are written in English. See
[documentation language](eng/documentation-language.md).

## License

The Rust program is [GPL-3.0](LICENSE). Bundled spectral profiles and their LUT
derivatives keep their upstream CC BY-SA 4.0 terms;
[attribution](docs/attribution.md) records both. No contributor license
agreement is required.
