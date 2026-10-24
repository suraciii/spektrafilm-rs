//! Device-owned spectral preparation. Only bounded source rows and coefficients
//! cross the host boundary; generated tables remain on the owning device.
use super::*;
use super::bounded::storage_binding_limit;
use spektrafilm_backend::device_image::DeviceImagePayload;
use spektrafilm_backend::preparation::{PreparedTable, SpectralTcLutParams};
use std::{any::Any, sync::Arc};

pub(crate) struct PreparationTable {
    pub backend_id: u64,
    pub buffer: wgpu::Buffer,
    pub f64_buffer: wgpu::Buffer,
}

impl DeviceImagePayload for PreparationTable {
    fn backend_id(&self) -> u64 { self.backend_id }
    fn as_any(&self) -> &dyn Any { self }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    cells: u32, size: u32, wavelengths: u32, used: u32,
    first: u32, kernel_len: u32, reflectance: u32, window: u32,
    surface: u32, coordinates: u32, pad: [u32; 2],
}

const SHADER: &str = concat!(
    include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("preparation.wgsl"),
);

// A host-side reduction is restricted to the small Gaussian coefficient array.
fn coefficient_sum(xs: &[f64]) -> f64 {
    match xs.len() {
        0 => 0.0, 1 => xs[0], 2 => xs[0] + xs[1],
        n => coefficient_sum(&xs[..n/2]) + coefficient_sum(&xs[n/2..]),
    }
}

impl WgpuBackend {
    /// Explicit diagnostic readback for an external preparation-parity probe.
    /// Production preparation and consumers keep this table device resident.
    pub fn diagnostic_spectral_table(&self, table: &PreparedTable) -> Result<Vec<f64>, String> {
        if table.values.backend_id() != self.backend_id {
            return Err("diagnostic spectral table belongs to another backend".to_owned());
        }
        let payload = table.values.as_any().downcast_ref::<PreparationTable>()
            .ok_or_else(|| "diagnostic table is not a spectral preparation payload".to_owned())?;
        if payload.backend_id != self.backend_id {
            return Err("diagnostic spectral payload belongs to another backend".to_owned());
        }
        Ok(self.pipe.readback_buffer_as(&payload.f64_buffer, payload.f64_buffer.size()))
    }

    pub(super) fn prepare_spectral_table_device(
        &self, params: &SpectralTcLutParams<'_>,
    ) -> Result<PreparedTable, String> {
        if !self.device.features().contains(wgpu::Features::SHADER_F64) {
            return Err("spectral preparation requires device f64 arithmetic".to_owned());
        }
        let size = params.size as usize;
        let nw = params.n_wavelengths as usize;
        let cells = size.checked_mul(size).ok_or("spectral dimensions overflow")?;
        let samples = cells.checked_mul(nw).ok_or("spectral dimensions overflow")?;
        if size < 2 || nw == 0 || nw > 81 || samples != params.spectra.len()
            || cells > u32::MAX as usize / 3 {
            return Err("spectral cube dimensions are not device indexable (2+ grid, 1..81 wavelengths required)".to_owned());
        }
        if !params.window.is_empty() && params.window.len() < nw {
            return Err("spectral window does not cover the wavelength axis".to_owned());
        }
        if !params.surface.is_empty() && (params.surface.len() != 3
            || params.surface.iter().any(|row| row.len() != 15)) {
            return Err("spectral surface requires three rows of 15 coefficients".to_owned());
        }
        if !params.gaussian_sigma.is_finite() || params.gaussian_sigma < 0.0
            || params.reference_tc.iter().chain(params.neutral_tc.iter()).any(|v| !v.is_finite()) {
            return Err("spectral preparation coordinates and Gaussian sigma must be finite".to_owned());
        }
        let limit = storage_binding_limit(self)?;
        let radius_f = (4.0 * params.gaussian_sigma + 0.5).floor();
        if radius_f > (i32::MAX / 4) as f64 {
            return Err("spectral Gaussian support exceeds device indexing".to_owned());
        }
        let radius = radius_f as usize;
        let nk = radius.checked_mul(2).and_then(|v| v.checked_add(1))
            .ok_or("spectral Gaussian support overflows")?;
        let aux_len = nw.checked_add(nk).and_then(|v| v.checked_add(51))
            .ok_or("spectral coefficients overflow")?;
        let scratch_len = nw + 54 + 256;
        let minimum = (nw * 3 * 8).max(aux_len * 8).max(scratch_len * 8)
            .max(nw * 4).max(24) as u64;
        if minimum > limit {
            return Err(format!("spectral minimum coefficient working set needs {minimum} bytes per binding; limit is {limit}"));
        }
        let table_bytes = cells as u64 * 3 * 8;
        if table_bytes > self.device.limits().max_buffer_size {
            return Err("spectral retained table exceeds device buffer capacity".to_owned());
        }
        let rows = ((limit / (nw as u64 * 4).max(24))
            .min(u64::from(u32::MAX) / nw.max(3) as u64)) as usize;
        let used = nw.min(params.sensitivity.len()).min(if params.reflectance {
            params.illuminant.len()
        } else { nw });
        let mut sensitivity = vec![[0.0; 3]; nw];
        sensitivity[..used].copy_from_slice(&params.sensitivity[..used]);
        let window = if params.window.is_empty() { vec![[1.0; 3]; nw] }
            else { params.window[..nw].to_vec() };
        let mut aux = vec![0.0f64; aux_len];
        if params.reflectance { aux[..used].copy_from_slice(&params.illuminant[..used]); }
        let mut kernel = vec![1.0; nk];
        if radius != 0 {
            let sigma2 = params.gaussian_sigma * params.gaussian_sigma;
            for (i, value) in kernel.iter_mut().enumerate() {
                let x = i as f64 - radius as f64;
                *value = (-0.5 / sigma2 * x * x).exp();
            }
            let sum = coefficient_sum(&kernel);
            for value in &mut kernel { *value /= sum; }
        }
        aux[nw..nw+nk].copy_from_slice(&kernel);
        let surface = nw + nk;
        if !params.surface.is_empty() {
            for c in 0..3 { aux[surface+c*15..surface+(c+1)*15].copy_from_slice(&params.surface[c]); }
        }
        let coordinates = surface + 45;
        let coordinate = |tc: [f64; 2], round: bool| -> [f64; 2] {
            tc.map(|v| if round { (v as f32 * (size-1) as f32) as f64 }
                else { v * (size-1) as f64 })
        };
        let reference = coordinate(params.reference_tc, params.reflectance);
        let neutral = coordinate(params.neutral_tc, false);
        aux[coordinates..coordinates+2].copy_from_slice(&reference);
        aux[coordinates+2..coordinates+4].copy_from_slice(&neutral);
        aux[coordinates+4..coordinates+6].copy_from_slice(&params.reference_tc);
        let upload = |label: &'static str, data: &[u8]| self.device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor { label: Some(label), contents: data,
                usage: wgpu::BufferUsages::STORAGE });
        let sens = upload("spectral_sensitivity", bytemuck::cast_slice(&sensitivity));
        let window = upload("spectral_window", bytemuck::cast_slice(&window));
        let aux = upload("spectral_coefficients", bytemuck::cast_slice(&aux));
        let allocate = |label: &'static str, bytes: u64| self.device.create_buffer(
            &wgpu::BufferDescriptor { label: Some(label), size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let scratch = allocate("spectral_neutral_coefficients", scratch_len as u64 * 8);
        let output = allocate("spectral_tc_f32", cells as u64 * 12);
        let output_f64 = allocate("spectral_tc_f64", table_bytes);
        let types = [wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: false }];
        let mut p = Params { cells: 0, size: params.size, wavelengths: params.n_wavelengths,
            used: used as u32, first: 0, kernel_len: nk as u32,
            reflectance: u32::from(params.reflectance), window: u32::from(!params.window.is_empty()),
            surface: u32::from(!params.surface.is_empty()), coordinates: coordinates as u32, pad: [0;2] };
        let dispatch = |entry: &'static str, p: &Params, source: &wgpu::Buffer,
                        result64: &wgpu::Buffer, result32: &wgpu::Buffer, n: u32| {
            let pipeline = self.cached_pipeline_at(SHADER, &types, entry);
            let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("spectral_prepare_params"), contents: bytemuck::bytes_of(p),
                usage: wgpu::BufferUsages::UNIFORM });
            let buffers = [&uniform, source, &sens, &window, &aux, &scratch, result64, result32];
            let entries: Vec<_> = buffers.iter().enumerate().map(|(i,b)| wgpu::BindGroupEntry {
                binding: i as u32, resource: b.as_entire_binding() }).collect();
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(entry), layout: &pipeline.layout, entries: &entries });
            let mut encoder = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(entry), timestamp_writes: None });
                pass.set_pipeline(&pipeline.pipeline);
                pass.set_bind_group(0, &group, &[]);
                dispatch_linear(&mut pass, n);
            }
            self.queue.submit(Some(encoder.finish()));
        };
        let dummy_source = upload("spectral_empty_source", &[0;4]);
        let neutral64 = allocate("spectral_final_neutral", 24);
        let dummy32 = allocate("spectral_empty_output", 12);
        // Upload only the original reflected 4x4 neighborhood, never a widened
        // or blurred cube. The device accumulates it in Mitchell loop order.
        let base = reference.map(|v| (v.clamp(0.0, (size-1) as f64) as usize).min(size-2));
        let reflect = |i: isize| if i < 0 { (-i) as usize }
            else if i >= size as isize { (2*(size as isize-1)-i) as usize } else { i as usize };
        let mut patch = Vec::with_capacity(16*nw);
        for i in 0..4 { for j in 0..4 {
            let cell = reflect(base[0] as isize+i-1)*size + reflect(base[1] as isize+j-1);
            patch.extend_from_slice(&params.spectra[cell*nw..(cell+1)*nw]);
        }}
        for (chunk_index, data) in patch.chunks(rows*nw).enumerate() {
            p.first = (chunk_index*rows) as u32;
            p.cells = (data.len()/nw) as u32;
            let source = upload("spectral_reference_rows", bytemuck::cast_slice(data));
            dispatch("reference", &p, &source, &neutral64, &dummy32, nw as u32);
        }
        dispatch("normalize", &p, &dummy_source, &neutral64, &dummy32, 1);
        for first in (0..cells).step_by(rows) {
            let n = rows.min(cells-first);
            p.first = first as u32;
            p.cells = n as u32;
            let source = upload("spectral_source_rows",
                bytemuck::cast_slice(&params.spectra[first*nw..(first+n)*nw]));
            let chunk64 = allocate("spectral_tc_chunk_f64", n as u64*24);
            let chunk32 = allocate("spectral_tc_chunk_f32", n as u64*12);
            dispatch("contract", &p, &source, &chunk64, &chunk32, n as u32);
            dispatch("gather_neutral", &p, &dummy_source, &chunk64, &chunk32, 48);
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&chunk64, 0, &output_f64, first as u64*24, n as u64*24);
            encoder.copy_buffer_to_buffer(&chunk32, 0, &output, first as u64*12, n as u64*12);
            self.queue.submit(Some(encoder.finish()));
        }
        dispatch("finish_neutral", &p, &dummy_source, &neutral64, &dummy32, 3);
        let neutral: Vec<f64> = self.pipe.readback_buffer_as(&neutral64, 24);
        self.device.context.record_executor(Executor::Gpu, None);
        Ok(PreparedTable { values: Arc::new(PreparationTable {
            backend_id: self.backend_id, buffer: output, f64_buffer: output_f64,
        }), neutral: [neutral[0], neutral[1], neutral[2]] })
    }
}
