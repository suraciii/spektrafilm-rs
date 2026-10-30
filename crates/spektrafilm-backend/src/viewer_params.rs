//! Bounded native-viewer presentation geometry; image samples stay in carriers.
/// Endpoint warnings applied only to photographic viewer pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewerClipping {
    #[default]
    Off,
    Overlay {
        shadows: bool,
        highlights: bool,
    },
    Shadows,
    Highlights,
}

impl ViewerClipping {
    pub fn active(self) -> bool {
        !matches!(
            self,
            Self::Off
                | Self::Overlay {
                    shadows: false,
                    highlights: false,
                }
        )
    }
}

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
    pub clipping: ViewerClipping,
}
