//! One page, built from the manifest and nothing else.
//!
//! The generator reads a [`Manifest`][super::manifest::Manifest] and prints
//! photographs with labels. Every string below either comes from that model or
//! names a control the reader operates.
use super::Assets;
use super::escape::get;
use super::escape::{CSS, escape};
use super::manifest::{Case, Image, Manifest, View};
use crate::common::{Error, Result};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const NAV: &str = "案例";
const FIT: &str = "适应";
const FULLSCREEN: &str = "全屏";

fn size(sizes: &BTreeMap<String, [u32; 2]>, image: &Image) -> [u32; 2] {
    sizes
        .get(&image.path)
        .copied()
        .filter(|[w, h]| *w > 0 && *h > 0)
        .unwrap_or([1, 1])
}

fn rect(view: &View, size: [u32; 2]) -> [u32; 4] {
    view.crop.unwrap_or([0, 0, size[0], size[1]])
}

/// The thumbnail's aspect ratio. The box itself is as wide as the rail, so the
/// background maths is expressed in percentages and fits whatever width it gets.
const THUMB_W: f64 = 88.0;
const THUMB_H: f64 = 50.0;

/// A CSS background that shows exactly the view's window of the reference.
fn thumbnail(sizes: &BTreeMap<String, [u32; 2]>, image: &Image, view: &View) -> String {
    let [image_width, image_height] = size(sizes, image);
    let [x, y, crop_width, crop_height] = rect(view, [image_width, image_height]);
    // cover: the window fills the box, and whatever falls outside is trimmed
    let k = (THUMB_W / crop_width as f64).max(THUMB_H / crop_height as f64);
    let wide = image_width as f64 * k;
    let tall = image_height as f64 * k;
    // percentage positioning resolves against (box - image), which is negative
    // once the image overflows — that is what a crop is.
    let percent = |offset: f64, room: f64| {
        if room.abs() < 0.001 {
            0.0
        } else {
            (offset / room * 10000.0).round() / 100.0
        }
    };
    let left = percent(
        THUMB_W / 2.0 - (x as f64 + crop_width as f64 / 2.0) * k,
        THUMB_W - wide,
    );
    let top = percent(
        THUMB_H / 2.0 - (y as f64 + crop_height as f64 / 2.0) * k,
        THUMB_H - tall,
    );
    format!(
        "background-image:url('{}');background-size:{}% {}%;background-position:{}% {}%",
        escape(&image.path),
        (wide / THUMB_W * 10000.0).round() / 100.0,
        (tall / THUMB_H * 10000.0).round() / 100.0,
        left,
        top
    )
}

fn locator(sizes: &BTreeMap<String, [u32; 2]>, image: &Image, view: &View) -> String {
    let [image_width, image_height] = size(sizes, image);
    let Some([x, y, width, height]) = view.crop else {
        return String::new();
    };
    if width >= image_width && height >= image_height {
        return String::new();
    }
    format!(
        "<div class=\"locator\" aria-hidden=\"true\"><img src=\"{}\" alt=\"\"><span style=\"left:{:.3}%;top:{:.3}%;width:{:.3}%;height:{:.3}%\"></span></div>",
        escape(&image.path),
        100.0 * x as f64 / image_width as f64,
        100.0 * y as f64 / image_height as f64,
        100.0 * width as f64 / image_width as f64,
        100.0 * height as f64 / image_height as f64
    )
}

fn plane(image: &Image, role: &str) -> String {
    let fit = image.fit.as_deref().unwrap_or(&image.path);
    format!(
        "<img class=\"plane {role}\" data-plane=\"{role}\" src=\"{}\" data-fit=\"{}\" data-native=\"{}\" alt=\"{}\" draggable=\"false\">",
        escape(fit),
        escape(fit),
        escape(&image.path),
        escape(&image.label)
    )
}

const EXPAND: &str = "<svg viewBox=\"0 0 14 14\" width=\"15\" height=\"15\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\"><path d=\"M5.5 1.5H1.5v4M8.5 1.5h4v4M5.5 12.5H1.5v-4M8.5 12.5h4v-4\"/></svg>";

fn hud() -> String {
    format!(
        "<div class=\"hud\"><button class=\"btn\" type=\"button\" data-scale>{FIT}</button><button class=\"btn\" type=\"button\" data-fullscreen aria-label=\"{FULLSCREEN}\" title=\"{FULLSCREEN}\">{EXPAND}</button></div>"
    )
}

fn stage(sizes: &BTreeMap<String, [u32; 2]>, case: &Case, view: &View, index: usize) -> String {
    let size = size(sizes, &case.reference);
    let [x, y, width, height] = rect(view, size);
    let aspect = width as f64 / height.max(1) as f64;
    let hidden = if index == 0 { "" } else { " hidden" };
    let mut inner = plane(&case.reference, "reference");
    if case.aligned
        && let Some(variant) = case.variants.first()
    {
        inner.push_str(&plane(variant, "variant"));
        inner.push_str("<div class=\"divider\" data-divider role=\"slider\" tabindex=\"0\" aria-label=\"分割位置\" aria-valuemin=\"0\" aria-valuemax=\"100\" aria-valuenow=\"50\" style=\"left:50%\"><span aria-hidden=\"true\"><svg viewBox=\"0 0 14 14\" width=\"14\" height=\"14\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\"><path d=\"M5.5 4 2.5 7l3 3M8.5 4l3 3-3 3\"/></svg></span></div>");
    }
    inner.push_str(&format!(
        "<span class=\"edge edge-left\">{}</span><span class=\"edge edge-right\">{}</span>",
        escape(&case.reference.label),
        escape(
            case.variants
                .first()
                .map(|v| v.label.as_str())
                .unwrap_or("")
        )
    ));
    inner.push_str(&locator(sizes, &case.reference, view));
    let controls = hud();
    format!(
        "<div class=\"surface\" data-surface data-view=\"{index}\" style=\"--ar:{aspect:.5}\"{hidden}><div class=\"stage\" data-stage data-crop=\"{x},{y},{width},{height}\" data-aligned=\"{}\" tabindex=\"0\" aria-label=\"{}\">{inner}</div>{controls}</div>",
        case.aligned,
        escape(&case.title)
    )
}

fn stages(sizes: &BTreeMap<String, [u32; 2]>, case: &Case) -> String {
    let mut html = String::from("<div class=\"stages\">");
    if case.aligned || case.variants.is_empty() {
        for (index, view) in case.views.iter().enumerate() {
            html.push_str(&stage(sizes, case, view, index));
        }
    } else {
        // Unregistered captures do not share a pixel grid; each keeps its own
        // surface instead of pretending one divider compares them.
        for (index, view) in case.views.iter().enumerate() {
            let size = size(sizes, &case.reference);
            let [x, y, width, height] = rect(view, size);
            let aspect = width as f64 / height.max(1) as f64;
            let mut html_sides = String::new();
            for (position, image) in std::iter::once(&case.reference)
                .chain(case.variants.iter())
                .enumerate()
            {
                let hidden = if index == 0 { "" } else { " hidden" };
                let face = plane(
                    image,
                    if position == 0 {
                        "reference"
                    } else {
                        "variant"
                    },
                );
                let controls = hud();
                html_sides.push_str(&format!(
                    "<div class=\"surface\" data-surface data-view=\"{index}\" style=\"--ar:{aspect:.5}\"{hidden}><div class=\"stage\" data-stage data-crop=\"{x},{y},{width},{height}\" data-aligned=\"false\" tabindex=\"0\" aria-label=\"{}\">{face}</div>{controls}</div>",
                    escape(&image.label)
                ));
            }
            html.push_str(&format!(
                "<div class=\"stages-pair\" data-view=\"{index}\"{}>{html_sides}</div>",
                if index == 0 { "" } else { " hidden" }
            ));
        }
    }
    html.push_str("</div>");
    html
}

fn views(sizes: &BTreeMap<String, [u32; 2]>, case: &Case) -> String {
    let mut html = String::from("<nav class=\"views\" aria-label=\"查看位置\">");
    for (index, view) in case.views.iter().enumerate() {
        html.push_str(&format!(
            "<button type=\"button\" data-view=\"{index}\" data-crop=\"{}\"{} aria-pressed=\"{}\" title=\"{}\"><span class=\"thumb\" style=\"{}\"></span><span class=\"label\">{}</span></button>",
            view.crop
                .map(|[x, y, w, h]| format!("{x},{y},{w},{h}"))
                .unwrap_or_default(),
            view.notes
                .first()
                .map(|note| format!(" data-note=\"{}\"", escape(note)))
                .unwrap_or_default(),
            index == 0,
            escape(&view.label),
            thumbnail(sizes, &case.reference, view),
            escape(&view.label)
        ));
    }
    html.push_str("</nav>");
    html
}

fn variants(case: &Case) -> String {
    if case.variants.len() < 2 {
        return String::new();
    }
    let mut html = String::from("<div class=\"variants\" role=\"tablist\" aria-label=\"候选\">");
    for (index, variant) in case.variants.iter().enumerate() {
        html.push_str(&format!(
            "<button class=\"btn\" type=\"button\" role=\"tab\" data-variant=\"{index}\" data-fit=\"{}\" data-native=\"{}\" aria-selected=\"{}\">{}</button>",
            escape(variant.fit.as_deref().unwrap_or(&variant.path)),
            escape(&variant.path),
            index == 0,
            escape(&variant.label)
        ));
    }
    html.push_str("</div>");
    html
}

fn notes(notes: &[String], class: &str) -> String {
    notes
        .iter()
        .map(|note| format!("<p class=\"{class}\">{}</p>", escape(note)))
        .collect()
}

fn case(sizes: &BTreeMap<String, [u32; 2]>, case: &Case, index: usize) -> String {
    let hidden = if index == 0 { "" } else { " hidden" };
    format!(
        "<section class=\"case\" data-case-panel data-index=\"{index}\" data-case-id=\"{}\"{hidden}>{}<div class=\"frame\">{}<div class=\"right\">{}<div class=\"stages\">{}</div></div></div><p class=\"note\" data-view-note hidden></p></section>",
        escape(&case.id),
        notes(&case.notes, "note"),
        views(sizes, case),
        variants(case),
        stages(sizes, case)
    )
}

pub(crate) fn page(
    manifest: &Manifest,
    sizes: &BTreeMap<String, [u32; 2]>,
    version: &str,
) -> String {
    // The case title is the case switcher: one control at the head of the page,
    // naming what the page is about and changing it. Without a second case the
    // title is just text, and gains no affordance it cannot honour.
    let mut title = String::new();
    if manifest.cases.len() > 1 {
        title.push_str(&format!(
            "<div class=\"dropdown cases\" data-cases><button class=\"case-button dropdown-toggle\" type=\"button\" data-bs-toggle=\"dropdown\" aria-expanded=\"false\" aria-label=\"{NAV}\" data-case-button><span data-case-name>{}</span></button><ul class=\"dropdown-menu\" data-case-menu>",
            escape(&manifest.cases[0].title)
        ));
        for (index, case) in manifest.cases.iter().enumerate() {
            title.push_str(&format!(
                "<li><button class=\"dropdown-item{active}\" type=\"button\" data-case-option=\"{index}\" aria-current=\"{current}\">{}</button></li>",
                escape(&case.title),
                active = if index == 0 { " active" } else { "" },
                current = if index == 0 { "true" } else { "false" },
            ));
        }
        title.push_str("</ul></div>");
    } else if let Some(case) = manifest.cases.first() {
        title.push_str(&format!("<h1>{}</h1>", escape(&case.title)));
    }
    let body: String = manifest
        .cases
        .iter()
        .enumerate()
        .map(|(index, panel)| case(sizes, panel, index))
        .collect();
    format!(
        "<!doctype html><html lang=\"zh-CN\" data-bs-theme=\"dark\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'none'; base-uri 'none'; form-action 'none'\"><title>{}</title><link rel=\"stylesheet\" href=\"bootstrap.css\"><style>:root{{--thumb-w:{THUMB_W};--thumb-h:{THUMB_H}}}</style><style>{CSS}</style><body><header><p class=\"brand\">{}</p>{title}</header><main>{body}</main><script src=\"bootstrap.bundle.js\"></script><script src=\"viewer.js?v={version}\"></script></body></html>",
        escape(&manifest.title),
        escape(&manifest.title)
    )
}

pub(crate) fn rectangle(region: &Value) -> Result<Option<[u32; 4]>> {
    let geometry = &region["geometry"];
    // A named geometry carries its span as its own fields; the array and
    // `rectangle` forms stay readable for retained records that use them.
    let mut named: Option<Value> = None;
    if let Some(object) = geometry.as_object()
        && let (Some(x), Some(y), Some(width), Some(height)) = (
            object.get("x").and_then(Value::as_u64),
            object.get("y").and_then(Value::as_u64),
            object.get("width").and_then(Value::as_u64),
            object.get("height").and_then(Value::as_u64),
        )
    {
        named = Some(json!([x, y, width, height]));
    }
    let rect = region
        .get("rectangle")
        .or_else(|| region.get("rect"))
        .or_else(|| {
            if geometry.is_array() {
                Some(geometry)
            } else {
                geometry.get("rectangle").or_else(|| geometry.get("rect"))
            }
        })
        .or(named.as_ref());
    let Some(rect) = rect else { return Ok(None) };
    let values = rect.as_array().filter(|v| v.len() == 4).ok_or_else(|| {
        Error::new(
            "region_bounds",
            "detail rectangle must contain four integers",
        )
    })?;
    let mut result = [0; 4];
    for (i, v) in values.iter().enumerate() {
        result[i] = v
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| {
                Error::new(
                    "region_bounds",
                    "detail rectangle must contain four nonnegative integers",
                )
            })?;
    }
    if result[2] == 0 || result[3] == 0 {
        return Err(Error::new(
            "region_bounds",
            "detail rectangle must have positive size",
        ));
    }
    Ok(Some(result))
}

pub(crate) fn geometry_rectangle(
    geometry: &Value,
    assets: &mut Assets,
) -> Result<Option<[u32; 4]>> {
    if let Some(path) = geometry["path"].as_str() {
        let Some(entry) = assets
            .copy(&json!(path))?
            .filter(|e| e["availability"] == "available")
        else {
            return Ok(None);
        };
        let (mask, w, h) = retained_mask(&assets.out.join(get(&entry, "path", "")))?;
        let mut bounds = [w, h, 0, 0];
        let mut found = false;
        for (i, valid) in mask.iter().enumerate() {
            if *valid {
                found = true;
                bounds[0] = bounds[0].min(i % w);
                bounds[1] = bounds[1].min(i / w);
                bounds[2] = bounds[2].max(i % w);
                bounds[3] = bounds[3].max(i / w);
            }
        }
        if !found {
            return Err(Error::new("region_bounds", "retained region mask is empty"));
        }
        return Ok(Some([
            bounds[0] as u32,
            bounds[1] as u32,
            (bounds[2] - bounds[0] + 1) as u32,
            (bounds[3] - bounds[1] + 1) as u32,
        ]));
    }
    rectangle(&json!({"geometry":geometry}))
}

/// A retained region mask as booleans, row-major, with its width and height.
pub(crate) fn retained_mask(
    path: &std::path::Path,
) -> crate::common::Result<(Vec<bool>, usize, usize)> {
    if path.extension().is_some_and(|v| v == "npy") {
        let (data, shape) = crate::image_data::read_npy(path)?;
        if shape.len() != 2 || shape[0] == 0 || shape[1] == 0 || data.iter().any(|v| !v.is_finite())
        {
            return Err(Error::new(
                "invalid_artifact",
                "retained region mask must contain finite two-dimensional samples",
            ));
        }
        Ok((
            data.into_iter().map(|v| v != 0.0).collect(),
            shape[1],
            shape[0],
        ))
    } else {
        let image = image::open(path)
            .map_err(|e| Error::new("invalid_artifact", e.to_string()))?
            .to_luma32f();
        Ok((
            image.pixels().map(|p| p[0] != 0.0).collect(),
            image.width() as usize,
            image.height() as usize,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::manifest::{Case, Image, Manifest, View};

    fn image(path: &str, label: &str) -> Image {
        Image {
            path: path.into(),
            fit: None,
            label: label.into(),
        }
    }

    fn manifest() -> Manifest {
        Manifest {
            title: "t".into(),
            cases: vec![Case {
                id: "a".into(),
                title: "A".into(),
                aligned: true,
                reference: image("a/ref.png", "基准"),
                variants: vec![image("a/var.png", "候选")],
                views: vec![
                    View {
                        id: "full".into(),
                        label: "全图".into(),
                        crop: None,
                        notes: vec![],
                    },
                    View {
                        id: "patch".into(),
                        label: "patch".into(),
                        crop: Some([10, 20, 40, 30]),
                        notes: vec!["更硬".into()],
                    },
                ],
                notes: vec!["整体偏冷".into()],
            }],
        }
    }

    #[test]
    fn a_page_only_prints_the_model_and_its_controls() {
        let mut sizes = BTreeMap::new();
        sizes.insert("a/ref.png".to_string(), [100u32, 100u32]);
        sizes.insert("a/var.png".to_string(), [100u32, 100u32]);
        let html = page(&manifest(), &sizes, "test");
        assert!(html.contains("整体偏冷"));
        assert!(html.contains(">patch<"));
        assert!(html.contains("data-note=\"更硬\""));
        assert!(html.contains("data-crop=\"10,20,40,30\""));
        assert!(html.contains("class=\"locator\""));
        assert!(html.contains("data-divider"));
        assert!(
            !html.contains("候选\"><span"),
            "one variant needs no picker"
        );
    }

    #[test]
    fn the_title_is_the_switcher_only_when_there_is_something_to_switch_to() {
        let mut sizes = BTreeMap::new();
        sizes.insert("a/ref.png".to_string(), [100u32, 100u32]);
        sizes.insert("a/var.png".to_string(), [100u32, 100u32]);

        let single = page(&manifest(), &sizes, "test");
        assert!(!single.contains("data-case-menu"));
        assert!(single.contains("<h1>A</h1>"));

        let mut model = manifest();
        let mut second = model.cases[0].clone();
        second.id = "b".into();
        second.title = "B".into();
        model.cases.push(second);
        let several = page(&model, &sizes, "test");
        assert_eq!(
            several.matches("data-case-menu").count(),
            1,
            "one menu, not one per case"
        );
        assert_eq!(several.matches("data-case-option=").count(), 2);
        assert!(several.contains("<span data-case-name>A</span>"));
        assert!(
            !several.contains("<h1>"),
            "the menu names the case; a heading repeats it"
        );
        assert!(
            !several.contains("<select"),
            "a native select opens a system menu the page cannot shape"
        );
    }

    #[test]
    fn the_controls_sit_under_the_print_in_both_alignment_modes() {
        let mut sizes = BTreeMap::new();
        sizes.insert("a/ref.png".to_string(), [100u32, 100u32]);
        sizes.insert("a/var.png".to_string(), [100u32, 100u32]);

        let mut unaligned = manifest();
        unaligned.cases[0].aligned = false;
        unaligned.cases[0].views.truncate(1);
        for model in [manifest(), unaligned] {
            let html = page(&model, &sizes, "test");
            let surfaces = html.matches("data-surface").count();
            assert_eq!(
                html.matches("</div><div class=\"hud\">").count(),
                surfaces,
                "every surface puts its bar after the print, never over it"
            );
        }
    }

    #[test]
    fn the_page_links_the_vendored_framework() {
        let mut sizes = BTreeMap::new();
        sizes.insert("a/ref.png".to_string(), [100u32, 100u32]);
        sizes.insert("a/var.png".to_string(), [100u32, 100u32]);
        let html = page(&manifest(), &sizes, "test");
        assert!(html.contains("<link rel=\"stylesheet\" href=\"bootstrap.css\">"));
        assert!(html.contains("<script src=\"bootstrap.bundle.js\"></script>"));
        assert!(
            html.contains("data-bs-theme=\"dark\""),
            "the theme is Bootstrap's dark set, then ours"
        );
        assert!(
            html.contains("style-src 'self' 'unsafe-inline'"),
            "an external stylesheet needs 'self'; the inline block still needs 'unsafe-inline'"
        );
    }

    #[test]
    fn several_variants_get_a_picker() {
        let mut model = manifest();
        model.cases[0].variants.push(image("a/var2.png", "31"));
        let mut sizes = BTreeMap::new();
        for path in ["a/ref.png", "a/var.png", "a/var2.png"] {
            sizes.insert(path.to_string(), [100u32, 100u32]);
        }
        let html = page(&model, &sizes, "test");
        assert_eq!(html.matches("data-variant=").count(), 2);
        assert!(html.contains(">31<"));
    }

    #[test]
    fn an_unaligned_case_gets_two_surfaces_and_no_divider() {
        let mut model = manifest();
        model.cases[0].aligned = false;
        model.cases[0].views.truncate(1);
        let mut sizes = BTreeMap::new();
        sizes.insert("a/ref.png".to_string(), [100u32, 100u32]);
        sizes.insert("a/var.png".to_string(), [100u32, 100u32]);
        let html = page(&model, &sizes, "test");
        assert!(!html.contains("data-divider"));
        assert_eq!(html.matches("data-stage").count(), 2);
    }

    #[test]
    fn a_full_frame_view_draws_no_locator() {
        let mut sizes = BTreeMap::new();
        sizes.insert("a/ref.png".to_string(), [100u32, 100u32]);
        sizes.insert("a/var.png".to_string(), [100u32, 100u32]);
        let mut model = manifest();
        model.cases[0].views = vec![model.cases[0].views[0].clone()];
        assert!(!page(&model, &sizes, "test").contains("class=\"locator\""));
    }
}
