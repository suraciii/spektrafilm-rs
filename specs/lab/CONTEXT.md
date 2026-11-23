# Measurement instrument

This context defines the language of the `spektrafilm-lab` instrument. The instrument measures the color science of delivered images and publishes structured records; it never renders, and it judges nothing.

## Language

### What is measured

**Delivered image**: A raster that a render or a delivery produced, read as pixels under one declared color space and transfer state. It is not a raw master or a working buffer.
_Avoid_: Output, result, capture, screenshot

**Source**: One delivered image set read under one declaration. In a comparison a source takes the role of baseline or candidate; in an analysis there is one source and no second one.
_Avoid_: Reference, input, end, sample

**Declaration**: The color space and transfer state a caller states for a source's samples, because the samples cannot be trusted to state them. The instrument never infers it from a file name or metadata.
_Avoid_: Metadata, properties, assumption

**Case**: One source within one invocation, identified by its file stem. A comparison is a set of cases, one per file name; an analysis is a set of cases, one per file.
_Avoid_: Pair, item, entry

### Where it is measured

**Region**: A named rectangle in delivered-pixel space, with one role. The whole raster is one implicit region, so every measurement belongs to a region.
_Avoid_: Population, window, ROI, patch, sample

**Region role**: What a region is a population of, such as a flat field or a neutral patch. It decides which families are measured there and carries no other meaning.
_Avoid_: Type, kind, category, label

### What is computed

**Family**: A named group of measurements that describe one physical quantity. A profile selects it; a reader groups by it; a caller does not name it.
_Avoid_: Metric group, suite, category, dimension

**Measurement**: One measured quantity: its metric identity, its availability, and either its value or the reason it does not exist.
_Avoid_: Readout, result, statistic, row, number

**Metric**: The stable identity of one measurement, such as `delta_e00.p95`. It names the quantity and nothing else: neither the family nor the source is part of it, so every record that measures the quantity uses the same identity.
_Avoid_: Key, field, name, label

**Side**: Which source a measurement row stands for: `baseline`, `candidate`, or the `delta` between them. It exists only when the same quantity is measured on more than one source; a one-source record states no side.
_Avoid_: Role, position, end, direction, channel

**Protocol**: The materialized measurement recipe a record retains and digests, so the numbers are reproducible from the record alone. An analysis profile generates it.
_Avoid_: Settings, configuration, parameters, options

**Analysis profile**: A named recipe a caller selects. It materializes one protocol and declares which families each region role measures.
_Avoid_: Preset, mode, analysis, method

### What is published

**Record**: The structured result of one invocation, either an analysis of one source or a comparison of two.
_Avoid_: Report, manifest, evidence, document, artifact

**Artifact**: A file a record retained and references, with its digest. A record is self-describing through its artifacts.
_Avoid_: Evidence, attachment, output

## Boundaries

A comparison's second source is a source in the candidate role, not a reference: color error is relative, and an analysis without a second source records that it has none rather than inventing one.
