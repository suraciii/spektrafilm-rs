// Packed image-independent channel coefficients shared by G2 and G4.
// Explicit padding on Rust side follows vec4 alignment (80 bytes/channel).
struct Table {
    min: f32,
    cap_total: f32,
    cap: vec4<f32>,
    frac: vec4<f32>,
    area: vec4<f32>,
    uniformity: f32,
    knot_count: u32,
    offset: u32,
    padding: u32,
}
