use super::*;

fn byte_at(words: &[u32], index: usize) -> u8 {
    (words[index / 4] >> ((index % 4) * 8)) as u8
}

// Independent host application of the fixed device representation. Native
// LCMS below remains the reference, not this copy of the kernel arithmetic.
fn apply(kind: ViewerIccKind, words: &[u32], input: [u8; 3]) -> [u8; 3] {
    match kind {
        ViewerIccKind::Identity => input,
        ViewerIccKind::Curves => std::array::from_fn(|channel| {
            byte_at(words, channel * 256 + usize::from(input[channel]))
        }),
        ViewerIccKind::MatrixShaper => {
            let shaped: [i32; 3] = std::array::from_fn(|channel| {
                words[12 + channel * 256 + usize::from(input[channel])] as i32
            });
            std::array::from_fn(|channel| {
                let row = channel * 3;
                let value = (words[row] as i32)
                    .wrapping_mul(shaped[0])
                    .wrapping_add((words[row + 1] as i32).wrapping_mul(shaped[1]))
                    .wrapping_add((words[row + 2] as i32).wrapping_mul(shaped[2]))
                    .wrapping_add(words[9 + channel] as i32)
                    .wrapping_add(8192)
                    >> 14;
                byte_at(
                    &words[780..],
                    channel * 16385 + value.clamp(0, 16384) as usize,
                )
            })
        }
        ViewerIccKind::ExactRgb8 => {
            let index = (usize::from(input[0]) * 65536
                + usize::from(input[1]) * 256
                + usize::from(input[2]))
                * 3;
            std::array::from_fn(|channel| byte_at(words, index + channel))
        }
    }
}

#[test]
fn null_pipeline_and_q14_preparation_match_lcms_over_entire_rgb8_domain() {
    let source = Profile::new_srgb();
    let destination = Profile::new_icc(include_bytes!(
        "../../../../../data/icc/saucecontrol/DisplayP3-v4.icc"
    ))
    .unwrap();
    let transform: Transform<[u8; 3], [u8; 3]> = Transform::new(
        &source,
        PixelFormat::RGB_8,
        &destination,
        PixelFormat::RGB_8,
        Intent::Perceptual,
    )
    .unwrap();
    let (kind, exact) = prepare_transform(&transform, std::ptr::null());
    assert_eq!(kind, ViewerIccKind::ExactRgb8);
    assert_eq!(exact.len(), ViewerIccKind::ExactRgb8.word_count());
    let resolved = prepare([0; 32], &source, &destination).unwrap();
    // The pinned optimizer path must actually exercise its ABI and Q14 tables,
    // rather than letting a broken fast path silently pass via ExactRgb8.
    if lcms2::version() == 2190 {
        assert_eq!(resolved.kind, ViewerIccKind::MatrixShaper);
    }
    let mut expected = vec![[0; 3]; 65536];
    for red in 0..256 {
        for (index, pixel) in expected.iter_mut().enumerate() {
            *pixel = [red as u8, (index / 256) as u8, index as u8];
        }
        transform.transform_in_place(&mut expected);
        for (index, expected) in expected.iter().enumerate() {
            let input = [red as u8, (index / 256) as u8, index as u8];
            assert_eq!(apply(kind, &exact, input), *expected, "exact {input:?}");
            assert_eq!(
                apply(resolved.kind, &resolved.words, input),
                *expected,
                "Q14 {input:?}"
            );
        }
    }
}

#[test]
fn profile_identity_ignores_generated_metadata_but_not_conversion_data() {
    fn identity(profile: &Profile) -> [u8; 32] {
        let mut hash = Sha256::new();
        profile_identity(&mut hash, profile).unwrap();
        hash.finalize().into()
    }
    let profile = Profile::new_srgb();
    let mut bytes = profile.icc().unwrap();
    bytes[24..36].copy_from_slice(&[0x07, 0xd0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
    bytes[84..100].fill(0x5a);
    let changed_metadata = Profile::new_icc(&bytes).unwrap();
    assert_eq!(identity(&profile), identity(&changed_metadata));
    let mut changed_curve = Profile::new_icc(&bytes).unwrap();
    assert!(changed_curve.write_tag(
        lcms2::TagSignature::RedTRCTag,
        lcms2::Tag::ToneCurve(&lcms2::ToneCurve::new(1.8))
    ));
    assert_ne!(identity(&profile), identity(&changed_curve));
}
