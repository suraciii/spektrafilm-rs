//! Read-only retained source windows and the native viewer presentation boundary.
use super::*;
use super::paged::ImagePage;
use spektrafilm_backend::{DeviceImage, DisplayTransformGpuParams, ViewerIccGpuParams, ViewerFullSourceGpuParams, ViewerComposeGpuParams, SampleDomain};

impl WgpuBackend {
    pub(super) fn viewer_inspect_gpu(&self, image: &DeviceImage, x: u32, y: u32) -> Result<[f32;3],String> {
        self.check_cancelled()?;
        if x>=image.width() || y>=image.height() { return Err("pixel inspection outside source".into()); }
        let source=self.buffer_payload(image)?;
        let sample=self.viewer_buffer(12,"viewer_source_pixel");
        let mut encoder=self.device.create_command_encoder(&Default::default());
        self.copy_samples_to(&mut encoder,source,(u64::from(y)*u64::from(image.width())+u64::from(x))*3,3,&sample,0)?;
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        let result: Vec<f32>=self.pipe.readback_buffer_cancellable(&sample,12)?;
        Ok([result[0],result[1],result[2]])
    }

    fn viewer_limit(&self)->u64 { let l=self.device.limits(); l.max_buffer_size.min(u64::from(l.max_storage_buffer_binding_size)) }
    fn viewer_buffer(&self,bytes:u64,label:&str)->wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size:bytes.max(4),usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false})
    }
    fn viewer_table(&self,bytes:&[u8])->wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("viewer_profile_table"),contents:if bytes.is_empty(){&[0;4]}else{bytes},usage:wgpu::BufferUsages::STORAGE})
    }
    fn viewer_dispatch(&self,shader:&'static str,words:&[u32],inputs:&[&wgpu::Buffer],count:u32,label:&'static str)->Result<wgpu::Buffer,String> {
        self.check_cancelled()?;
        #[cfg(test)]
        self.pipe.test_operation(label)?;
        let bytes=u64::from(count)*12;
        if count==0 || bytes>self.viewer_limit(){return Err(format!("{label}: minimum viewer working buffer exceeds device limits"));}
        let mut aligned = words.to_vec();
        aligned.resize(words.len().next_multiple_of(4),0);
        let params=self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some(label),contents:bytemuck::cast_slice(&aligned),usage:wgpu::BufferUsages::UNIFORM});
        let out=self.viewer_buffer(bytes,label);
        let mut types=vec![wgpu::BufferBindingType::Uniform];types.extend(inputs.iter().map(|_|wgpu::BufferBindingType::Storage{read_only:true}));types.push(wgpu::BufferBindingType::Storage{read_only:false});
        let pipeline=self.cached_pipeline(shader,&types);
        let mut entries=vec![wgpu::BindGroupEntry{binding:0,resource:params.as_entire_binding()}];
        entries.extend(inputs.iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry{binding:i as u32+1,resource:b.as_entire_binding()}));entries.push(wgpu::BindGroupEntry{binding:inputs.len() as u32+1,resource:out.as_entire_binding()});
        let group=self.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some(label),layout:&pipeline.layout,entries:&entries});
        let mut encoder=self.device.create_command_encoder(&Default::default());{
            let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some(label),timestamp_writes:None});pass.set_pipeline(&pipeline.pipeline);pass.set_bind_group(0,&group,&[]);
            let groups=count.div_ceil(256);let max=self.device.limits().max_compute_workgroups_per_dimension;let x=groups.min(max);let y=groups.div_ceil(x);if y>max{return Err(format!("{label}: dispatch exceeds device limits"));}pass.dispatch_workgroups(x,y,1);
        }
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        self.check_cancelled()?;
        Ok(out)
    }
    fn viewer_window(&self,image:&DeviceImage,window:[u32;4])->Result<wgpu::Buffer,String>{
        self.check_cancelled()?;
        let bytes=u64::from(window[2])*u64::from(window[3])*12;if bytes>self.viewer_limit(){return Err("viewer source window exceeds device limits".into());}
        let buffer=self.viewer_buffer(bytes,"viewer_source_window");let mut encoder=self.device.create_command_encoder(&Default::default());self.copy_window_to(&mut encoder,self.buffer_payload(image)?,window[0],window[1],window[2],window[3],&buffer)?;
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        self.check_cancelled()?;
        Ok(buffer)
    }
    fn viewer_finish(&self,pages:Vec<ImagePage>,width:u32,height:u32)->DeviceImage{
        self.retain_image_pages(pages,width,height,SampleDomain::Encoded,spektrafilm_math::colorspace::resolve("sRGB").expect("registered sRGB"),None)
    }

    pub(super) fn viewer_fit_gpu(&self,image:&DeviceImage,width:u32,height:u32,display:Option<&DisplayTransformGpuParams>,icc:Option<&ViewerIccGpuParams<'_>>)->Result<DeviceImage,String>{
        self.check_cancelled()?;
        self.buffer_payload(image)?;
        let count=width.checked_mul(height).filter(|n|*n>0).ok_or("invalid viewer fit dimensions")?;
        let mut pages=Vec::new();let mut first=0;
        while first<count{
            self.check_cancelled()?;
            let mut n=(count-first).min((self.viewer_limit()/12).min(u64::from(u32::MAX)) as u32);
            let window=loop{
                let (x0,x1,y0,y1)=output_extent(first,n,width);
                let sample=|v:u32,src:u32,out:u32|((v as f32+0.5)*src as f32/out as f32-0.5).clamp(0.0,(src-1)as f32);
                let x=(sample(x0,image.width(),width).floor().max(0.0)as u32).saturating_sub(1);let y=(sample(y0,image.height(),height).floor().max(0.0)as u32).saturating_sub(1);
                let right=(sample(x1,image.width(),width).floor()as u32+3).min(image.width());let bottom=(sample(y1,image.height(),height).floor()as u32+3).min(image.height());
                let w=[x,y,right-x,bottom-y];if u64::from(w[2])*u64::from(w[3])*12<=self.viewer_limit(){break w;}
                if n==1{return Err("viewer bilinear footprint exceeds device limits".into());}n=n.div_ceil(2);
            };
            let source=self.viewer_window(image,window)?;
            let words=[image.width(),image.height(),width,height,window[0],window[1],window[2],first,n];
            let out=self.viewer_dispatch(include_str!("../../../spektrafilm-shaders/wgsl/viewer_resample_retained.wgsl"),&words,&[&source],n,"viewer_fit")?;
            pages.push(ImagePage{first_pixel:first,n_pixels:n,buffer:out});first+=n;
        }
        let mut current=self.viewer_finish(pages,width,height);
        if let Some(d)=display{
            let mut pages=Vec::new();for page in &self.buffer_payload(&current)?.pages{
                self.check_cancelled()?;
                let mut words=vec![page.n_pixels,d.src_cctf,u32::from(d.decode),u32::from(d.clamp)];words.extend(pack_matrix_columns(&d.matrix).iter().flatten().map(|v|v.to_bits()));
                let out=self.viewer_dispatch(concat!(include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),include_str!("../../../spektrafilm-shaders/wgsl/viewer_display_transform.wgsl")),&words,&[&page.buffer],page.n_pixels,"viewer_display")?;
                pages.push(ImagePage{first_pixel:page.first_pixel,n_pixels:page.n_pixels,buffer:out});
            }current=self.viewer_finish(pages,width,height);
        }
        if let Some(c)=icc{current=self.viewer_icc_retained(&current,c)?;}
        self.check_cancelled()?;
        Ok(current)
    }

    pub(super) fn viewer_full_gpu(&self,image:&DeviceImage,p:&ViewerFullSourceGpuParams<'_>)->Result<DeviceImage,String>{
        self.check_cancelled()?;
        self.buffer_payload(image)?;let count=p.tex_width.checked_mul(p.tex_height).filter(|n|*n>0).ok_or("invalid viewer viewport dimensions")?;
        let first_table:Vec<i32>=p.icc_first.iter().flatten().copied().collect();let second_table:Vec<u32>=p.icc_second.iter().flatten().copied().collect();let first_table=self.viewer_table(bytemuck::cast_slice(&first_table));let second_table=self.viewer_table(bytemuck::cast_slice(&second_table));
        let mut pages=Vec::new();let mut first=0;
        while first<count{
            self.check_cancelled()?;
            let mut n=(count-first).min((self.viewer_limit()/12).min(u64::from(u32::MAX))as u32);
            let window=loop{
                let w=source_window(first,n,p.tex_width,p.tex_height,[p.rect_min[0],p.rect_min[1],p.rect_min[0]+p.rect_size[0],p.rect_min[1]+p.rect_size[1]],[p.image_min[0],p.image_min[1],p.image_min[0]+p.image_size[0],p.image_min[1]+p.image_size[1]],image.width(),image.height(),p.interpolation);
                if u64::from(w[2])*u64::from(w[3])*12<=self.viewer_limit(){break w;}
                if n==1{return Err("viewer interpolation footprint exceeds device limits".into());}n=n.div_ceil(2);
            };
            let source=self.viewer_window(image,window)?;let mut words=vec![p.tex_width,p.tex_height,image.width(),image.height()];words.extend(p.rect_min.into_iter().chain(p.rect_size).chain(p.image_min).chain(p.image_size).map(f32::to_bits));words.extend([p.interpolation,p.src_cctf,u32::from(p.decode),u32::from(p.transform)]);words.extend(pack_matrix_columns(&p.matrix).iter().flatten().map(|v|v.to_bits()));words.extend([u32::from(p.icc),u32::from(p.icc_matrix)]);words.extend(p.icc_matrix_fixed14.iter().flatten().map(|v|*v as u32));words.extend(p.icc_offset_fixed14.map(|v|v as u32));words.extend([window[0],window[1],window[2],first,n]);
            let out=self.viewer_dispatch(include_str!("../../../spektrafilm-shaders/wgsl/viewer_full_source_retained.wgsl"),&words,&[&source,&first_table,&second_table],n,"viewer_full_source")?;pages.push(ImagePage{first_pixel:first,n_pixels:n,buffer:out});first+=n;
        }
        self.check_cancelled()?;
        Ok(self.viewer_finish(pages,p.tex_width,p.tex_height))
    }

    pub(super) fn viewer_compose_gpu(&self,image:&DeviceImage,paper:Option<&DeviceImage>,previous:Option<&DeviceImage>,split:Option<&DeviceImage>,p:&ViewerComposeGpuParams)->Result<Vec<[u8;4]>,String>{
        self.viewer_compose_gpu_with_limit(image, paper, previous, split, p, self.viewer_limit())
    }

    fn viewer_compose_gpu_with_limit(&self,image:&DeviceImage,paper:Option<&DeviceImage>,previous:Option<&DeviceImage>,split:Option<&DeviceImage>,p:&ViewerComposeGpuParams,limit:u64)->Result<Vec<[u8;4]>,String>{
        self.check_cancelled()?;
        let count=p.width.checked_mul(p.height).filter(|n|*n>0).ok_or("invalid viewer presentation dimensions")?;let mut result=Vec::with_capacity(count as usize);let mut first=0;
        while first<count{
            self.check_cancelled()?;
            let mut n=(count-first).min((limit/12).min(u64::from(u32::MAX))as u32);
            let windows=loop{
                let mut windows=[[0,0,1,1];4];for (i,source)in [Some(image),paper,previous,split].into_iter().enumerate(){if let Some(source)=source{
                    windows[i]=if i==0&&p.full_source{let(x0,x1,y0,y1)=output_extent(first,n,p.width);[x0,y0,x1-x0+1,y1-y0+1]}else{source_window(first,n,p.width,p.height,p.rect,if i==1{p.paper_rect}else{p.image_rect},source.width(),source.height(),if i==1{4}else{p.interpolation})};
                }}if windows.iter().all(|w|u64::from(w[2])*u64::from(w[3])*12<=limit){break windows;}
                if n==1{return Err("viewer composition footprint exceeds device limits".into());}n=n.div_ceil(2);
            };
            let mut buffers=Vec::new();for(i,source)in [Some(image),paper,previous,split].into_iter().enumerate(){buffers.push(if let Some(source)=source{self.viewer_window(source,windows[i])?}else{self.viewer_buffer(12,"viewer_absent_layer")});}
            let mut words=vec![p.width,p.height,image.width(),image.height()];for size in [paper,previous,split]{words.extend(size.map(|i|[i.width(),i.height()]).unwrap_or([0,0]));}words.extend([p.interpolation,u32::from(p.full_source),u32::from(p.output_layer),0,0,0]);words.extend(p.rect.into_iter().chain(p.border).chain(p.paper_rect).chain(p.image_rect).map(f32::to_bits));words.extend(p.gray.map(f32::to_bits));words.push(p.split.unwrap_or(-1.0).to_bits());words.extend([p.fade.to_bits(),0,0,0]);words.extend(windows.into_iter().flatten());words.extend([first,n,0,0]);
            let references:Vec<_>=buffers.iter().collect();let out=self.viewer_dispatch(include_str!("../../../spektrafilm-shaders/wgsl/viewer_compose_retained.wgsl"),&words,&references,n,"viewer_presentation")?;
            result.extend(self.pipe.readback_buffer_cancellable::<u32>(&out,u64::from(n)*12)?.as_chunks::<3>().0.iter().map(|v|v[0].to_le_bytes()));first+=n;
        }
        self.check_cancelled()?;
        Ok(result)
    }
}

fn output_extent(first:u32,count:u32,width:u32)->(u32,u32,u32,u32){let last=first+count-1;let y0=first/width;let y1=last/width;if y0==y1{(first%width,last%width,y0,y1)}else{(0,width-1,y0,y1)}}
// Both callers provide independent output/source geometry and interpolation bounds.
#[allow(clippy::too_many_arguments)]
fn source_window(first:u32,count:u32,width:u32,height:u32,rect:[f32;4],image:[f32;4],sw:u32,sh:u32,mode:u32)->[u32;4]{
    let(x0,x1,y0,y1)=output_extent(first,count,width);let radius=match mode{0|1=>1,2|3=>2,4=>3,_=>4};
    let coord=|v:u32,extent:u32,lo:f32,hi:f32,ilo:f32,ihi:f32,size:u32|{let p=lo+(v as f32+0.5)*(hi-lo)/extent as f32;((p-ilo)/(ihi-ilo)*size as f32-0.5).floor()as i32};
    let x0=coord(x0,width,rect[0],rect[2],image[0],image[2],sw);let x1=coord(x1,width,rect[0],rect[2],image[0],image[2],sw);let y0=coord(y0,height,rect[1],rect[3],image[1],image[3],sh);let y1=coord(y1,height,rect[1],rect[3],image[1],image[3],sh);
    let left=x0.min(x1).saturating_sub(radius+1).clamp(0,sw as i32-1)as u32;let right=x0.max(x1).saturating_add(radius+2).clamp(1,sw as i32)as u32;let top=y0.min(y1).saturating_sub(radius+1).clamp(0,sh as i32-1)as u32;let bottom=y0.max(y1).saturating_add(radius+2).clamp(1,sh as i32)as u32;[left,top,(right-left).max(1),(bottom-top).max(1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::{ComputeBackend, RetainedImageMeta, Precision};
    use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};

    #[test]
    fn viewer_page_cancellation_stops_readback_and_preserves_healthy_reuse() {
        let gpu = crate::test_support::explicit_adapter();
        let host = ImageBuf::from_data(1, 1, vec![0.5 as _; 3]);
        let image = gpu.retain_image(&host, &RetainedImageMeta {
            width: 1, height: 1, domain: SampleDomain::Encoded, precision: Precision::host(),
            space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(), pixel_pitch_um: None,
        }).unwrap();
        let rect = [0.0, 0.0, 6.0, 1.0];
        let params = ViewerComposeGpuParams {
            width: 6, height: 1, rect, border: rect, paper_rect: rect, image_rect: rect,
            gray: [0.0; 3], interpolation: 0, full_source: false, output_layer: true,
            split: None, fade: 1.0,
        };
        let cancellation = Arc::new(AtomicBool::new(false));
        let readbacks = Arc::new(AtomicUsize::new(0));
        gpu.pipe.test_on_readback({
            let cancellation = cancellation.clone();
            let readbacks = readbacks.clone();
            move || {
                readbacks.fetch_add(1, Ordering::Relaxed);
                cancellation.store(true, Ordering::Relaxed);
            }
        });
        let mut operation = gpu.derived_explicit();
        operation.pipe = operation.pipe.with_cancellation(Some(cancellation.clone()));
        operation.cancellation = Some(cancellation.clone());
        assert_eq!(operation.viewer_compose_gpu_with_limit(&image, None, None, None, &params, 24).unwrap_err(), "operation cancelled");
        assert_eq!(readbacks.load(Ordering::Relaxed), 1, "cancelled viewer must not read the next presentation page");
        gpu.pipe.test_on_readback(|| {});
        let healthy = operation.with_cancellation(None).unwrap();
        let pixels = healthy.viewer_compose_retained(&image, None, None, None, &params).unwrap();
        assert_eq!(pixels, vec![[128, 128, 128, 255]; 6]);
        assert_eq!(gpu.viewer_compose_gpu_with_limit(&image, None, None, None, &params, 24).unwrap(), pixels);
        assert_eq!(image.to_host(gpu.backend()).unwrap().data, host.data);
        gpu.pipe.test_fail_operation("viewer_presentation");
        assert!(healthy.viewer_compose_retained(&image, None, None, None, &params).unwrap_err().contains("injected device resource failure"));
        assert_eq!(gpu.pipe.test_failure_attempts(), 1);
    }
}
