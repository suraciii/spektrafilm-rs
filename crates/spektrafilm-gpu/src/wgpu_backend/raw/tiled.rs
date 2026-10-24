//! The production RAW route uses bounded integer image pages. Reductions run
//! over the complete source before demosaic windows or output gathers begin.
use super::*;
impl WgpuBackend {
    pub(super) fn raw_process_paged(&self,sensor:&RawSensor,params:&RawGpuParams)->Result<spektrafilm_backend::DeviceImage,String>{
        self.check_cancelled()?;
        if sensor.width==0||sensor.height==0||u64::from(sensor.width)*u64::from(sensor.height)>u64::from(u32::MAX){return Err("RAW dimensions exceed device indexing".into());}
        if sensor.cfa_rows==0||sensor.cfa_columns==0||sensor.cfa.len()!=sensor.cfa_rows as usize*sensor.cfa_columns as usize||sensor.cfa.iter().any(|&c|c>3){return Err("Invalid native RAW CFA metadata".into());}
        if sensor.colors==0||sensor.colors>4||sensor.black_map.len()!=sensor.black_map_rows as usize*sensor.black_map_columns as usize{return Err("Invalid native RAW colour/black metadata".into());}
        let sites=if sensor.fuji_width!=0{u64::from(sensor.copied_width)*u64::from(sensor.copied_height)}else{u64::from(sensor.width)*u64::from(sensor.height)};
        if sensor.samples.len() as u64!=sites*u64::from(sensor.layout.channels()){return Err("RAW sample allocation disagrees with sensor layout".into());}
        let(out_width,out_height)=sensor.output_dimensions()?;
        let frame=RawFrame{
            width:sensor.width,height:sensor.height,cfa_rows:sensor.cfa_rows,cfa_columns:sensor.cfa_columns,
            black_rows:sensor.black_map_rows,black_columns:sensor.black_map_columns,
            green_merge:u32::from(params.green_merge),has_black_map:u32::from(!sensor.black_map.is_empty()),
            merge_row:params.green_merge_row,merge_column:params.green_merge_column,
            source_width:if sensor.fuji_width!=0{sensor.copied_width}else{sensor.width},
            source_height:if sensor.fuji_width!=0{sensor.copied_height}else{sensor.height},
            channels:sensor.layout.channels(),fuji_width:sensor.fuji_width,fuji_layout:u32::from(sensor.fuji_layout),colors:sensor.colors,
            subtract_black_map:u32::from(sensor.layout.channels()>1||sensor.zero_is_bad),
            mix_green:u32::from(sensor.mix_green&&!params.green_merge)|(u32::from(sensor.signed_foveon)<<1),
        };
        let frame_buf=RawBuffer::upload(&self.device,"raw_paged_frame",bytemuck::bytes_of(&frame),wgpu::BufferUsages::UNIFORM);
        let cfa:Vec<u32>=sensor.cfa.iter().map(|&c|u32::from(c)).collect();
        let cfa=RawBuffer::upload(&self.device,"raw_paged_cfa",bytemuck::cast_slice(&cfa),wgpu::BufferUsages::STORAGE);
        let plane=self.raw_sensor_paged_gpu(sensor,params,&frame,&frame_buf,&cfa)?;
        let xyz=RawBuffer::upload(&self.device,"raw_paged_xyz",bytemuck::cast_slice(&sensor.xyz_camera),wgpu::BufferUsages::STORAGE);
        let cbrt=RawBuffer::zeros(&self.device,"raw_paged_cbrt",65536*4);
        let mut encoder=self.device.create_command_encoder(&Default::default());
        self.raw_dispatch(CBRT_SHADER,&[&cbrt],&[wgpu::BufferBindingType::Storage{read_only:false}],65536,"raw_cbrt").encode(&mut encoder,"raw_cbrt");
        self.raw_submit(encoder)?;
        let plane=self.raw_demosaic_paged_gpu(sensor,params,plane,&cfa,&xyz,&cbrt)?;
        let pages=self.raw_output_paged_gpu(sensor,params,&plane)?;
        self.check_cancelled()?;
        Ok(self.retain_image_pages(pages,out_width,out_height,spektrafilm_backend::SampleDomain::Linear,&spektrafilm_math::colorspace::ACES2065_1,None))
    }
}
