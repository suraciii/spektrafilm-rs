// Keep the vendored patch's unit tests in the workspace test lane. The vendor
// crate itself is not a workspace member and cannot use `cargo test -p`.
#[path = "../../../vendor/wgpu-hal/src/vulkan/procedural_math.rs"]
mod procedural_math;

#[test]
fn production_marker_selects_procedural_and_excludes_other_grain_engines() {
    let procedural = include_str!("../src/wgpu_backend/grain/procedural.wgsl");
    let spektra = concat!(
        include_str!("../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
        include_str!("../src/wgpu_backend/grain/work_f32.wgsl"),
        include_str!("../src/wgpu_backend/grain/spektra/spektra.wgsl")
    );
    let mt = concat!(
        include_str!("../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
        include_str!("../src/wgpu_backend/grain/work_f32.wgsl"),
        include_str!("../../spektrafilm-shaders/wgsl/mt19937.wgsl"),
        include_str!("../src/wgpu_backend/grain/mt19937/mt19937.wgsl")
    );
    // A small well-formed stream with one ordinary add. Feed the actual parsed
    // production modules to the selector; old shared-marker selection fails.
    let words = vec![
        0x07230203,
        0x00010000,
        0,
        10,
        0,
        (2 << 16) | 19,
        1,
        (5 << 16) | 129,
        1,
        2,
        3,
        4,
    ];
    for (source, selected) in [(procedural, true), (spektra, false), (mt, false)] {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        let after = procedural_math::preserve_operations(&module, words.clone()).unwrap();
        assert_eq!(after != words, selected);
    }
}
