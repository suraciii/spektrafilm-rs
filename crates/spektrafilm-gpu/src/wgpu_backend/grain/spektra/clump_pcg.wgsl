// Spektra PCG32 clumping field.
//
// The GPU Spektra realization uses PCG32 for both particle sampling and the
// optional lognormal micro-structure field. Each global pixel/channel owns its
// stream, so a bounded transport window never changes the field.

struct Params {
  first: u32,
  count: u32,
  base_seed: u64,
  micro_sigma: f64,
  monochrome: u32,
  pad: vec3<u32>,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> clump: array<f64>;

fn pcg_splitmix(x0: u32) -> u32 {
  var z = x0;
  z = (z ^ (z >> 16u)) * 0x85ebca6bu;
  z = (z ^ (z >> 13u)) * 0xc2b2ae35u;
  return z ^ (z >> 16u);
}

struct Pcg { state: u32, inc: u32 }

fn pcg_next(rng: ptr<function, Pcg>) -> u32 {
  let s = (*rng).state;
  (*rng).state = s * 747796405u + (*rng).inc;
  let word = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (word >> 22u) ^ word;
}

/// 53-bit uniform in [0, 1), matching the CPU double stream's resolution.
fn pcg_uniform(rng: ptr<function, Pcg>) -> f64 {
  let hi = u64(pcg_next(rng) >> 5u);
  let lo = u64(pcg_next(rng) >> 6u);
  return (f64(hi) * 67108864.0 + f64(lo)) / 9007199254740992.0;
}

/// Marsaglia polar normal. The rejection loop keeps clear of sin/cos and
/// preserves the same normal law as the reference.
fn pcg_normal(rng: ptr<function, Pcg>) -> f64 {
  var result: f64 = 0.0;
  loop {
    let u = 2.0 * pcg_uniform(rng) - 1.0;
    let v = 2.0 * pcg_uniform(rng) - 1.0;
    let s = u * u + v * v;
    if s > 0.0 && s < 1.0 {
      result = u * sqrt(-2.0 * sf64_log(s) / s);
      break;
    }
  }
  return result;
}

fn make_rng(global_i: u32, channel: u32) -> Pcg {
  let seed_lo = u32(p.base_seed) ^ pcg_splitmix(global_i) ^ pcg_splitmix(channel);
  let seed_hi = u32(p.base_seed >> 32u);
  let state = pcg_splitmix(seed_lo ^ seed_hi);
  let inc = (pcg_splitmix(global_i ^ 0x9e3779b9u) << 1u) | 1u;
  return Pcg(state, inc);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let local = gid.x;
  if local >= p.count { return; }
  let global_i = p.first + local;
  let variance = p.micro_sigma * p.micro_sigma;
  let sum = 1.0 + variance;
  let sigma2 = sf64_log(sum) * (variance / (sum - 1.0));
  let sigma = sqrt(sigma2);
  if p.monochrome != 0u {
    var rng = make_rng(global_i, 0u);
    let z = pcg_normal(&rng);
    let value = sf64_exp(z * sigma - 0.5 * sigma2);
    for (var ch = 0u; ch < 3u; ch = ch + 1u) {
      clump[local * 3u + ch] = value;
    }
    return;
  }
  for (var ch = 0u; ch < 3u; ch = ch + 1u) {
    var rng = make_rng(global_i, ch);
    let z = pcg_normal(&rng);
    clump[local * 3u + ch] = sf64_exp(z * sigma - 0.5 * sigma2);
  }
}
