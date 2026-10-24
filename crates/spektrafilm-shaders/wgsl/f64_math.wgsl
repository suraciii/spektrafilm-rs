// Explicit binary64 arithmetic avoids driver extended-instruction narrowing.
// Range reduction keeps the log series argument below 0.172 and the exp
// series argument below ln(2)/2; neither function calls log/exp/pow builtins.
fn sf64_log(value: f64) -> f64 {
  if value != value || value > 1.7976931348623157e308 { return value; }
  if value < 0.0 { return (value - value) / (value - value); }
  if value == 0.0 { return (value - 1.0) / abs(value); }
  var x = value;
  var exponent = 0i;
  loop {
    if x >= 0.7071067811865475244 { break; }
    x = x * 2.0;
    exponent = exponent - 1i;
  }
  loop {
    if x < 1.4142135623730950488 { break; }
    x = x * 0.5;
    exponent = exponent + 1i;
  }
  let z = (x - 1.0) / (x + 1.0);
  let z2 = z * z;
  var term = z;
  var sum = z;
  for (var k = 3u; k <= 43u; k = k + 2u) {
    term = term * z2;
    sum = sum + term / f64(k);
  }
  // Split ln(2) keeps the range-reduction residual near one ulp.
  return f64(exponent) * 0.69314718036912381649
    + (2.0 * sum + f64(exponent) * 1.9082149292705877000e-10);
}

fn sf64_exp(value: f64) -> f64 {
  if value != value || value > 1.7976931348623157e308 { return value; }
  if value < -745.1332191019411 { return f64(0.0); }
  if value > 709.782712893384 { return value / (value - value); }
  let scaled = value * 1.4426950408889634074;
  let exponent = i32(select(ceil(scaled - 0.5), floor(scaled + 0.5), scaled >= 0.0));
  let r = (value - f64(exponent) * 0.69314718036912381649)
    - f64(exponent) * 1.9082149292705877000e-10;
  var term: f64 = 1.0;
  var sum: f64 = 1.0;
  for (var k = 1u; k <= 20u; k = k + 1u) {
    term = term * r / f64(k);
    sum = sum + term;
  }
  if exponent > 0i {
    for (var k = 0i; k < exponent; k = k + 1i) { sum = sum * 2.0; }
  } else {
    for (var k = 0i; k > exponent; k = k - 1i) { sum = sum * 0.5; }
  }
  return sum;
}

fn sf64_powi(value: f64, exponent: u32) -> f64 {
  var base = value;
  var power = exponent;
  var result: f64 = 1.0;
  loop {
    if power == 0u { break; }
    if (power & 1u) != 0u { result = result * base; }
    power = power >> 1u;
    if power != 0u { base = base * base; }
  }
  return result;
}

fn sf64_round(value: f64) -> f64 {
  if value == 0.0 { return value; }
  if abs(value) >= 4503599627370496.0 { return value; }
  let magnitude = abs(value);
  let whole = floor(magnitude);
  let rounded = whole + select(f64(0.0), f64(1.0), magnitude - whole >= 0.5);
  return select(-rounded, rounded, value >= 0.0);
}

// Sign-preserving binary64 power. The driver's native `pow` is not used: on
// the software Vulkan adapter it returns zero for ordinary inputs such as
// pow(0.97, 2). `exp(power * log(|value|))` with the sign applied outside is
// the validated composition of the explicit routines above. Zero returns
// zero, NaN propagates, and non-finite inputs follow the trimmed 56-output
// contract of `sf64_exp`/`sf64_log`.
fn sf64_pow(value: f64, exponent: f64) -> f64 {
  if value == 0.0 || value != value { return value; }
  return sign(value) * sf64_exp(exponent * sf64_log(abs(value)));
}
