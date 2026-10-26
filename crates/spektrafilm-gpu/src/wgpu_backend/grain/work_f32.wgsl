// Spektra's NumPy binary32 working-array boundaries.
// Reconstruct promotion from binary32 bits. A numeric f64(f32(expression))
// cast pair can lose the required binary32 boundary on Vulkan drivers.
fn sf64_from_f32(value: f32) -> f64 {
  let bits = bitcast<u32>(value);
  let sign = u64(bits & 0x80000000u) << 32u;
  let exponent = (bits >> 23u) & 0xffu;
  let fraction = bits & 0x7fffffu;
  if exponent == 0u {
    let magnitude = f64(fraction) * 1.401298464324817070923729583289916131280e-45;
    return select(magnitude, -magnitude, sign != u64(0u));
  }
  let promoted_exponent = select(exponent + 896u, 2047u, exponent == 255u);
  return bitcast<f64>(sign | (u64(promoted_exponent) << 52u) | (u64(fraction) << 29u));
}

// A native WGSL f32 divide may use an approximate reciprocal. Grain's
// probability and Poisson rate must instead match NumPy's f32 ufunc stores:
// promote the already-rounded operands, divide, then round the quotient once.
fn sf32_div(numerator: f32, denominator: f32) -> f32 {
  return f32(sf64_from_f32(numerator) / sf64_from_f32(denominator));
}

// Explicit narrowing prevents contraction across NumPy's separate f32 stores.
fn sf32_mul(left: f32, right: f32) -> f32 {
  return f32(sf64_from_f32(left) * sf64_from_f32(right));
}

fn sf32_sub(left: f32, right: f32) -> f32 {
  return f32(sf64_from_f32(left) - sf64_from_f32(right));
}
