# Measure delivered images

`spektrafilm-lab` measures images that already exist: a render, an export, a scan.
It never renders, and it presents nothing — it publishes records. This page covers
the instrument itself: [`benchmark.md`](benchmark.md) covers the evaluation around
it, and [`measurement.md`](measurement.md) covers chain measurement on a retained
run.

## What it reads

One raster (`PNG`, `TIFF`, `EXR`) or a directory of them, under a declaration you
state:

```
spektrafilm-lab analyze --source render.png --declaration sRGB:encoded --out /tmp/a
```

`--declaration` is `SPACE:STATE`. The space is one of `sRGB`, `Display P3`,
`DCI-P3`, `Adobe RGB (1998)`, `ProPhoto RGB`, `ITU-R BT.2020`, `ACEScg`,
`ACES2065-1`; the state is `encoded` or `decoded`. Both halves are closed, so a
typo is refused while the arguments are read — nothing is created.

To measure two images and their difference, point the same flags at a pair:

```
spektrafilm-lab compare --baseline a.png --candidate b.png \
  --declaration sRGB:encoded --roi flat=0,0,64,64:flat_field --out /tmp/c
```

A directory source measures every image in it; a file that is not a container this
instrument reads is named and refuses the batch rather than being skipped.

## Regions

A region is a named rectangle with a role; the role decides which families can be
measured there. Pass one or more `--roi name=x,y,w,h[:role]`, or author a document
and pass `--regions file.json`. `regions` writes such a document from a grid
without reading any image:

```
spektrafilm-lab regions --id g --rows 3 --columns 4 --origin 0,0 \
  --patch-size 64x64 --gap 0,0 --space render --role flat_field --out grid.json
```

Geometry is in delivered pixels. The role is required, because a generator does not
decide what a region is for.

## What a row says

A record holds one row per measured quantity:

```json
{"region_id": "flat", "family": "grain", "metric": "grain.flat_mean.rms.R",
 "side": "baseline", "value": 0.0975531744255464, "units": "linear-sRGB sample RMS",
 "sample_count": 3844, "availability": "available", "estimator": "…", "domain": "…"}
```

- `metric` is the name of one physical quantity, and it is the same in every record
  that measures it. `analyze` publishes `grain.flat_mean.rms.R`; a comparison
  publishes the same name with `side` set.
- `side` exists only when a quantity is measured on more than one source:
  `baseline`, `candidate`, or the `delta` between them. A one-source record states
  none.
- `availability` is `available` or `unavailable`. An unavailable row states
  `reason` and no value. The instrument never publishes a zero it did not measure.

## Reading a record

A record is a directory: `analysis.json` or `comparison.json`, the retained
artifacts it references, and the materialized protocol with its digest. Query rows
without reading the file layout:

```
spektrafilm-bench inspect --comparison /tmp/c --metric grain.flat_mean.rms.R --side baseline
```

To look at a pair, build the offline page:

```
spektrafilm-bench report comparison --comparison /tmp/c --out /tmp/report
```

## Schema and validation

One endpoint owns the document vocabulary:

```
spektrafilm-lab document --kind regions            # print the schema
spektrafilm-lab document --kind regions --check grid.json
```

A kind this binary prints but cannot decide is refused with
`unvalidatable_document_kind`, which names the difference instead of pretending the
kind does not exist.

## What it does not do

- It does not render, and it does not link a renderer.
- It does not present. Pages belong to `spektrafilm-bench`.
- It does not apply acceptance rules. Policies and gates belong to
  `spektrafilm-bench compare runs`.
- It does not fit or calibrate. It measures what exists.
