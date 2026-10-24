# Instrument design

## Boundary

- `SpektraFilm` remains the renderer. The instrument (`spektrafilm-lab`) is an external evaluation orchestrator, artifact producer, and in-process chain measurer.
- It consumes the renderer's public CLI, machine-readable dry-run output, floating image output, and diagnostics, and it shares the renderer's parameter surface and chain execution for readings. It does not import renderer internals or duplicate runtime parameter resolution.

- The instrument owns suite selection, target identity, dependency hashing, run isolation, comparison policy, metric execution, region selection, evidence indexing, comparison report generation, and the development measurement of an executed chain. A metric adapter owns one measurement implementation.
- The report builder owns presentation of an existing comparison only. Prepared-image review and the benchmark-owned Photo Report are outside the instrument boundary; their image inventories and visual-report builders belong to separate benchmark commands.

It is one binary over two libraries that stay separate: the evaluation library stays renderer-independent and depends only on the shared mathematics crate, and the command plumbing shared with the user binaries stays a library rather than forked code. A new development command lands in the instrument, not in `spektrafilm`, `spektrafilm-f64`, or the evaluation library.

Ordinary suite/image execution MUST NOT offer optional native capture. Dedicated instrument endpoints own research observation. Human debug diagnostics and resumable film/print artifacts remain separate contracts.

- The instrument MUST keep rendering and evaluation separate. A run produces immutable render evidence.
- A comparison reads runs and produces immutable measurements; a report reads a retained comparison and produces review artifacts; the retained comparison selects the builder. This allows an agent to rerun only the failed stage and allows a person to inspect a report when the renderer is unavailable.
- Preflight shares renderer configuration resolution with execution and publishes a separate validation record. It never enters the render-evidence lifecycle.
- Target construction owns executable pinning for both local commands and fixtures. The inspection view reads validated comparison records, projects existing measurements, and leaves metric computation and policy evaluation with their current owners.

```mermaid
flowchart LR
    S[Suite] --> R[Run orchestrator]
    T[Target] --> R
    R --> X[Renderer CLI]
    X --> M[Render masters and diagnostics]
    M --> C[Comparison engine]
    P[Policy] --> C
    C --> E[Measurements and findings]
    E --> H[Review report]
    E --> G[Optional gate status]
```

## Surfaces and stability

The command grammar, the document registry, and the machine-output contract live in the [specification](spec.md); this document states the boundaries behind them.

- Stability is bound to a document kind and its schema version, never to a command name.
- A supported command keeps its meaning across revisions; an experimental command MAY change, but its results MUST still satisfy the document, artifact, and machine-output contracts below. A retained record stays readable: a reader ignores unknown fields inside a supported version and fails on an unsupported version rather than guessing a default.

## Unix-style boundary

- The instrument is a collection of smart endpoints, not a research workflow manager. Pipes, files, and retained directories carry bytes and identities without interpreting domain meaning.
- The endpoint at each boundary owns parsing, validation, metric semantics, and publication. An endpoint MUST NOT call a downstream endpoint or consume human-readable output from another process.

The stable composition is:

```text
authored declaration → run → retained evidence → sample → retained readings → compare → inspect/report
```

The result of one endpoint is a valid input to the next, but no endpoint requires a particular caller. A Python script, shell command, or another implementation MAY replace any edge if it preserves the declared document or retained-record contract. This is the research extensibility boundary; it is not a reason to add a generic plugin or expression language.

The instrument MUST keep stdout, stderr, and named artifacts independent. stdout is one machine-readable result object; stderr contains progress, human diagnostics, and child-process text; named files contain evidence. A command MUST NOT use terminal detection to change the machine contract, and it MUST NOT parse another command's stderr.

## DSL boundary

The renderer's `--preset`, `--params`, and `--set` language is the only parameter DSL. Instrument documents MAY carry the same typed process options and source ordering, but they do not add aliases, arithmetic, units, includes, environment interpolation, expressions, shell commands, conditionals, loops, wildcard selectors, or optimizer instructions. The loader produces a semantic declaration before execution; parser output is not the execution model.

Instrument-only facts stay outside the renderer language: case and region identities, stage selection, seed identity, source declarations, artifact destinations, protocol selection, and retained-record roles. They are typed endpoint inputs, not runtime parameter paths.

- An experiment document is a durable declaration for one experiment endpoint. It is not a job graph, sweep language, optimizer program, or complete process job file.
- It MAY select one case, carry process sources, identify input bytes, and declare the evidence that one endpoint MUST publish. It MUST NOT acquire a second way to express renderer parameters.

## Batch boundary

- Repeated research trials belong to an outer producer and the existing endpoints. A future batch endpoint MAY accept a finite list of already-authored experiment declarations and execute them independently, publishing one retained record per item plus an index.
- It MUST NOT generate parameter variants, evaluate objectives, choose a best item, branch on measurements, or invoke shell commands. Candidate generation and optimization remain outside the instrument.

The first implementation slice MUST NOT add batch, parallelism, cache, or optimizer semantics. It MUST make the existing single-endpoint pipe usable from retained run records, then add a finite declaration-list carrier only after seed and record identity semantics are correct.

## Endpoint options

- Two designs were considered for experiment research automation. A workflow DSL could express variants, dependencies, and selection in one document, but would duplicate shell orchestration, add a second parameter language, and make provenance depend on hidden control flow.
- A finite declaration-list endpoint keeps each experiment independently reproducible and leaves control flow to the caller. The latter is selected because it preserves dumb pipes and smart endpoints.

## Record and failure boundaries

- Each endpoint publishes at most one terminal record for its destination. The record is the completion token; its presence means the referenced artifacts have been finalized, hashed, and validated for the supported schema.
- A failed operation MUST NOT publish a successful terminal record. A partial staging directory MAY remain only when it is explicitly marked incomplete and cannot be consumed as a completed input.

The pipe distinguishes these boundaries:

| Boundary | Meaning | Downstream behavior |
|---|---|---|
| argument syntax | the invocation cannot be parsed | no endpoint work; exit `2` |
| declaration validation | authored input or local dependency is invalid | no renderer or metric execution; exit `1` |
| execution failure | renderer, stage capture, or computation failed | failed result MAY retain structured partial evidence; never a completed record |
| measurement unavailable | the record is valid but a metric has no eligible observation | comparison MAY complete with reason; never a zero value |
| policy/gate failure | computed evidence does not satisfy a selected policy | completed comparison remains readable; gate changes status only |
| publication failure | final artifact cannot be committed | no success result; source records remain unchanged |

- An endpoint MUST validate the input kind before reading a shape-specific payload. A retained `kind` and schema version are authoritative.
- Consumers MAY ignore unknown fields inside a supported version, but they MUST NOT infer a different kind from field presence. A record that names a downstream artifact MUST retain its relative path, digest, media/layout declaration, and producer identity so an external tool can replace the next endpoint without losing provenance.

The command result and the retained record have different roles. stdout identifies the command outcome and terminal artifact; the retained record carries the full evidence. A command MUST NOT serialize a large buffer, progress stream, or human explanation into stdout as an undocumented side channel.

## Delivery sequence

The implementation is staged so each slice preserves the current endpoint contracts:

1. **Semantic handlers.** Split the existing flat `run` and `compare` dispatch internally into explicit suite/image/stimulus and runs/readings/pairs handlers. Keep the current grammar and output unchanged; remove field-presence dispatch as the source of semantic meaning.
2. **Retained-run sampling.** Add a run-record adapter to `sample` that resolves one case, seed, stage, and buffer from a retained run, verifies the native manifest, and delegates to the existing evidence reader. It MUST NOT duplicate stage decoding or region statistics.
3. **Experiment seed correctness.** Either publish one retained record per declared seed with an explicit index or reject multi-seed experiment documents before rendering. Do not add batch semantics before this identity is correct.
4. **Fixture endpoint.** Ship one executable 45-region fixture and one explicit non-grid regions fixture that exercise `run suite` or `run image`, `sample`, `compare runs`, and `inspect` as separate endpoints. The fixture MUST NOT depend on an optimizer or a universal score.
5. **Explicit grammar.** Expose the semantic subcommands defined by the instrument and lab specifications. Keep flat forms only as compatibility aliases while their behavior is identical and documented; remove the aliases in a later compatibility revision.
6. **Terminal result metadata.** Add complete artifact identity to command results without introducing workflow control fields or changing endpoint ownership.
7. **Finite declaration lists.** Only after the previous slices are stable, consider a simple batch carrier containing independently authored experiment declarations. It MAY execute and index them; it MUST NOT define variants, conditions, objective selection, caching, or parallel execution.

The first implementation slice is therefore not a `sweep` command. It is the smallest complete pipe from retained evidence to retained comparison, with correct seed and record identity. External autoresearch code remains the caller that generates the next declaration.

## Determinism and build

Instrument commands read declared inputs and write named outputs. They MUST NOT compare against budgets or emit pass/fail verdicts of their own; interpretation belongs to the caller or to a policy document. An execution MUST be a pure function of its inputs: identical inputs produce identical artifacts.

- The binary is built at f64 reference precision through the shared `precision-f64` gate, because a chain measurement MUST reproduce the reference path. Dedicated `experiment run` and `measure run` resolve with shared parameter plumbing and execute the shared core with its common observer.
- They do not drive research capture through `process` flags or a duplicate stage walk; their records identify the owning replayable instrument invocation, dependencies, and actual observer configuration, not invented process argv. Ordinary image-domain suite/image orchestration still executes the real renderer CLI and retains image masters and diagnostics without optional native capture.

Release packaging and its compatibility promise are defined by [Rust delivery](../benchmark/rust-delivery/spec.md).

## Measurement boundary

A measurement shares the parameter surface and the execution path of the user binaries. It MUST NOT resolve parameters on its own, reconstruct pipeline mathematics, or walk the stages beside an observed execution. A stage walk beside the executed chain would report a different result under a stochastic stage, so the observed path is the only permitted source of readings.

The measured range is the same fact for both surfaces: `--enter` becomes the resolved `taps.inject` and the collection boundary is the chain's own `taps.collect` or the last requested reading. Nothing else decides what executes, so the retained effective document describes the execution and a re-verification reproduces it.

- The executed chain is resolved per invocation. A resolved parameter document is evidence, never a declaration.
- A command MUST NOT accept a stored resolution as its chain input, because nothing relates that document to the profiles, the data inventory, and the executable present at run time. This applies the resolution stance of the [CLI design](../cli/design.md) to measurement: resolve before executing, and keep the resolution beside the result instead of passing it between commands.

The instrument MUST NOT own a second implementation of a metric. A color-difference, equal-lightness, or region-aggregation algorithm keeps one owner, and the measurement commands consume it.

## Reading boundary

- Readings are defined only for a point-wise chain: an injected numeric sample has no neighborhood and no stochastic realization. An operation that mixes samples, measures the whole frame, resizes it, or draws stochastic values would describe a different signal than the measured one.
- The command fails before executing instead of reporting those numbers; the [point-wise table](spec.md#point-wise-requirement) is the specification of that check.

Eligibility is a measured property, not a reading of a flag name: each operation carries the first stage whose values it changes, and it is listed only when it executes inside the measured range. The instrument MUST NOT widen that table by guessing that a control is spatial because its name suggests so.

- A [regions document](spec.md#regions-document) is the only authority for a reading position. `regions grid` expands one finite grid into that document without reading images or producing a report.
- A command MUST NOT infer geometry from image dimensions, labels, color similarity, or a removed review manifest. A sample outside the declared source remains an error rather than a clamped value.

A guard run is a frozen measurement artifact together with the stimuli that produced it. It carries no tolerance and no verdict; a tolerance belongs to the protocol document that declares it. Frozen readings make chain drift visible, so a comparison MUST report an identity difference beside the value difference instead of refusing the pair.

## Artifact boundary

- A record is a directory: the record document carries declarations and identity, its table view carries numbers with their identities, and dedicated research `evidence/` carries stage buffers under the instrument-owned [native observation contract](native-observations.md).
- Comparison, verification, and later reproduction read that same directory; there is no second retained-reading format. Native observations are not resumable `.sfstage` states and cannot be obtained from human debug output.

- A run publishes last.
- The stage capture runs into a hidden staging directory beside `--out`, and the artifacts are named only when every check has passed: a failed measurement leaves neither a record nor a half-published `evidence/`, so the same destination stays usable. A reader is therefore allowed to treat the presence of a record as the presence of a complete measurement.

A retained bundle is read through its own manifest: the buffer digest is verified against the manifest before any value is attributed to the run, so damaged bytes are an error rather than a reading.

## Chosen boundary options

- A renderer-integrated evaluation system would remove subprocess setup but bind historical image evaluation to the current runtime. A standalone instrument preserves image comparisons across renderer executables and retained artifacts.
- The selected boundary therefore keeps ordinary suite/image rendering behind the real renderer CLI while dedicated experiments and point-wise measurements use the shared core observer in-process. Ownership is explicit rather than a process capture option: instrument research owns retention, and the core owns executed values.

- The instrument MUST NOT add a second parameter DSL. Suite and target documents MAY reference the renderer's existing `--preset`, `--params`, and `--set` carriers.
- The instrument validates their shape and passes them to the renderer in the renderer's defined precedence order. Instrument-only settings use typed JSON fields and have no shell or expression evaluation.

The delivered instrument MUST use the Rust workspace implementation defined in [Rust delivery](../benchmark/rust-delivery/spec.md). Its public document contracts remain language-neutral. Native C and C++ metric dependencies MAY remain behind narrow adapters.

The single-image CLI adapter MUST build the existing suite and case representation and call the existing run orchestrator. It MUST resolve each authored dependency against its owning document before producing the retained suite. It MUST NOT add chart detection or a separate simulation-parameter vocabulary.

- Externally preparing all images would keep the instrument smaller but would discard original metering and film-scale context for simulation crops. An instrument image editor would duplicate those rules.
- The chosen boundary accepts prepared local files, delegates simulation geometry to the renderer, and owns only completed-result measurement and detail regions. Preparation provenance remains passive evidence.

### Standard configuration ownership

- The [standard configurations](../benchmark/standard-configurations/spec.md) extend fixture generation instead of adding a new preset kind. A special film profile would mix test conditions with material data.
- A look preset alone cannot own source interpretation, route, geometry, or seed populations. Existing configuration and suite documents own those settings.

The fixture generator MUST use one construction rule for each standard rendering purpose. It MUST write standalone configurations and embed equivalent values into suite cases from that rule. It MUST NOT maintain separate handwritten defaults for local-image and suite execution.

The renderer's preset export and parameter resolver remain authoritative for look fields, defaults, and profile compatibility. The instrument MUST NOT reconstruct the preset schema or scrape a human listing. Effect-specific probes MUST apply explicit existing parameter overlays after the exported look.

Generation MUST complete validation before publishing its final provenance record. Renderer export output MUST remain a file dependency rather than hidden generator state. Run orchestration, comparison, and report generation MUST consume the generated documents without a special standard-mode path.

Preset export captures the selected renderer's look at generation time. Reusing that file separates an algorithm revision from a bundled-look revision. Existing run identity MUST still include the renderer and selected data; a fixed preset file does not freeze material data or promise historical parameter-language compatibility.


## Component responsibilities

### Document loader

The loader parses strict JSON, rejects duplicate keys and unknown authored fields, resolves paths relative to the containing document, and validates the schema and cross-document identities. It MUST NOT resolve environment interpolation or execute document content.

### Dependency resolver

The resolver hashes executable bytes, selected data inventory, source files, presets, sparse parameter files, masks, transforms, references, and protocol inputs. It MUST produce a deterministic sorted inventory. Modification times and directory traversal order MUST NOT affect identity.

### Run orchestrator

The orchestrator selects cases, applies target changes after case configuration, invokes the renderer, captures stdout and stderr separately, validates the machine result, and records the actual output and diagnostics. It MUST use unique artifact paths and refuse destination replacement. It MUST remove incomplete case artifacts or mark them incomplete.

The orchestrator MUST execute the real `spektrafilm` or `spektrafilm-f64` entry point. It MUST request a floating master suitable for the selected measurement domain and an independent lossless review image. It MUST NOT use a lossy preview as the numerical master.

The orchestrator MUST retain the command vector after path normalization, selected environment allowlist, process identity, renderer result, and execution diagnostics. It MUST identify the renderer revision when the executable exposes one and retain the executable hash regardless.

- The orchestrator MUST retain an observed floating decoded-input result for both raster and RAW cases through the renderer's input route. Source and simulation MUST use the same case geometry and decoding controls.
- Source output conversion and disabled CMYK ink style MUST be explicit overrides; image-input and simulation behavior retain their existing owners. Source dimensions and actual result geometry MUST remain independent.

- The ordinary run orchestrator MUST NOT request optional native-stage capture for simulation, source-only execution, or preflight, and it MUST retain explicit observation absence. Dedicated experiment and measurement endpoints own [native observations](native-observations.md) through the common core observer, including actual exposure and profile-prepared normalization.
- No endpoint MAY infer those values from authored controls, repeat preparation mathematics, or rerender to backfill an ordinary or historical run. Existing supported captured run records remain readable; intermediate coordinates stay distinct from completed-image regions.

Comparison retention MUST reuse the run validation already performed for that run. It MUST NOT reload and rehash every case for each selected case or role. Copying requested evidence MUST preserve the bundle's relative dependency layout and artifact identities.

### Comparison engine

The comparison engine matches results by stable case and seed identities. It verifies protocol, dimensions, source declarations, effective parameter compatibility, and applicable reference scope before computing measurements. It MUST produce an explicit incompatibility result instead of silently aligning incompatible runs.

It MUST compute maps before summaries. Summary scalars MUST be derived from the retained map or curve and the declared valid mask. Automatic hotspot selection MUST use those same retained measurements.

- Single-run comparison MUST consume source and simulation masters from one run. It MUST reuse existing measurement adapters for input/simulation and existing reference evaluation for reference/simulation, without synthesizing a second simulation run or measuring a self-comparison as a substitute.
- Each pair owns its roles, measurement availability, maps, hotspots, and review artifacts; reference processing and correspondence limits MUST apply before metric selection. The report builder MUST consume these pair records without assigning reference images baseline status.

- Comparison owns regional color coordinates and native-stage aggregates. It MUST reuse the existing color transforms, resolved region populations, and regional medians.
- Per-region aggregation MUST decode each captured stage buffer at most once per run role. Inspection and report consumers MUST project these retained records without recalculation; the report MUST keep the two color-center estimators and native-stage domains visibly separate.

### Metric adapters

Each adapter MUST declare input domain, required metadata, output schema, availability reasons, deterministic implementation identity, and calibration dependencies. It MUST NOT inspect human log text. It MUST NOT silently transform an input to improve a score.

The first metric set is color and signed Lab diagnostics, tone response, numeric health, LDR-FLIP, flat-field grain statistics, bright-source halation profiles, and edge response. SSIM is optional and MUST NOT be a prerequisite for the first qualified protocol.

### Policy evaluator

The evaluator applies exact selectors and fixed constraints to comparison measurements. It MUST evaluate protected properties separately from expected changes and reference objectives. It MUST retain all rule outcomes even when one failure determines the overall status.

It MUST NOT infer budgets, tune parameters, choose a more favorable reference, or rewrite an unavailable measurement. It MAY produce `insufficient_evidence`.

### Report builder

- The report builder creates an offline report from comparison artifacts. It MUST NOT render or recalculate metrics.
- It MUST preserve artifact hashes and relative paths referenced by the immutable comparison snapshot. Permitted indexed dependencies MUST remain locally resolvable after the original comparison is removed; redistribution restrictions and unavailable artifacts MUST remain explicit; it MUST emit accessible full-image, A/B, locator, detail, and difference views.

Case navigation MUST preserve request order. Pair selection MUST change the complete evidence panel, including labels, scalar summary, difference artifacts, regions, and native detail. Pixel-aligned pairs use synchronized viewers; regional and visual pairs use separately identified native images and corresponding regions where supplied.

Report fragment navigation identifies case, seed, pair, and region by retained semantic identities. Internal DOM ordering MUST NOT define public location identity. Accessible disclosure controls and direct links belong to the report builder; neither navigation nor inspection MAY recalculate a measurement.

Region inspection MUST NOT become a second evaluation pipeline; it MUST use retained measurements. A selected visual pair without measured evidence MUST remain visually inspectable with an explicit unavailable numeric state. Native detail and numerical artifacts retain their existing owners.

## Identity and cache invariant

A render result is reusable only when its suite, case, target, renderer executable, data inventory, source input, parameter files, protocol-independent render settings, selected environment, and seed identities match. A comparison result is reusable only when its two run identities, protocol digest, policy digest, and metric implementation identities match.

- The instrument implementation identity MUST cover local workspace dependencies that implement measurements, including shared color and Lab mathematics. A path dependency without a lockfile change MUST NOT escape this identity.
- Dedicated research observer configuration MUST be retained separately from image settings; ordinary suite/image records retain explicit absence. Observed stage availability MUST never be inferred from a rendered image or restored by an implicit rerender.

The instrument MUST never reuse a stale result because a path stayed the same. It MUST reject a changed dependency before using the cache. It MUST NOT use a partial or failed run as a qualified baseline.

## Coordinate and color invariant

Every artifact MUST declare its coordinate space. Renderer coordinates, transformed comparison coordinates, common-area masks, review image coordinates, and native detail coordinates MUST remain distinct. A local transform MUST never be applied to an artifact without changing its identity and recording its purpose.

- Color conversion MUST occur at one named boundary for each metric. A map and its scalar summary MUST use the same converted samples and valid mask.
- Display clipping MUST NOT alter native numerical measurements. A report MUST label the display copy separately from the numerical master.

## Stochastic invariant

- A fixed seed provides a repeatable realization under the same renderer conditions. It does not make unrelated seeds or precision paths equivalent.
- Deterministic probes MUST disable stochastic effects explicitly. Grain conclusions require the declared seed population and MUST retain per-seed observations.

## Failure and availability invariant

- The pipeline MUST distinguish configuration failure, renderer failure, measurement unavailability, policy failure, and report publication failure. Each boundary MUST retain stable machine codes and relevant artifacts.
- An unavailable metric MUST never serialize as zero. A failed policy gate MUST NOT delete the completed comparison.

A comparison can complete with `insufficient_evidence`, but a qualified pass requires all required measurements, compatible conditions, calibrated policy, and successful artifact validation. Human approval can add a review decision but cannot rewrite the measured result.

## Delivery

[Rust delivery](../benchmark/rust-delivery/spec.md) defines the delivered implementation's scope, distribution, workspace and execution boundary, numerical implementation, and protocol identity. The delivered implementation is the Rust workspace crate.
