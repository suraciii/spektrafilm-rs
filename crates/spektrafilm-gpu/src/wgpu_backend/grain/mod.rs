use super::*;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ShaderParams {
    dimensions: [u32; 4],
    controls: [f32; 4],
    geometry: [f32; 4],
    flags: [f32; 4],
}

fn shader_params(width: u32, height: u32, params: &crate::GrainV2GpuParams) -> ShaderParams {
    ShaderParams {
        dimensions: [width, height, spektrafilm_math::grain::seeded_phase(params.seed).to_bits(), params.mode],
        controls: [params.amount, params.shadows, params.midtones, params.highlights],
        geometry: [params.raw_scale, params.cluster_size, params.rotation, params.color],
        flags: [params.resolution_factor, params.film_type as f32, params.colored as u32 as f32, params.clustered as u32 as f32],
    }
}

impl WgpuBackend {
    /// Temporary Metal parity diagnostic; emits 32 intermediates per analogue sample.
    pub fn grain_v2_arithmetic_trace(&self, img: &ImageBuf, params: &crate::GrainV2GpuParams) -> Vec<f32> {
        use wgpu::util::DeviceExt;
        let mut source = include_str!("grain_v2.wgsl").split("@compute").next().unwrap().to_owned();
        source.push_str(r#"
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>){
 let sample=gid.x;let count=p.dimensions.x*p.dimensions.y*9u;if(sample>=count){return;}
 let idx=sample/9u;let tap=sample%9u;let width=p.dimensions.x;let height=p.dimensions.y;
 let uv=vec2(f32(idx%width),f32(idx/width))*vec2(1./f32(width-1u),1./f32(height-1u));
 let gsf=max(5200./f32(width),3100./f32(height));let size=floor(vec2(f32(width),f32(height))*gsf);
 let at=vec2<i32>(uv*(size/(1.+(p.geometry.x-1.)/47.*1.5)));
 let coord=vec2<f32>(clamp(at+vec2<i32>(i32(tap%3u)-1,i32(tap/3u)-1),vec2<i32>(0),vec2<i32>(size)-1));
 let timer=bitcast<f32>(p.dimensions.z);let scale=clamp(p.geometry.x,0.5,1.4);let scaled=size*scale;
 let coords=coord/scaled;let mult=scaled/p.geometry.y/scale;let aspect=scaled.x/scaled.y;
 let seed=vec4(coords.x,timer,coords.y,timer);let sn=snoise(timer,seed);
 let angle=sn.x*p.geometry.z;let q=rotate(coords,angle,aspect);let v=vec3(q*mult,0.);
 let texel=1./256./p.geometry.y;let split=frexp(texel*floor(v));let pi=ldexp(split.fract,split.exp)+0.5*texel;
 let tc=pi.xy;let sx=frexp((tc.x+timer)*12.9898);let sy=frexp((tc.y+timer)*78.233);
 let px=ldexp(sx.fract,sx.exp);let py=ldexp(sy.fract,sy.exp);let phase=px+py;let reduced=trig_reduce(phase);
 let sine=grain_sin(phase);let n=sine*43758.5453;let perm=rnm(tc,timer).w;
 let generated=generator(coord,size,0.5,vec3(0.5),false);
 let values=array<f32,32>(coord.x,coord.y,size.x,size.y,coords.x,coords.y,random4(seed),sn.x,sn.w,angle,
 grain_sin(angle),grain_cos(angle),q.x,q.y,v.x,v.y,pi.x,pi.y,pi.z,tc.x,tc.y,px,py,phase,reduced.x,reduced.y,
 sine,n,perm,pn(v,timer,texel),generated.x,generated.y);
 for(var j=0u;j<32u;j++){output_rgb[sample*32u+j]=values[j];}
}
"#);
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_trace_params"), contents: bytemuck::bytes_of(&shader_params(img.width, img.height, params)),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let dummy = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_trace_unused_input"), contents: &[0; 16], usage: wgpu::BufferUsages::STORAGE,
        });
        let count = img.width * img.height * 9;
        let n_bytes = u64::from(count) * 32 * 4;
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grain_trace_output"), size: n_bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grain_trace_readback"), size: n_bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grain_trace"), source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let types = [wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true }, wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: true }];
        let layout_entries: Vec<_> = types.iter().enumerate().map(|(i, &ty)| wgpu::BindGroupLayoutEntry {
            binding: i as u32, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None,
        }).collect();
        let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grain_trace"), entries: &layout_entries,
        });
        let pipeline_layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grain_trace"), bind_group_layouts: &[&layout], push_constant_ranges: &[],
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("grain_trace"), layout: Some(&pipeline_layout), module: &shader,
            entry_point: Some("main"), compilation_options: Default::default(), cache: None,
        });
        let buffers = [&uniform, &dummy, &output, &dummy];
        let entries: Vec<_> = buffers.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry {
            binding: i as u32, resource: b.as_entire_binding(),
        }).collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grain_trace"), layout: &layout, entries: &entries,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);pass.set_bind_group(0, &group, &[]);pass.dispatch_workgroups(count.div_ceil(256), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, n_bytes);
        self.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).unwrap(); });
        self.device.poll(wgpu::Maintain::Wait);rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();let values = bytemuck::cast_slice(&data).to_vec();
        drop(data);readback.unmap();values
    }
    /// Execute Grain V2 on native encoded RGB with no transfer or primaries conversion.
    pub fn grain_v2_gpu(&self, img: &ImageBuf, params: &crate::GrainV2GpuParams) -> ImageBuf {
        use wgpu::util::DeviceExt;
        if img.width == 0 || img.height == 0 { return img.clone(); }
        let input = scalars_to_f32(&img.data);
        let original = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_encoded_input"),
            contents: bytemuck::cast_slice(input.as_ref()),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let state = build_grain_v2_state(&self.device, params, img.width, img.height, &original, None, self);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grain_readback"), size: state.n_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        state.encode_pass(&mut encoder, img.pixel_count() as u32, &readback);
        self.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).unwrap(); });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();
        let output = f32_to_scalars(bytemuck::cast_slice(&data).to_vec());
        drop(data);
        readback.unmap();
        ImageBuf::from_data(img.width, img.height, output)
    }
}

/// Shared standalone/resident preparation, separable resolution filter, and grain.
/// Resident input is encoded on GPU before the same half-storage preparation;
/// the state only encodes commands and never submits or reads image data.
pub(super) struct GrainV2State {
    output: wgpu::Buffer,
    _buffers: Vec<wgpu::Buffer>,
    filter: CachedPipelineRef,
    grain: CachedPipelineRef,
    prepare_group: wgpu::BindGroup,
    filter_groups: Option<[wgpu::BindGroup; 2]>,
    grain_group: wgpu::BindGroup,
    n_bytes: u64,
}

pub(super) fn build_grain_v2_state(
    device: &wgpu::Device,
    params: &crate::GrainV2GpuParams,
    width: u32,
    height: u32,
    input: &wgpu::Buffer,
    output_space: Option<&spektrafilm_math::colorspace::RgbColorSpace>,
    backend: &WgpuBackend,
) -> GrainV2State {
    use spektrafilm_math::colorspace::Cctf;
    use wgpu::util::DeviceExt;
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct FilterParams {
        dimensions: [u32; 4],
        taps: [u32; 4],
        matrix: [[f32; 4]; 3],
    }
    let n_bytes = u64::from(width) * u64::from(height) * 3 * 4;
    let create = |label| device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label), size: n_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let prepared = create("grain_prepared_input");
    let output = create("grain_output");
    let a = params.amount.clamp(0., 1.);
    let gsf = (5200. / width as f32).max(3100. / height as f32);
    let base = (1. + (params.raw_scale - 1.) / 47.) * (1. - params.resolution_factor.clamp(0., 100.) / 100.) / gsf;
    let radius = base * if params.mode != 0 {
        1.87 * (0.12 * a * a + 0.68 * a + 0.2)
    } else {
        (if params.film_type == 1 { 1.2 } else { 1.6 }) * (0.7 * a * a + 0.3 * a + 0.05)
    };
    let optical = params.film_type != 1;
    let weights: Vec<[f32; 2]> = if optical {
        spektrafilm_math::grain::optical_weights(radius).into_iter().map(|w| [w, 0.]).collect()
    } else { spektrafilm_math::grain::fast_blur_weights(radius) };
    let weights_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("grain_resolution_weights"), contents: bytemuck::cast_slice(&weights), usage: wgpu::BufferUsages::STORAGE,
    });
    let storage = wgpu::BufferBindingType::Storage { read_only: true };
    let writable = wgpu::BufferBindingType::Storage { read_only: false };
    let uniform = wgpu::BufferBindingType::Uniform;
    let filter = backend.cached_pipeline(include_str!("film_resolution.wgsl"), &[uniform, storage, storage, writable]);
    let grain = backend.cached_pipeline(include_str!("grain_v2.wgsl"), &[uniform, storage, writable, storage]);
    let bind = |pipeline: &CachedPipelineRef, buffers: &[&wgpu::Buffer]| {
        let entries: Vec<_> = buffers.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry {
            binding: i as u32, resource: b.as_entire_binding(),
        }).collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("grain_pass"), layout: &pipeline.layout, entries: &entries })
    };
    let curve = output_space.map_or(0, |space| match space.cctf {
        Cctf::Linear => 1, Cctf::Acescct => 2, Cctf::Srgb => 3,
        Cctf::ProPhoto => 4, Cctf::Rec2020 => 5, Cctf::AdobeRgb1998 => 6, Cctf::Gamma2_6 => 7,
    });
    let matrix = output_space.map_or([[0.; 4]; 3], |space| space.rgb_to_rgb_identity().map(|row| [row[0] as f32, row[1] as f32, row[2] as f32, 0.]));
    let filter_params = |axis| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("grain_filter_params"),
        contents: bytemuck::bytes_of(&FilterParams {
            dimensions: [width, height, axis, optical as u32],
            taps: [weights.len() as u32, curve, 0, 0], matrix,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let prepare_params = filter_params(2);
    let prepare_group = bind(&filter, &[&prepare_params, input, &weights_buf, &prepared]);
    let mut buffers = vec![prepare_params, weights_buf];
    let filtered = (radius > 0.).then(|| create("grain_filtered_input"));
    let filter_groups = filtered.as_ref().map(|filtered| {
        let horizontal = filter_params(0);
        let vertical = filter_params(1);
        let h = bind(&filter, &[&horizontal, &prepared, &buffers[1], &output]);
        let v = bind(&filter, &[&vertical, &output, &buffers[1], filtered]);
        buffers.extend([horizontal, vertical]);
        [h, v]
    });
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("grain_params"), contents: bytemuck::bytes_of(&shader_params(width, height, params)), usage: wgpu::BufferUsages::UNIFORM,
    });
    let grain_group = bind(&grain, &[&params_buf, &prepared, &output, filtered.as_ref().unwrap_or(&prepared)]);
    buffers.extend([params_buf, prepared]);
    buffers.extend(filtered);
    GrainV2State { output, _buffers: buffers, filter, grain, prepare_group, filter_groups, grain_group, n_bytes }
}

impl GrainV2State {
    pub(super) fn encode_pass(&self, encoder: &mut wgpu::CommandEncoder, n_pixels: u32, output: &wgpu::Buffer) {
        let run = |encoder: &mut wgpu::CommandEncoder, pipeline: &CachedPipelineRef, group: &wgpu::BindGroup| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("grain_pass"), timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, group, &[]);
            dispatch_linear(&mut pass, n_pixels);
        };
        run(encoder, &self.filter, &self.prepare_group);
        if let Some([horizontal, vertical]) = &self.filter_groups {
            run(encoder, &self.filter, horizontal);
            run(encoder, &self.filter, vertical);
        }
        run(encoder, &self.grain, &self.grain_group);
        encoder.copy_buffer_to_buffer(&self.output, 0, output, 0, self.n_bytes);
    }
}
