# Photograph comparison and continued work

## Authority

This target extends [GUI product design](../spec.md). It owns comparison references, photograph adjustment records, and multi-photograph work. It does not claim that these capabilities are implemented.

[GUI workflow](../../gui-workflow/spec.md) owns edits, history, and render identity. [Presets](../../presets/spec.md) owns reusable looks. [Photo export](../../photo-export/spec.md) owns rendering and file publication.

## A/B comparison

A **comparison reference** is one captured, accepted adjustment state for the current source. B is the current editable state. Neither A nor B means the original input or the scanner's automatic baseline.

```mermaid
flowchart LR
    B[Current adjustments B] -->|Capture| A[Reference A]
    A --> V[Shared viewer]
    B --> V
    A -->|Adopt: one reversible edit| B
```

The viewer must alternate A and B with shared zoom, image location, and display transform. Each side must identify its quality, dimensions, and readiness. Unequal quality must be explicit.

Capture must exclude pending text and RAW drafts. It must retain material selections, workflow, explicit edit ownership, and preset context. A missing matching render must show pending or unavailable, not unrelated retained pixels.

Inspecting A must not edit B, add history, or change Save and Export inputs. Adopting A must follow [Edit history](../../gui-workflow/spec.md#edit-history), without rereading a changed preset file. A failed render must preserve B.

A must be session-local. Accepted source replacement, RAW reprocessing, or complete-state loading must clear it; failed replacement must preserve it. Incompatible geometry must disable synchronized comparison with an explanation. Multiple references and version trees are outside this target.

## Photograph adjustment records

A **photograph adjustment record** associates a source identity with accepted input interpretation, RAW settings, geometry, photographic adjustments, and material dependencies. It is not a preset or startup default.

```mermaid
flowchart LR
    S[Source and accepted adjustments] -->|Save record| R[Adjustment record]
    R --> C{Source and dependencies valid?}
    C -->|Yes| O[Reopen photograph]
    C -->|No| K[Explain; keep current photograph]
```

The GUI must offer explicit save and reopen actions. It must disclose excluded pending drafts. Records must exclude application preferences, undo stacks, jobs, and Export drafts.

Reopening must reject missing or changed dependencies before replacing the active context. Relocating a source must require explicit selection and identity validation. It must not silently substitute defaults or a same-named different file. Accepted reopening follows the complete-state history boundary in [GUI workflow](../../gui-workflow/spec.md#edit-history).

A failed record write must preserve the previous valid record. Record saving must not modify the source or preset. Closing or replacing unsaved accepted work must offer Save record, Discard, and Cancel; failed saving must stop the transition.

Records must disclose external dependencies. Embedded project packages, cloud synchronization, and crash recovery are outside this target.

## Work list

A **work list** is an explicitly selected set of photographs with one active entry. It is not a media catalog.

```mermaid
flowchart LR
    L[Selected photographs] --> E[Edit active photograph]
    E --> R[Save its adjustment record]
    E --> C[Copy selected adjustment scope]
    C --> D[Validate each destination]
    L --> Q[Submit selected entries]
    Q --> X[Export queue]
```

The GUI must support adding, removing, and moving between entries. Each entry must retain its own adjustments and record-save state. Switching must respect the unsaved-work boundary above. Failed loading must preserve the active photograph. A delayed result from another entry must not replace its output.

Removing an entry must not delete its source, record, or exported files. Single-photograph editing must not require a work list.

### Copy adjustments

Copy must require explicit scope and destination selection. It must disclose the controls replaced. Existing look, section, feature-profile, and operator-correction scopes remain authoritative.

Copy must preserve destination source interpretation, RAW settings, geometry, source calibration, and delivery settings. Automatic frame setup must use each destination, not copied prepared values. Each destination must commit atomically or remain unchanged, with an individual outcome. Successful application must be reversible in that destination's current history.

### Export queue

```mermaid
stateDiagram-v2
    [*] --> Waiting: Capture source, adjustments, destination
    Waiting --> Running
    Waiting --> Canceled
    Running --> Completed: Publication succeeds
    Running --> Failed
    Running --> Canceled: Cancellation takes effect
```

Submission must capture each entry independently of later edits or removal. Pending RAW changes must remain distinct from the accepted decode being exported. Duplicate destinations must be resolved before submission; replacement of existing files must require an explicit choice.

The queue must expose per-entry outcomes and cancellation. Live operation status stays in the [header](../../gui-workflow/spec.md#labels-and-surfaces). A failed entry must not stop unrelated work. Canceling remaining work must preserve completed files. [Photo export](../../photo-export/spec.md#publication) owns individual publication guarantees; the batch is not one atomic transaction.

Retry must distinguish repeating the failed submission from submitting current adjustments. Sequential execution is sufficient. Restart recovery and parallel rendering are outside this target.

## Excluded scope

This target does not add ratings, smart albums, folder watching, a media database, local masks, a node editor, or new film algorithms.
