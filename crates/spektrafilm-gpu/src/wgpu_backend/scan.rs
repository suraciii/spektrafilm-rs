//! Resident scanner acquisition and bounded inverse scan.
use super::WgpuBackend;
use super::chain_chunks::PointwisePass;
use super::grain::{RasterStorage, raster::RasterCommands, v1::blur::{GrainBlurState, build_grain_blur_paged}};
use spektrafilm_backend::{ConvertSolverGpuParams, ScannerDensityGpuParams};

pub(super) struct ConvertInverseState { pass: PointwisePass }
impl ConvertInverseState {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.pass.submit(backend)?;
        backend.check_cancelled()
    }
}
pub(super) struct ScanDensityState {
    pass: PointwisePass,
    optics: Option<(PointwisePass, GrainBlurState, RasterCommands, PointwisePass)>,
    status: wgpu::Buffer,
}
impl ScanDensityState {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        if let Some((transmit, blur, restore, density)) = &self.optics {
            transmit.submit(backend)?;
            blur.submit(backend)?;
            restore.submit(backend)?;
            density.submit(backend)?;
        }
        self.pass.submit(backend)?;
        backend.check_cancelled()
    }
    pub(super) fn check(&self, backend: &WgpuBackend) -> Result<(), String> {
        let code = backend.pipe.readback_buffer_as::<u32>(&self.status, 4)[0];
        if code != 0 { return Err("density inverse exceeds finite relative exposure range or output is not representable in the selected precision".into()); }
        Ok(())
    }
}
impl WgpuBackend {
    fn scan_pass(&self, shader: &'static str, types: &[wgpu::BufferBindingType], buffers: &[&dyn RasterStorage], images: &[(usize,bool)], count:u32) -> Result<PointwisePass,String> {
        let pipeline = self.cached_pipeline(shader, types);
        self.chain_pointwise(pipeline,buffers,images,count,false,"scan_device")
    }
    pub(super) fn build_scan_direct_state(&self,p:&spektrafilm_backend::FilmChainParams<'_>,
        source:&dyn RasterStorage,output:&dyn RasterStorage,width:u32,height:u32)->Result<PointwisePass,String> {
        if !self.device.features().contains(wgpu::Features::SHADER_F64) { return Err("direct scanner acquisition requires native shader f64".into()); }
        let (density,base)=if p.scan_film {(p.film_channel_density,p.film_base_density)}else{(p.print_channel_density,p.print_base_density)};
        let count=density.len().min(p.scan_illuminant.len()).min(spektrafilm_math::spectral::N_WAVELENGTHS);
        let has_base=base.len()>=count && !base.is_empty();
        let active:Vec<_>=(0..count).filter(|&i|!density[i].iter().any(|v|v.is_nan()) && (!has_base || !base[i].is_nan()) && !p.scan_illuminant[i].is_nan()).collect();
        if active.is_empty() { return Err("direct scanner acquisition has no contributing wavelengths".into()); }
        #[repr(C)]
        #[derive(Clone,Copy,bytemuck::Pod,bytemuck::Zeroable)]
        struct Params {header:[u32;4],normalization:[f64;2],cat:[[f64;4];3],rgb:[[f64;4];3]}
        let row=|r:[f64;3]|[r[0],r[1],r[2],0.0];
        let params=Params{header:[width*height,active.len()as u32,0,0],normalization:[p.scan_normalization,0.0],cat:p.scan_cat.map(row),rgb:p.scan_base_xyz_to_rgb.map(row)};
        let table=|name,bytes:&[u8]|self.pipe.static_buffer(name,bytes,wgpu::BufferUsages::STORAGE);
        let dye:Vec<_>=active.iter().map(|&i|density[i]).collect();
        let base:Vec<_>=active.iter().map(|&i|if has_base{base[i]}else{0.0}).collect();
        let lamp:Vec<_>=active.iter().map(|&i|p.scan_illuminant[i]).collect();
        let cmfs:Vec<_>=active.iter().map(|&i|[spektrafilm_math::spectral::CMF_X_F64[i],spektrafilm_math::spectral::CMF_Y_F64[i],spektrafilm_math::spectral::CMF_Z_F64[i]]).collect();
        let uniform=self.pipe.static_buffer("direct_scan_params",bytemuck::bytes_of(&params),wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_SRC);
        let dye=table("direct_scan_dyes",bytemuck::cast_slice(&dye));
        let base=table("direct_scan_base",bytemuck::cast_slice(&base));
        let lamp=table("direct_scan_lamp",bytemuck::cast_slice(&lamp));
        let cmfs=table("direct_scan_cmfs",bytemuck::cast_slice(&cmfs));
        let buffers:[&dyn RasterStorage;7]=[&uniform,source,&dye,&base,&lamp,&cmfs,output];
        use wgpu::BufferBindingType::{Uniform as U,Storage as S};
        self.scan_pass(concat!(include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),include_str!("../../../spektrafilm-shaders/wgsl/scan/direct_spectral.wgsl")),
            &[U,S{read_only:true},S{read_only:true},S{read_only:true},S{read_only:true},S{read_only:true},S{read_only:false}],&buffers,&[(1,false),(6,true)],width*height)
    }
    pub(super) fn build_convert_inverse_state(&self, p: &ConvertSolverGpuParams<'_>, src: &(impl RasterStorage + ?Sized), dst: &(impl RasterStorage + ?Sized), n: u32) -> Result<ConvertInverseState, String> {
        if !self.device.features().contains(wgpu::Features::SHADER_F64) { return Err("convert-film inverse scan requires native shader f64".into()); }
        let wl = p.n_wl as usize;
        if wl == 0 || p.dye.len() < wl || p.illuminant.len() < wl || p.cmfs.len() < wl || !p.normalization.is_finite() || p.normalization == 0.0 {
            return Err("convert-film inverse scan has invalid spectral coefficients".into());
        }
        // Missing spectral rows produce nonfinite light for every bounded CMY
        // and the CPU excludes them before integration. Compact those rows
        // once; uploading NaNs relies on driver fast-math NaN comparisons and
        // can contaminate the normal equations before their final clamp.
        let active: Vec<_> = (0..wl).filter(|&i| {
            !p.base.get(i).copied().unwrap_or(0.0).is_nan()
                && !p.dye[i].iter().any(|value| value.is_nan())
                && p.illuminant[i].is_finite()
        }).collect();
        if active.is_empty() { return Err("convert-film inverse scan has no contributing spectral rows".into()); }
        let dyes: Vec<_> = active.iter().map(|&i|p.dye[i]).collect();
        let bases: Vec<_> = active.iter().map(|&i|p.base.get(i).copied().unwrap_or(0.0)).collect();
        let lamps: Vec<_> = active.iter().map(|&i|p.illuminant[i]).collect();
        let matching: Vec<_> = active.iter().map(|&i|p.cmfs[i]).collect();
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct Uniform {
            header: [u32; 4], gain: f64, normalization: f64,
            seed_log_rgb: [f64; 4], seed_inverse: [[f64; 4]; 3],
            xyz_to_rgb: [[f64; 4]; 3], calibration: [[f64; 4]; 3], cmy_max: [f64; 4],
        }
        let row = |v: [f64; 3]| [v[0], v[1], v[2], 0.0];
        let uniform = Uniform {
            header: [n, active.len() as u32, 8, if p.decode_input { p.cctf_kind } else { 0 }],
            gain: p.gain, normalization: p.normalization,
            seed_log_rgb: row(p.seed_log_rgb), seed_inverse: p.seed_inverse.map(row),
            xyz_to_rgb: p.xyz_to_rgb.map(row), calibration: p.calibration.map(row), cmy_max: row(p.cmy_max),
        };
        let table = |label, bytes: &[u8]| self.pipe.static_buffer(label, bytes, wgpu::BufferUsages::STORAGE);
        let uniform=self.pipe.static_buffer("convert_params", bytemuck::bytes_of(&uniform), wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_SRC);
        let dye=table("convert_dye", bytemuck::cast_slice(&dyes));
        let base=table("convert_base", bytemuck::cast_slice(&bases));
        let lamp=table("convert_lamp", bytemuck::cast_slice(&lamps));
        let cmfs=table("convert_cmfs", bytemuck::cast_slice(&matching));
        let source=src.image_pages();
        let destination=dst.image_pages();
        let buffers:[&dyn RasterStorage;7]=[&uniform,&source,&dye,&base,&lamp,&cmfs,&destination];
        use wgpu::BufferBindingType::{Uniform as U, Storage as S};
        Ok(ConvertInverseState { pass: self.scan_pass(concat!(
            include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
            include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
            include_str!("../../../spektrafilm-shaders/wgsl/scan/convert_inverse_solver.wgsl")),
            &[U, S {read_only:true}, S {read_only:true}, S {read_only:true}, S {read_only:true}, S {read_only:true}, S {read_only:false}], &buffers, &[(1,false),(6,true)],n)? })
    }
    pub(super) fn build_scan_density_state(&self, p: &ScannerDensityGpuParams, src: &(impl RasterStorage + ?Sized), dst: &(impl RasterStorage + ?Sized), width: u32, height: u32) -> Result<ScanDensityState, String> {
        if !self.device.features().contains(wgpu::Features::SHADER_F64) { return Err("negative-to-positive acquisition requires native shader f64".into()); }
        if p.counts.iter().enumerate().any(|(c, &n)| n < 2 || p.offsets[c] as usize + n as usize > p.density.len()) || p.exposure.len() != p.density.len() {
            return Err("negative-to-positive acquisition has invalid calibrated axes".into());
        }
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct Uniform {
            header: [u32;4], offsets: [u32;4], counts: [u32;4], pad: [u32;4],
            contrast: [f64;4], dmax: [f64;4], brightness: [f64;4], denominator: [f64;4], camera: [f64;4],
        }
        let row = |v: [f64;3]| [v[0],v[1],v[2],0.0];
        let uniform = Uniform {
            header: [width*height,0,0,0], offsets: [p.offsets[0],p.offsets[1],p.offsets[2],0],
            counts: [p.counts[0],p.counts[1],p.counts[2],0], pad: [0;4],
            contrast: row(p.contrast), dmax: row(p.dmax), brightness: row(p.brightness_ev), denominator: row(p.denominator), camera: [p.camera_ev,0.0,0.0,0.0],
        };
        let table = |label, bytes: &[u8]| self.pipe.static_buffer(label, bytes, wgpu::BufferUsages::STORAGE);
        use wgpu::BufferBindingType::{Uniform as U, Storage as S};
        let status = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scan_density_status"), contents: bytemuck::bytes_of(&0u32),
            usage: wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,
        });
        let source=src.image_pages();
        let destination=dst.image_pages();
        let pass = self.scan_pass(concat!(include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"), include_str!("../../../spektrafilm-shaders/wgsl/scan/scanner_density.wgsl")),
            &[U,S{read_only:true},S{read_only:true},S{read_only:true},S{read_only:false},S{read_only:false}], &[
                &self.pipe.static_buffer("density_params", bytemuck::bytes_of(&uniform), wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_SRC),
                &source, &table("density_axes",bytemuck::cast_slice(&p.density)), &table("exposure_axes",bytemuck::cast_slice(&p.exposure)), &destination, &status,
            ], &[(1,false),(4,true)], width*height)?;
        let optics = if p.lens_blur > 0.0 {
            let make = |inverse: u32| self.scan_pass(concat!(include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"), include_str!("../../../spektrafilm-shaders/wgsl/scan/transmission.wgsl")),
                &[U,S{read_only:false}], &[
                    &self.pipe.static_buffer("transmission_params", bytemuck::cast_slice(&[width*height,inverse,0,0]), wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_SRC), &source,
                ], &[(1,true)], width*height);
            let blur=build_grain_blur_paged(self,width,height,&source,&destination,[f64::from(p.lens_blur);3],0,0)?;
            let mut restore=RasterCommands::default();
            restore.copy(&destination,0,&source,0,u64::from(width)*u64::from(height)*12);
            Some((make(0)?,blur,restore,make(1)?))
        } else { None };
        Ok(ScanDensityState { pass, optics, status })
    }
}
