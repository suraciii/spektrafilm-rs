// Viewer presentation. Immutable pipeline floats are shared for inspection;
// a persistent worker prepares disposable fit data and bounded viewport pixels.
use serde_json::Value;

mod colortransform;
mod decorations;
mod raster;
mod viewer;
mod worker;

pub use colortransform::{input_display_raster, output_display_raster};
<<<<<<< HEAD
=======
#[cfg(test)]
pub use decorations::virtual_paper_back;
#[cfg(test)]
pub(crate) use decorations::virtual_paper_back_without_watermark;
>>>>>>> origin/main-0.3.4
pub use raster::{DisplayRaster, Interpolation};
pub use viewer::Viewer;
#[cfg(test)]
pub(crate) use viewer::ViewerObservation;

pub const ANIMATION_MAX_PIXELS: usize = 1_500_000;
pub const INTERPOLATIONS: [&str; 7] = [
    "nearest", "linear", "cubic", "spline16", "spline36", "lanczos", "blackman",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewLayer {
    Input,
    #[default]
    Output,
    PaperBack,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DisplaySettings {
    pub layer: ViewLayer,
    pub interpolation: Interpolation,
    pub gray_18_canvas: bool,
    /// Padding on each side, as a fraction of the INPUT long edge.
    pub white_padding: f32,
    pub reveal: bool,
    pub crossfade: bool,
    pub use_display_transform: bool,
}
impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            layer: ViewLayer::Output,
            interpolation: Interpolation::Spline36,
            gray_18_canvas: true,
            white_padding: 0.03,
            reveal: true,
            crossfade: true,
            use_display_transform: true,
        }
    }
}
impl DisplaySettings {
    pub fn from_json(v: &Value) -> Self {
        let mut s = Self::default();
        s.gray_18_canvas = v["gray_18_canvas"].as_bool().unwrap_or(s.gray_18_canvas);
        s.white_padding = v["white_padding"]
            .as_f64()
            .filter(|x| x.is_finite())
            .unwrap_or(0.03)
            .clamp(0.0, 1.0) as f32;
        s.interpolation =
            Interpolation::parse(v["output_interpolation"].as_str().unwrap_or("spline36"));
        s.use_display_transform = v["use_display_transform"]
            .as_bool()
            .unwrap_or(s.use_display_transform);
        s.reveal = v["reveal"].as_bool().unwrap_or(true);
        s.crossfade = v["crossfade"].as_bool().unwrap_or(true);
        s.layer = match v["viewer_layer"].as_str() {
            Some("input") => ViewLayer::Input,
            Some("paper_back") => ViewLayer::PaperBack,
            _ => ViewLayer::Output,
        };
        s
    }
    pub fn to_json(&self) -> Value {
        serde_json::json!({"viewer_layer":match self.layer { ViewLayer::Input=>"input",ViewLayer::Output=>"output",ViewLayer::PaperBack=>"paper_back" },"output_interpolation":self.interpolation.name(),"gray_18_canvas":self.gray_18_canvas,"white_padding":self.white_padding,"reveal":self.reveal,"crossfade":self.crossfade,"use_display_transform":self.use_display_transform})
    }
}
