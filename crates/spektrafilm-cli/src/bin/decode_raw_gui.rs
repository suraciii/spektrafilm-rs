//! Decode the shared GUI/export RAW boundary to linear ACES float TIFF.
use std::{fs::File,io::BufWriter,path::PathBuf};
use anyhow::Result;
use clap::{Parser,ValueEnum};
use spektrafilm_raw::{RawOptions,WhiteBalance};
use tiff::encoder::{TiffEncoder,colortype::RGB32Float};

#[derive(Clone,Copy,ValueEnum)]
enum Wb { AsShot,Daylight,Tungsten,Custom }
#[derive(Parser)]
struct Args {
    input:PathBuf,
    output:PathBuf,
    #[arg(long, value_enum, default_value="as-shot")]
    raw_white_balance:Wb,
    #[arg(long)] raw_temperature:Option<f64>,
    #[arg(long)] raw_tint:Option<f64>,
    #[arg(long)] lens_correction:bool,
}
fn main()->Result<()> {
    let args=Args::parse();
    let options=RawOptions {
        white_balance:match args.raw_white_balance { Wb::AsShot=>WhiteBalance::AsShot,Wb::Daylight=>WhiteBalance::Daylight,Wb::Tungsten=>WhiteBalance::Tungsten,Wb::Custom=>WhiteBalance::Custom },
        temperature:args.raw_temperature,tint:args.raw_tint,lens_correction:args.lens_correction,
    };
    let result=spektrafilm_raw::load(&args.input,&options)?;
    let mut encoder=TiffEncoder::new(BufWriter::new(File::create(&args.output)?))?;
    let pixels:Vec<f32>=result.image.data.iter().map(|&v|v as f32).collect();
    encoder.write_image::<RGB32Float>(result.image.width,result.image.height,&pixels)?;
    eprintln!("LibRaw {}: {}x{} linear ACES2065-1; {}",spektrafilm_raw::decoder_version(),result.image.width,result.image.height,result.lens_info);
    Ok(())
}
