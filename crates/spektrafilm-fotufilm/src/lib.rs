//! Complete pinned Fotufilm simulation, including authoritative preparation and native optics.
mod native;
mod packet;
mod producer;

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const SOURCE_REVISION: &str = "59d43471dbef45c85ae442e507a291fdebfb2ec5";
pub const RECORD_DENSITY_SPACE: &str = "Fotufilm record density";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Request {
    pub stock: String,
    pub controls: BTreeMap<String, serde_json::Value>,
    pub native: BTreeMap<String, serde_json::Value>,
}
impl Default for Request {
    fn default() -> Self {
        Self {
            stock: "portra400".into(),
            controls: BTreeMap::new(),
            native: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StockInfo {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Full,
    Negative,
    Print,
    Texture,
}
impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Negative => "negative",
            Self::Print => "print",
            Self::Texture => "texture",
        }
    }
}

struct Resources {
    root: PathBuf,
    module: Arc<producer::Module>,
    stock: serde_json::Value,
}
#[derive(Clone)]
pub struct Session {
    resources: Arc<Resources>,
    request: Request,
}
impl Session {
    pub fn new(data_dir: &Path, request: Request) -> Result<Self, String> {
        let root = data_dir.join("fotufilm");
        let stock = load_stock(&root, &request.stock)?;
        let module = producer::load(&root)?;
        let session = Self {
            resources: Arc::new(Resources {
                root,
                module,
                stock,
            }),
            request,
        };
        session.validate_request()?;
        Ok(session)
    }
    pub fn request(&self) -> &Request {
        &self.request
    }
    pub fn validate_request(&self) -> Result<(), String> {
        let mut input = self.envelope(1, 1, None, None);
        input["kind"] = "alignment-validate".into();
        let bytes = producer::prepare(
            &self.resources.module,
            &self.resources.root,
            &input,
            None,
            None,
        )?;
        let validation: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid original Fotufilm validation result: {e}"))?;
        if validation.get("valid").and_then(|v| v.as_bool()) != Some(true) {
            return Err("Fotufilm producer did not validate the request".into());
        }
        Ok(())
    }
    pub fn render(
        &self,
        width: u32,
        height: u32,
        scene: &[f32],
        seed: u64,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<f32>, String> {
        self.render_inner(width, height, scene, seed, cancellation, None)
    }
    pub fn render_stage(
        &self,
        width: u32,
        height: u32,
        scene: &[f32],
        seed: u64,
        cancellation: Option<&AtomicBool>,
        stage: Stage,
    ) -> Result<Vec<f32>, String> {
        self.render_inner(width, height, scene, seed, cancellation, Some(stage))
    }
    fn render_inner(
        &self,
        width: u32,
        height: u32,
        scene: &[f32],
        seed: u64,
        cancellation: Option<&AtomicBool>,
        stage: Option<Stage>,
    ) -> Result<Vec<f32>, String> {
        check_cancel(cancellation)?;
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .ok_or("Fotufilm image dimensions overflow")?;
        if width == 0
            || height == 0
            || scene.len()
                != pixels
                    .checked_mul(3)
                    .ok_or("Fotufilm sample count overflow")?
        {
            return Err("Fotufilm requires width × height × 3 samples".into());
        }
        if scene.iter().any(|v| !v.is_finite()) {
            return Err("Fotufilm input samples must be finite".into());
        }
        let mut rgba = Vec::with_capacity(pixels * 4);
        for pixel in scene.as_chunks::<3>().0 {
            rgba.extend_from_slice(pixel);
            rgba.push(1.0);
        }
        let input = self.envelope(width, height, Some(seed), stage);
        let bytes = producer::prepare(
            &self.resources.module,
            &self.resources.root,
            &input,
            Some(&rgba),
            cancellation,
        )?;
        drop(rgba);
        let prepared = packet::Prepared::parse(&bytes)?;
        if prepared.width != width || prepared.height != height {
            return Err("Fotufilm prepared geometry does not match the source image".into());
        }
        check_cancel(cancellation)?;
        let mut planar = vec![0.0; scene.len()];
        for (index, pixel) in scene.as_chunks::<3>().0.iter().enumerate() {
            for channel in 0..3 {
                planar[channel * pixels + index] = pixel[channel];
            }
        }
        let output = native::render(&prepared, &planar, cancellation)?;
        check_cancel(cancellation)?;
        let mut interleaved = vec![0.0; scene.len()];
        for (index, pixel) in interleaved.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            for channel in 0..3 {
                pixel[channel] = output[channel * pixels + index];
            }
        }
        Ok(interleaved)
    }
    fn envelope(
        &self,
        width: u32,
        height: u32,
        seed: Option<u64>,
        stage: Option<Stage>,
    ) -> serde_json::Value {
        let mut alignment = self.request.native.clone();
        if let Some(seed) = seed {
            alignment.insert("seed".into(), seed.into());
        }
        if let Some(stage) = stage {
            alignment.insert("stage".into(), stage.name().into());
        }
        serde_json::json!({"stock":self.resources.stock,"width":width,"height":height,"controls":self.request.controls,"alignment":alignment})
    }
}

pub fn stocks(data_dir: &Path) -> Result<Vec<StockInfo>, String> {
    let directory = data_dir.join("fotufilm/stocks");
    let entries = std::fs::read_dir(&directory)
        .map_err(|e| format!("Fotufilm stock directory {}: {e}", directory.display()))?;
    let mut stocks = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|n| n.to_str()) else {
            continue;
        };
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Fotufilm stock {}: {e}", path.display()))?;
        let name = value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(id)
            .to_owned();
        stocks.push(StockInfo {
            id: id.to_owned(),
            name,
        });
    }
    stocks.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    Ok(stocks)
}
pub fn catalogue(data_dir: &Path, stock: &str) -> Result<serde_json::Value, String> {
    let root = data_dir.join("fotufilm");
    let definition = load_stock(&root, stock)?;
    let module = producer::load(&root)?;
    let request = serde_json::json!({"kind":"alignment-catalogue","stock":definition});
    let bytes = producer::prepare(&module, &root, &request, None, None)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("Invalid original Fotufilm control catalogue: {e}"))
}
fn load_stock(root: &Path, id: &str) -> Result<serde_json::Value, String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
        || id == "."
        || id == ".."
    {
        return Err("Invalid Fotufilm stock identity".into());
    }
    let bytes = producer::verified_bytes(root, &format!("stocks/{id}.json"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Fotufilm stock {id}: {e}"))
}
fn check_cancel(cancellation: Option<&AtomicBool>) -> Result<(), String> {
    if cancellation.is_some_and(|c| c.load(Ordering::Relaxed)) {
        Err("Fotufilm operation cancelled".into())
    } else {
        Ok(())
    }
}
