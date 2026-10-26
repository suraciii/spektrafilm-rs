// Shared NumPy MT19937 words. Each distribution owner defines its arithmetic.
// The including owner supplies `mt` slots with idx, seeded and state fields.
const MT_N: u32 = 624u;
const MT_M: u32 = 397u;
const MT_MATRIX_A: u32 = 0x9908b0dfu;
const MT_UPPER_MASK: u32 = 0x80000000u;
const MT_LOWER_MASK: u32 = 0x7fffffffu;

/// `Mt::reseed`: the reference's seed-to-state recurrence, verbatim.
fn mt_reseed(plane: u32, seed: u32) {
  mt[plane].idx = MT_N;
  mt[plane].state[0] = seed;
  for (var i = 1u; i < MT_N; i = i + 1u) {
    let prev = mt[plane].state[i - 1u];
    mt[plane].state[i] = 1812433253u * (prev ^ (prev >> 30u)) + i;
  }
  mt[plane].seeded = 1u;
}

/// `rand_mt::fill_next_state`: the twist over the whole block, with the
/// wrap-around pair the reference writes explicitly.
fn mt_twist(plane: u32) {
  for (var i = 0u; i < MT_N - MT_M; i = i + 1u) {
    let x = (mt[plane].state[i] & MT_UPPER_MASK) | (mt[plane].state[i + 1u] & MT_LOWER_MASK);
    mt[plane].state[i] = mt[plane].state[i + MT_M] ^ (x >> 1u)
      ^ select(0u, MT_MATRIX_A, (x & 1u) != 0u);
  }
  for (var i = MT_N - MT_M; i < MT_N - 1u; i = i + 1u) {
    let x = (mt[plane].state[i] & MT_UPPER_MASK) | (mt[plane].state[i + 1u] & MT_LOWER_MASK);
    mt[plane].state[i] = mt[plane].state[i + MT_M - MT_N] ^ (x >> 1u)
      ^ select(0u, MT_MATRIX_A, (x & 1u) != 0u);
  }
  let x = (mt[plane].state[MT_N - 1u] & MT_UPPER_MASK) | (mt[plane].state[0u] & MT_LOWER_MASK);
  mt[plane].state[MT_N - 1u] = mt[plane].state[MT_M - 1u] ^ (x >> 1u)
    ^ select(0u, MT_MATRIX_A, (x & 1u) != 0u);
  mt[plane].idx = 0u;
}

/// `rand_mt::temper`, verbatim.
fn mt_temper(input: u32) -> u32 {
  var x = input;
  x = x ^ (x >> 11u);
  x = x ^ ((x << 7u) & 0x9d2c5680u);
  x = x ^ ((x << 15u) & 0xefc60000u);
  return x ^ (x >> 18u);
}

/// `Mt::next_u32` for one plane.
fn mt_next(plane: u32) -> u32 {
  if (mt[plane].idx >= MT_N) {
    mt_twist(plane);
  }
  let x = mt[plane].state[mt[plane].idx];
  mt[plane].idx = mt[plane].idx + 1u;
  return mt_temper(x);
}
