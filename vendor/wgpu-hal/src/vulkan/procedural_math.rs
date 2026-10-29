//! Preserve separate rounding in the Procedural module's float-bit hash inputs.
//! This is not a general strict-IEEE mode. Explicit Fma instructions are unchanged.

const MARKER: &str = "spektrafilm_procedural_no_contraction";
const DECORATE: u32 = 71;
const NO_CONTRACTION: u32 = 42;

pub(super) fn preserve_operations(
    module: &naga::Module,
    mut words: Vec<u32>,
) -> Result<Vec<u32>, &'static str> {
    if !module
        .functions
        .iter()
        .any(|(_, f)| f.name.as_deref() == Some(MARKER))
    {
        return Ok(words);
    }
    if words.len() < 5 || words[0] != 0x07230203 {
        return Err("invalid Procedural SPIR-V header");
    }
    let mut cursor = 5;
    let mut insert = None;
    let mut targets = Vec::new();
    let mut decorated = std::collections::BTreeSet::new();
    while cursor < words.len() {
        let count = (words[cursor] >> 16) as usize;
        let op = words[cursor] & 0xffff;
        if count == 0 || count > words.len() - cursor {
            return Err("invalid Procedural SPIR-V instruction length");
        }
        // OpFAdd/Sub/Mul/Div and OpVectorTimesScalar: type, result, lhs, rhs.
        if matches!(op, 129 | 131 | 133 | 136 | 142) {
            if count != 5 {
                return Err("invalid Procedural SPIR-V arithmetic instruction");
            }
            targets.push(words[cursor + 2]);
        }
        if op == DECORATE && count >= 3 && words[cursor + 2] == NO_CONTRACTION {
            decorated.insert(words[cursor + 1]);
        }
        // Naga writes annotations before the first type declaration.
        if insert.is_none() && (19..=39).contains(&op) {
            insert = Some(cursor);
        }
        cursor += count;
    }
    let insert = insert.ok_or("missing Procedural SPIR-V type section")?;
    let annotations = targets
        .into_iter()
        .filter(|id| !decorated.contains(id))
        .flat_map(|id| [(3 << 16) | DECORATE, id, NO_CONTRACTION]);
    words.splice(insert..insert, annotations);
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "
        fn spektrafilm_procedural_no_contraction(v:f32)->f32 {return v;}
        @group(0) @binding(0) var<storage,read_write> values:array<f32>;
        @compute @workgroup_size(1) fn main() {
            let a=values[0];let b=values[1];
            values[2]=spektrafilm_procedural_no_contraction((a+b)*(a-b)/b);
            values[3]=fma(a,b,a);
            let v=vec3<f32>(1.425,3.892,5.835)*a*b;
            values[4]=v.y;
            values[5]=((vec2(a,b)+vec2(b,a))*(vec2(a,b)-vec2(b,a))/vec2(b,a)).x;
        }";

    fn emit(source: &str, entry: bool) -> (naga::Module, Vec<u32>) {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let pipeline = entry.then(|| naga::back::spv::PipelineOptions {
            shader_stage: naga::ShaderStage::Compute,
            entry_point: "main".into(),
        });
        let words = naga::back::spv::write_vec(
            &module,
            &info,
            &naga::back::spv::Options::default(),
            pipeline.as_ref(),
        )
        .unwrap();
        (module, words)
    }

    fn instructions(words: &[u32]) -> Vec<&[u32]> {
        let mut cursor = 5;
        let mut result = Vec::new();
        while cursor < words.len() {
            let n = (words[cursor] >> 16) as usize;
            result.push(&words[cursor..cursor + n]);
            cursor += n;
        }
        result
    }

    #[test]
    fn normal_and_entrypoint_emission_preserve_only_separate_arithmetic() {
        for entry in [false, true] {
            let (module, original) = emit(SOURCE, entry);
            let after = preserve_operations(&module, original.clone()).unwrap();
            let before_ops = instructions(&original);
            let after_ops = instructions(&after);
            let ids: std::collections::BTreeSet<_> = before_ops
                .iter()
                .filter(|op| matches!(op[0] & 0xffff, 129 | 131 | 133 | 136 | 142))
                .map(|op| op[2])
                .collect();
            assert!(ids.len() >= 10, "exercise scalar and vector arithmetic");
            assert_eq!(
                before_ops.iter().filter(|op| op[0] & 0xffff == 142).count(),
                2,
                "writer must emit the vector-times-scalar chain"
            );
            // GLSL.std.450 Fma=50, OpExtInst=12.
            assert!(before_ops
                .iter()
                .any(|op| op[0] & 0xffff == 12 && op[4] == 50));
            let is_nc = |op: &&[u32]| {
                op.len() == 3 && op[0] & 0xffff == DECORATE && op[2] == NO_CONTRACTION
            };
            let nc: Vec<_> = after_ops.iter().filter(|op| is_nc(op)).collect();
            assert_eq!(nc.len(), ids.len());
            assert_eq!(
                nc.iter()
                    .map(|op| op[1])
                    .collect::<std::collections::BTreeSet<_>>(),
                ids
            );
            assert_eq!(&original[..5], &after[..5]);
            assert_eq!(
                before_ops,
                after_ops
                    .iter()
                    .filter(|op| !is_nc(op))
                    .copied()
                    .collect::<Vec<_>>()
            );
            assert_eq!(preserve_operations(&module, after.clone()).unwrap(), after);
        }
    }

    #[test]
    fn shared_metal_marker_and_unmarked_modules_are_unchanged() {
        for marker in ["spektrafilm_strict_float", "ordinary_function"] {
            for entry in [false, true] {
                let (module, words) = emit(&SOURCE.replace(MARKER, marker), entry);
                assert_eq!(preserve_operations(&module, words.clone()).unwrap(), words);
            }
        }
    }

    #[test]
    fn malformed_selected_module_returns_error() {
        let (module, words) = emit(SOURCE, false);
        for invalid in [
            vec![],
            vec![0; 5],
            words[..5].to_vec(),
            {
                let mut w = words.clone();
                w[5] = 0;
                w
            },
            {
                let mut w = words.clone();
                w[5] = 0xffff0000;
                w
            },
        ] {
            assert!(preserve_operations(&module, invalid).is_err());
        }
    }
}
