use super::*;
use lcms2::{Intent, PixelFormat, Profile, Tag, TagSignature, ToneCurve, Transform};

fn samples()->Vec<[f32;3]>{
    let mut out=vec![[0.,0.,0.],[1.,1.,1.],[-0.1,0.5,1.1]];
    for byte in 0..256u32{
        let v=byte as f32/255.;
        // Both neighboring representable floats expose the viewer's truncation
        // boundary independently from LCMS's output-word rounding.
        for v in [f32::from_bits(v.to_bits().saturating_sub(1)),v,f32::from_bits(v.to_bits()+1)]{
            out.extend([[v,v,v],[v,0.,1.],[0.125,v,0.875],[1.,0.,v]]);
        }
    }
    let mut state=0x72df_14b9u32;
    for _ in 0..2048{
        let p=std::array::from_fn(|_|{state=state.wrapping_mul(1664525).wrapping_add(1013904223);(state>>8) as f32/16777215.});out.push(p);
    }
    out
}
fn compare(backend:&dyn ComputeBackend,name:&str,destination:&Profile){
    let input=samples();let source=Profile::new_srgb();
    // This is the unchanged application reference: default optimization,
    // perceptual intent, RGB8 at both ends, no alternate flags or cache policy.
    let transform:Transform<[u8;3],[u8;3]>=Transform::new(&source,PixelFormat::RGB_8,destination,PixelFormat::RGB_8,Intent::Perceptual).unwrap_or_else(|e|panic!("{name}: reference transform: {e}"));
    let mut expected:Vec<[u8;3]>=input.iter().map(|p|p.map(|v|(v.clamp(0.,1.)*255.) as u8)).collect();
    transform.transform_in_place(&mut expected);
    let resolved=super::super::resolve_viewer_icc_pipeline(&source,destination).unwrap_or_else(|e|panic!("{name}: parse: {e}"));
    let actual=backend.viewer_icc_transform(&input,&resolved.params()).unwrap_or_else(|e|panic!("{name}: device: {e}")).expect("selected GPU ICC must not decline a valid profile");
    assert_eq!(actual.len(),expected.len());
    for (i,(actual,expected)) in actual.iter().zip(expected).enumerate(){
        let actual=actual.map(|v|(v*255.).round() as u8);
        assert_eq!(actual,expected,"{name}: pixel{i} input{:?}",input[i]);
    }
}
#[test]
fn default_lcms_rgb8_real_matrix_profiles_and_rounding(){
    let backend=spektrafilm_gpu::test_support::adapter();
    let fixtures:&[(&str,&[u8])]=&[
        ("P3 parametric v4",include_bytes!("../../../../data/icc/saucecontrol/DisplayP3-v4.icc")),
        ("P3 tabulated v2",include_bytes!("../../../../data/icc/saucecontrol/DisplayP3-v2-micro.icc")),
        ("Adobe v4",include_bytes!("../../../../data/icc/saucecontrol/AdobeCompat-v4.icc")),
        ("ColorMatch v2",include_bytes!("../../../../data/icc/saucecontrol/ColorMatchCompat-v2.icc")),
        ("ProPhoto v2",include_bytes!("../../../../data/icc/saucecontrol/ProPhoto-v2-micro.icc")),
        ("Rec2020 v4",include_bytes!("../../../../data/icc/saucecontrol/Rec2020Compat-v4.icc")),
        ("Lab transfer v4",include_bytes!("../../../../data/icc/ellelstone/ACES-elle-V4-labl.icc")),
    ];
    for (name,bytes) in fixtures{compare(backend.backend(),name,&Profile::new_icc(bytes).unwrap());}
    compare(backend.backend(),"sRGB identity joined optimizer",&Profile::new_srgb());
}
#[test]
fn default_lcms_rgb8_authored_curve_classes(){
    let backend=spektrafilm_gpu::test_support::adapter();
    let curves=[ToneCurve::new(1.8),ToneCurve::new_parametric(5,&[2.4,1./1.055,0.055/1.055,1./12.92,0.04045,0.,0.]).unwrap(),ToneCurve::new_tabulated(&[0,400,1200,7000,18000,35000,65535]),ToneCurve::new_tabulated_float(&[0.,0.01,0.09,0.4,1.])];
    for (i,curve) in curves.iter().enumerate(){
        let mut profile=Profile::new_srgb();profile.set_version(if i%2==0{2.1}else{4.3});
        for tag in [TagSignature::RedTRCTag,TagSignature::GreenTRCTag,TagSignature::BlueTRCTag]{assert!(profile.write_tag(tag,Tag::ToneCurve(curve)));}
        compare(backend.backend(),&format!("authored curve{i}"),&profile);
    }
}
#[test]
fn default_lcms_rgb8_lut_precedence_and_interpolation_classes(){
    use std::ffi::c_void;
    unsafe extern "C"{fn cmsPipelineInsertStage(pipeline:*mut c_void,location:u32,stage:*mut c_void)->i32;fn cmsStageDup(stage:*const c_void)->*mut c_void;}
    let backend=spektrafilm_gpu::test_support::adapter();
    for floats in [false,true]{for lab in [false,true]{
        let mut profile=Profile::new_srgb();profile.set_version(if floats{4.3}else{2.1});
        profile.set_pcs(if lab{lcms2::ColorSpaceSignature::LabData}else{lcms2::ColorSpaceSignature::XYZData});
        let pipeline=lcms2::Pipeline::new(3,3).unwrap();
        let values:Vec<f32>=(0..5).flat_map(|r|(0..5).flat_map(move|g|(0..5).flat_map(move|b|{let [r,g,b]=[r,g,b].map(|v|v as f32/4.);[0.05+0.9*r,0.03+0.94*g,0.02+0.96*b]}))).collect();
        let stage=if floats{lcms2::Stage::new_clut(5,3,3,Some(values.as_slice())).unwrap()}else{let words:Vec<u16>=values.iter().map(|&v|(v*65535.).round() as u16).collect();lcms2::Stage::new_clut(5,3,3,Some(words.as_slice())).unwrap()};
        // Native Stage/Pipeline references are transparent foreign handles.
        let stage=unsafe{cmsStageDup((&*stage as *const lcms2::StageRef).cast())};
        assert!(!stage.is_null());
        assert_ne!(unsafe{cmsPipelineInsertStage((&*pipeline as *const lcms2::PipelineRef).cast_mut().cast(),1,stage)},0);
        let tag=if floats{TagSignature::BToD0Tag}else{TagSignature::BToA0Tag};assert!(profile.write_tag(tag,Tag::Pipeline(&pipeline)));
        compare(backend.backend(),&format!("authored CLUT floats{floats} Lab{lab}"),&profile);
    }}
}
