# Lab CLI design

## Boundary

The lab CLI is a sibling of `spektrafilm` and `spektrafilm-f64`, not a mode of them. Those binaries keep a stable, user-facing surface. Lab commands may change while a method is developed. A new development command lands in the lab CLI, not in `spektrafilm`, `spektrafilm-f64`, or `spektrafilm-bench`. The extra binary keeps the user surface clean. The lab contract can move quickly without dragging the user binaries along.

Lab commands compute through `spektrafilm-core` and share the user binaries' command plumbing and parameter surface (`--preset`, `--data-dir`, `--set`). They must not duplicate spectral math, parameter resolution, or the rendering entry. A research render remains `spektrafilm-f64 process`.

## Determinism

Lab commands read declared inputs and write named outputs. They must not compare against budgets or emit pass/fail verdicts. Interpretation belongs to the caller.

An evaluation must be a pure function of its inputs. Identical inputs produce identical reports.

## Build

The lab binary is a workspace build target gated on the `precision-f64` feature. It is not part of release packages.

The command plumbing shared with the user binaries is a library, not forked code.
