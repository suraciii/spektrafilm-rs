//! Bounded native-viewer presentation geometry; image samples stay in carriers.
#[derive(Clone, Copy, Debug)]
pub struct ViewerComposeGpuParams {
    pub width: u32,
    pub height: u32,
    pub rect: [f32; 4],
    pub border: [f32; 4],
    pub paper_rect: [f32; 4],
    pub image_rect: [f32; 4],
    pub gray: [f32; 3],
    pub interpolation: u32,
    pub full_source: bool,
    pub output_layer: bool,
    pub fade: f32,
}
