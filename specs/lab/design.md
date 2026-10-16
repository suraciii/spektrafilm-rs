# Lab CLI design

## Boundary

The lab CLI is a sibling of `spektrafilm` and `spektrafilm-f64`, not a mode of them. Those binaries keep a stable, user-facing surface. Lab commands may change while a method is developed. A new development command lands in the lab CLI, not in `spektrafilm`, `spektrafilm-f64`, or `spektrafilm-bench`. The extra binary keeps the user surface clean. The lab contract can move quickly without dragging the user binaries along.

Lab commands compute through `spektrafilm-core` and share the user binaries' command plumbing and parameter surface (`--preset`, `--data-dir`, `--set`). They must not duplicate spectral math, parameter resolution, or the rendering entry. A research render remains `spektrafilm-f64 process`, and `experiment run` reaches it through the shared `process` entry rather than a second render path.

A lab document that names a chain carries parameter *values*, not a parameter language. It uses the process surface's names and precedence so that resolving it is the same resolution the user binaries perform, and it must not accept a parameter file path that would resolve outside the document's own directory tree.

A lab document that names a case consumes the region vocabulary the [benchmark](../benchmark/spec.md) owns. It references one case and declares the grid that reproduces its regions; it does not restate the region list, the roles, or the measurement families. A geometry that does not match the declared grid is a failure, not a crop.

`experiment` converts a bundle's buffers byte-for-byte. It does not measure regions, compute color differences, or resample: those belong to the benchmark, and a converter that "helpfully" averages or reorders channels would hide a misread.

## Determinism

Lab commands read declared inputs and write named outputs. They must not compare against budgets or emit pass/fail verdicts. Interpretation belongs to the caller.

An evaluation must be a pure function of its inputs. Identical inputs produce identical reports.

## Build

The lab binary is a workspace build target gated on the `precision-f64` feature. It is not part of release packages.

The command plumbing shared with the user binaries is a library, not forked code.

## Measurement boundary

A measurement command shares the parameter surface and the execution path of the user binaries. It must not resolve parameters on its own, reconstruct pipeline mathematics, or walk the stages beside an observed execution. A stage walk beside the executed chain would report a different result under a stochastic stage, so the observed path is the only permitted source of readings.

The measured range is the same fact for both binaries: `--enter` becomes the resolved `taps.inject` and the collection boundary is the chain's own `taps.collect` or the last requested reading. Nothing else decides what executes, so the retained effective document describes the execution and a re-verification reproduces it.

The executed chain is resolved per invocation. A resolved parameter document is evidence, never a declaration. A command must not accept a stored resolution as its chain input, because nothing relates that document to the profiles, the data inventory, and the executable present at run time. This applies the resolution stance of the [CLI design](../cli/design.md) to measurement: resolve before executing, and keep the resolution beside the result instead of passing it between commands. A measurement records its resolved chain, and `measure verify` re-resolves the recorded declaration and compares the two identities without judging them.

The lab must not own a second implementation of a metric. A color-difference, equal-lightness, or region-aggregation algorithm keeps one owner, and the measurement commands consume it.

## Reading boundary

Readings are defined only for a point-wise chain. An injected numeric sample has no neighborhood and no stochastic realization, so an operation that mixes samples, measures the whole frame, resizes it, or draws stochastic values would describe a different signal than the measured one. The command fails before executing instead of reporting those numbers.

Eligibility is a measured property, not a reading of a flag name: each operation carries the first stage whose values it changes, and it is listed only when it executes inside the measured range. The table and its measurements live in the [specification](spec.md#point-wise-requirement); the lab must not widen it by guessing that a control is spatial because its name suggests so.

A sample plan is the only authority for a reading position. A command must not infer geometry from image dimensions, labels, or color similarity. A sample outside the declared source remains an error rather than a clamped value.

A guard run is a frozen measurement artifact together with the stimuli that produced it. It carries no tolerance and no verdict; a tolerance belongs to the protocol document that declares it. Frozen readings make chain drift visible, so a comparison must report an identity difference beside the value difference instead of refusing the pair.

## Artifact boundary

A reading record is a directory: `measure.json` carries the declarations and the identity, `points.csv` carries the numbers with their sample identities, and `evidence/` carries the stage buffers of an executed run in the form the user binaries already write. A comparison, a verification, and a later reproduction all read that same directory, so the lab must not invent a second record format for a retained reading.

`measure.json` carries:

- `schema_version`, `kind`, and `executed`.
- `declaration`: the chain options, the entry, the read stages, the condition, and the stimulus document — everything needed to resolve the same chain again.
- `stimulus`: the manifest path and the array hashes of the stimulus document.
- `identity` for an executed run: the renderer's path, hash, and version, the data inventory digest, the profile identities, the declared parameter sources and the content hash of every parameter file they name, the stock-specific application, the effective parameter document, the precision, and the seed. The effective document is what `measure verify` diffs leaf by leaf, so a re-verification names the parameter that moved instead of only its digest. A sampled source records its own `source` block instead, because no chain was resolved to read it.
- `stages`: one record per read stage with `id`, `domain`, `units`, `channel_order`, the declared color space and transfer state, the buffer hash, and the availability together with a stable reason when the stage did not run.
- `groups` and `summaries`: the group identities and their per-channel statistics.
- `executor`: the backend and the precision of an executed run, so a reading names the route that produced it.

`points.csv` carries one row per sample and stage:

```text
sample_id,source,partition,stage,domain,units,channel_order,c0,c1,c2,valid
```

A value that is not finite is an empty field and `valid` is `false`. The lab must not write a placeholder number in its place.

`measure compare` publishes `compare.json` and `deltas.csv` with `pair,sample_id,source,partition,stage,domain,units,b0,b1,b2,c0,c1,c2,d0,d1,d2,d_e00,valid,clipped`. One row per joined sample and stage per comparison, so the reference comparisons keep their per-sample rows; `pair` names the comparison (`candidate_minus_baseline`, `candidate_minus_reference`, `baseline_minus_reference`), and `d` is the minuend side minus the subtrahend side of that comparison. `b0..b2` are the comparison's subtrahend side and `c0..c2` its minuend side, so for the primary comparison they are the baseline and the candidate; the `pair` label names which comparison the row belongs to. `d_e00` stays empty when the stage declares no color space, `clipped` counts the channels the distance had to bound, and a distance that cannot be computed is reported with its reason in the record rather than as a zero. A pair whose two sides declare different color interpretations writes no channel deltas either: its delta fields are empty and `valid` is `false`, so a file-level aggregation cannot mix a pair the record itself calls incomparable.

`measure verify` writes no artifact; it prints its report, so a verification cannot overwrite the record it verifies.

A run publishes last. The stage capture runs into a hidden staging directory beside `--out`, and the artifacts are named only when every check has passed: a failed measurement leaves neither a record nor a half-published `evidence/`, so the same destination stays usable. A reader is therefore allowed to treat the presence of a record as the presence of a complete measurement.

A retained bundle is read through its own manifest: the buffer digest is verified against the manifest before any value is attributed to the run, so damaged bytes are an error rather than a reading.
