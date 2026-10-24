// Scanner-setup source accessor for a reference-precision (binary64) input
// buffer. The shared part of the reduction is `scanner_setup_lib.wgsl`; this
// file only states how stored samples become reference-precision values.

@group(0) @binding(1) var<storage, read> setup_source: array<f64>;

fn source_value(index: u32) -> f64 {
  return setup_source[index];
}
