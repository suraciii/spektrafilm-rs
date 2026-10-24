//! ICC authored-data upload, device numerical preparation, and resident evaluation.
use super::*;
use spektrafilm_backend::{DeviceImage, SampleDomain, ViewerIccGpuParams};

const ICC_SHADER: &str = concat!(
    include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("viewer_icc_eval.wgsl"),
    include_str!("viewer_icc_prepare.wgsl"),
    include_str!("viewer_icc_apply.wgsl"),
);
impl WgpuBackend {
    pub(super) fn viewer_icc_gpu(&self,image:&DeviceImage,p:&ViewerIccGpuParams<'_>)->Result<DeviceImage,String>{
        self.check_cancelled()?;
        if !self.device.features().contains(wgpu::Features::SHADER_F64){return Err("viewer ICC requires device f64 coefficient evaluation".into());}
        if p.words.len()<2{return Err("viewer ICC pipeline directory is missing".into());}
        let count=p.words[0] as usize;
        let first=p.words[1] as usize;
        let end=first.checked_add(count.checked_mul(4).ok_or("ICC stage count overflow")?).ok_or("ICC directory overflow")?;
        if end>p.words.len(){return Err("viewer ICC stage directory is truncated".into());}
        let mut curves=0u32;
        for s in p.words[first..end].as_chunks::<4>().0{
            if s[0]==1{curves=curves.checked_add(s[2]).ok_or("ICC curve count overflow")?;}
            // BPC darkest-colorant auxiliary stages live after the main directory.
            if s[0]==9{
                let at=s[3] as usize;let aux=p.words.get(at..at+3).ok_or("ICC BPC descriptor is truncated")?;
                let begin=first+aux[1] as usize*4;let end=begin+aux[2] as usize*4;
                for a in p.words.get(begin..end).ok_or("ICC BPC stage directory is truncated")?.as_chunks::<4>().0{if a[0]==1{curves=curves.checked_add(a[2]).ok_or("ICC curve count overflow")?;}}
            }
        }
        let scratch_bytes=(262144u64+8192*u64::from(curves))*4;
        let source=self.buffer_payload(image)?;
        let limit=self.device.limits();
        let binding_limit=super::bounded::storage_binding_limit(self)?;
        let word_bytes=std::mem::size_of_val(p.words) as u64;let value_bytes=std::mem::size_of_val(p.values) as u64;
        if [scratch_bytes,word_bytes,value_bytes].into_iter().any(|n|n>binding_limit){return Err("viewer ICC profile working set exceeds device binding limits".into());}
        let words=self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("icc_authored_metadata"),contents:bytemuck::cast_slice(p.words),usage:wgpu::BufferUsages::STORAGE});
        let values=self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("icc_authored_coefficients"),contents:if p.values.is_empty(){&[0;8]}else{bytemuck::cast_slice(p.values)},usage:wgpu::BufferUsages::STORAGE});
        let scratch=self.device.create_buffer(&wgpu::BufferDescriptor{label:Some("icc_device_optimizer"),size:scratch_bytes,usage:wgpu::BufferUsages::STORAGE,mapped_at_creation:false});
        let bpc=self.device.create_buffer(&wgpu::BufferDescriptor{label:Some("icc_bpc_coefficients"),size:48,usage:wgpu::BufferUsages::STORAGE,mapped_at_creation:false});
        let output:Vec<_>=source.pages.iter().map(|page|super::paged::ImagePage{
            first_pixel:page.first_pixel,n_pixels:page.n_pixels,
            buffer:self.device.create_buffer(&wgpu::BufferDescriptor{label:Some("icc_disposable_view"),size:u64::from(page.n_pixels)*12,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false}),
        }).collect();
        let first_source=source.pages.first().ok_or("ICC source has no pages")?;
        let first_output=output.first().ok_or("ICC output has no pages")?;
        let types=[wgpu::BufferBindingType::Uniform,wgpu::BufferBindingType::Storage{read_only:true},wgpu::BufferBindingType::Storage{read_only:true},wgpu::BufferBindingType::Storage{read_only:true},wgpu::BufferBindingType::Storage{read_only:false},wgpu::BufferBindingType::Storage{read_only:false},wgpu::BufferBindingType::Storage{read_only:false}];
        let prep=self.pipeline_cache.get_or_compile_at(&self.device,ICC_SHADER,&types,"prepare_main").0;
        let apply=self.pipeline_cache.get_or_compile_at(&self.device,ICC_SHADER,&types,"apply_main").0;
        // Every phase is a separate pass: a queue-ordered device barrier makes
        // generated curves visible to inversion and the generated lattice.
        let phases=[(6,curves.div_ceil(64).max(1)),(7,1),(0,1),(1,257),(2,1),(3,64),(4,562),(5,1)];
        let jobs=phases.into_iter().map(|(mode,groups)|(mode,groups,first_source.n_pixels,&first_source.buffer,&first_output.buffer))
            .chain(source.pages.iter().zip(&output).map(|(input,output)|(8,input.n_pixels.div_ceil(64),input.n_pixels,&input.buffer,&output.buffer)));
        for (mode,groups,n_pixels,input,output) in jobs {
            self.check_cancelled()?;
            let mut encoder=self.device.create_command_encoder(&Default::default());
            let uniform=[n_pixels,mode,count as u32,curves];
            let uniform=self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("icc_dispatch"),contents:bytemuck::cast_slice(&uniform),usage:wgpu::BufferUsages::UNIFORM});
            let pipeline=if mode==8{&apply}else{&prep};
            let buffers=[&uniform,input,&words,&values,&scratch,output,&bpc];
            let entries:Vec<_>=buffers.iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry{binding:i as u32,resource:b.as_entire_binding()}).collect();
            let group=self.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("icc_dispatch"),layout:&pipeline.layout,entries:&entries});
            let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some("viewer_icc_device_preparation"),timestamp_writes:None});
            pass.set_pipeline(&pipeline.pipeline);pass.set_bind_group(0,&group,&[]);
            let x=groups.min(limit.max_compute_workgroups_per_dimension).max(1);let y=groups.div_ceil(x);
            if y>limit.max_compute_workgroups_per_dimension{return Err("viewer ICC dispatch exceeds device limits".into());}
            pass.dispatch_workgroups(x,y,1);
            drop(pass);
            self.check_cancelled()?;
            self.queue.submit(Some(encoder.finish()));
        }
        self.check_cancelled()?;
        Ok(self.retain_image_pages(output,image.width(),image.height(),SampleDomain::Encoded,image.space(),image.pixel_pitch_um()))
    }
    pub(super) fn viewer_icc_samples_gpu(&self,rgb:&[[f32;3]],p:&ViewerIccGpuParams<'_>)->Result<Vec<[f32;3]>,String>{
        if rgb.is_empty(){return Ok(Vec::new());}
        let width=u32::try_from(rgb.len()).map_err(|_|"ICC sample count exceeds device index range")?;
        let image=ImageBuf::from_data(width,1,f32_to_scalars(rgb.iter().flatten().copied().collect()));
        let source=self.upload_retained(&image,&spektrafilm_backend::RetainedImageMeta{width,height:1,domain:SampleDomain::Encoded,precision:spektrafilm_backend::Precision::F32,space:spektrafilm_math::colorspace::resolve("sRGB")?,pixel_pitch_um:None})?;
        let transformed=self.viewer_icc_gpu(&source,p)?;
        let samples=self.materialize_buffer(&transformed)?;
        Ok(samples.data.as_chunks::<3>().0.iter().map(|p|[p[0] as f32,p[1] as f32,p[2] as f32]).collect())
    }
}
