use super::*;
use super::paged::RawPlane;
const U:wgpu::BufferBindingType=wgpu::BufferBindingType::Uniform;
const R:wgpu::BufferBindingType=wgpu::BufferBindingType::Storage{read_only:true};
const W:wgpu::BufferBindingType=wgpu::BufferBindingType::Storage{read_only:false};
#[repr(C)]
#[derive(Clone,Copy,bytemuck::Pod,bytemuck::Zeroable)]
struct PageParams{region:[u32;4],control:[u32;4]}
impl WgpuBackend{
    pub(super) fn raw_sensor_paged_gpu(&self,sensor:&RawSensor,params:&RawGpuParams,frame:&RawFrame,frame_buf:&RawBuffer,cfa:&RawBuffer)->Result<RawPlane,String>{
        let plane=self.raw_allocate_plane(sensor.width,sensor.height,"raw_sensor_pages")?;
        let normalization = self.prepare_raw_float_gpu(sensor, params)?;
        let black = &normalization.black;
        let repeat = &normalization.repeat;
        let scale_params = &normalization.scale_params;
        let maximum=RawBuffer::zeros(&self.device,"raw_global_maximum",4);
        let limit=super::super::bounded::storage_binding_limit(self)?;
        let source_capacity=(limit/16).max(1) as usize;
        let channels=sensor.layout.channels() as usize;
        for (chunk,samples) in sensor.samples.chunks(source_capacity*channels).enumerate(){
            let first=(chunk*source_capacity) as u32;let count=(samples.len()/channels) as u32;
            let input=RawBuffer::upload(&self.device,"raw_sensor_input_chunk",bytemuck::cast_slice(samples),wgpu::BufferUsages::STORAGE);
            let mut encoder=self.device.create_command_encoder(&Default::default());
            for page in &plane.pages{
                if sensor.fuji_width==0&&(first+count<=page.first_pixel||first>=page.first_pixel+page.n_pixels){continue;}
                let uniform=RawBuffer::upload(&self.device,"raw_sensor_page_params",bytemuck::bytes_of(&PageParams{
                    region:[page.first_pixel,page.n_pixels,first,0],control:[sensor.filters,u32::from(sensor.zero_is_bad),count,0],
                }),wgpu::BufferUsages::UNIFORM);
                let target=RawBuffer{buffer:page.buffer.clone()};
                self.raw_dispatch(concat!(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_float_helpers.wgsl"),include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_page_sensor.wgsl")),
                    &[frame_buf,&uniform,&input,cfa,black,&target,&maximum,repeat,&normalization.conversion],&[U,U,R,R,R,W,W,R,R],page.n_pixels,"raw_sensor_page").encode(&mut encoder,"raw_sensor_page");
            }
            self.raw_submit(encoder)?;
        }
        let scales=RawBuffer::zeros(&self.device,"raw_global_scales",16);
        if sensor.zero_is_bad{
            self.raw_missing_sites_paged(sensor,frame,frame_buf,&plane,&scales,black,repeat,&maximum,limit)?;
        }
        let totals=RawBuffer::zeros(&self.device,"raw_wb_global_totals",64);
        let one_block=RawBuffer::zeros(&self.device,"raw_wb_one_block",32);
        if params.auto_white_balance{
            for y in (0..sensor.height).step_by(8){for x in (0..sensor.width).step_by(8){
                let width=(sensor.width-x).min(8);let height=(sensor.height-y).min(8);
                let window=RawBuffer::zeros(&self.device,"raw_wb_source_block",u64::from(width)*u64::from(height)*16);
                let mut encoder=self.device.create_command_encoder(&Default::default());
                plane.copy_window_to(&mut encoder,x,y,width,height,&window.buffer)?;
                let mut local=*frame;local.width=width;local.height=height;
                let local_frame=RawBuffer::upload(&self.device,"raw_wb_local_frame",bytemuck::bytes_of(&local),wgpu::BufferUsages::UNIFORM);
                let tile:Vec<u32>=(0..sensor.cfa_rows).flat_map(|r|(0..sensor.cfa_columns).map(move|c|u32::from(sensor.cfa_at(y+r,x+c)))).collect();
                let tile=RawBuffer::upload(&self.device,"raw_wb_local_cfa",bytemuck::cast_slice(&tile),wgpu::BufferUsages::STORAGE);
                self.raw_dispatch(AUTO_WB,&[&local_frame,scale_params,&maximum,&window,&tile,&one_block],&[U,U,R,R,R,W],1,"raw_wb_block").encode(&mut encoder,"raw_wb_block");
                let mode=RawBuffer::upload(&self.device,"raw_wb_accumulate_mode",bytemuck::bytes_of(&[0u32;4]),wgpu::BufferUsages::UNIFORM);
                self.raw_dispatch(WB_TOTALS,&[scale_params,&maximum,&scales,frame_buf,&one_block,&totals,&mode],&[U,R,W,U,R,W,U],1,"raw_wb_totals").encode(&mut encoder,"raw_wb_totals");
                self.raw_submit(encoder)?;
            }}
        }
        let mode=RawBuffer::upload(&self.device,"raw_wb_resolve_mode",bytemuck::bytes_of(&[1u32,0,0,0]),wgpu::BufferUsages::UNIFORM);
        let mut encoder=self.device.create_command_encoder(&Default::default());
        self.raw_dispatch(WB_TOTALS,&[scale_params,&maximum,&scales,frame_buf,&one_block,&totals,&mode],&[U,R,W,U,R,W,U],1,"raw_scale_resolve").encode(&mut encoder,"raw_scale_resolve");
        for page in &plane.pages{
            let uniform=RawBuffer::upload(&self.device,"raw_scale_page_params",bytemuck::bytes_of(&PageParams{region:[page.first_pixel,page.n_pixels,0,0],control:[2,sensor.filters,0,0]}),wgpu::BufferUsages::UNIFORM);
            let target=RawBuffer{buffer:page.buffer.clone()};
            self.raw_dispatch(PROCESS,&[frame_buf,&uniform,&target,&scales,black,repeat,&maximum],&[U,U,W,R,R,R,W],page.n_pixels,"raw_scale_page").encode(&mut encoder,"raw_scale_page");
        }
        self.raw_submit(encoder)?;
        Ok(plane)
    }

    // The bounded repair pass borrows independent shader bindings and its storage limit.
    #[allow(clippy::too_many_arguments)]
    fn raw_missing_sites_paged(&self,sensor:&RawSensor,_frame:&RawFrame,frame_buf:&RawBuffer,plane:&RawPlane,scales:&RawBuffer,black:&RawBuffer,repeat:&RawBuffer,maximum:&RawBuffer,limit:u64)->Result<(),String>{
        let capacity=(limit/(5*16)).saturating_sub(4) as u32;
        if capacity==0{return Err("Device cannot bind the native missing-site 5x5 neighborhood".into());}
        for y in 0..sensor.height{
            for x in (0..sensor.width).step_by(capacity as usize){
                let end=(x+capacity).min(sensor.width);let left=x.saturating_sub(2);let top=y.saturating_sub(2);
                let right=(end+2).min(sensor.width);let bottom=(y+3).min(sensor.height);
                let width=right-left;let height=bottom-top;
                let window=RawBuffer::zeros(&self.device,"raw_missing_window",u64::from(width)*u64::from(height)*16);
                let uniform=RawBuffer::upload(&self.device,"raw_missing_params",bytemuck::bytes_of(&PageParams{region:[y-top,x-left,end-left,width],control:[0,sensor.filters,left,top]}),wgpu::BufferUsages::UNIFORM);
                let mut encoder=self.device.create_command_encoder(&Default::default());
                plane.copy_window_to(&mut encoder,left,top,width,height,&window.buffer)?;
                self.raw_dispatch(PROCESS,&[frame_buf,&uniform,&window,scales,black,repeat,maximum],&[U,U,W,R,R,R,W],1,"raw_missing_sites").encode(&mut encoder,"raw_missing_sites");
                plane.copy_window_from(&mut encoder,x,y,end-x,1,&window.buffer,width,x-left,y-top)?;
                self.raw_submit(encoder)?;
            }
        }
        for page in &plane.pages{
            let uniform=RawBuffer::upload(&self.device,"raw_missing_black_params",bytemuck::bytes_of(&PageParams{region:[page.first_pixel,page.n_pixels,0,0],control:[1,sensor.filters,0,0]}),wgpu::BufferUsages::UNIFORM);
            let target=RawBuffer{buffer:page.buffer.clone()};let mut encoder=self.device.create_command_encoder(&Default::default());
            self.raw_dispatch(PROCESS,&[frame_buf,&uniform,&target,scales,black,repeat,maximum],&[U,U,W,R,R,R,W],page.n_pixels,"raw_missing_black").encode(&mut encoder,"raw_missing_black");self.raw_submit(encoder)?;
        }Ok(())
    }
}
const PROCESS:&str=concat!(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_page_process.wgsl"));
const AUTO_WB:&str=concat!(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_scale_helpers.wgsl"),include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_auto_wb.wgsl"));
const WB_TOTALS:&str=concat!(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_scale_helpers.wgsl"),include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_wb_totals.wgsl"));
