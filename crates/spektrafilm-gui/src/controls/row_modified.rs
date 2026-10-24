//! Per-row modified markers derived from the section reset contract.

use egui::{Context, Ui};
use serde_json::Value;
use spektrafilm_core::params::RuntimeParams;

use crate::section_reset;

pub(crate) const MARK_COLOR: egui::Color32 = crate::theme::ACCENT;
const MARK_RADIUS: f32 = crate::theme::SPACE_1 * 0.75;

fn section_id() -> egui::Id {
    egui::Id::new("control_current_section")
}

pub(crate) fn current_section(ctx: &Context) -> Option<String> {
    ctx.data(|data| data.get_temp::<String>(section_id()))
}

/// Validate a changed row against the accepted runtime and shared typed validator.
/// This runs only on an actual edit, not while painting or searching rows.
pub(crate) fn accepts_value(ctx: &Context, path: &str, value: Value) -> bool {
    if path.starts_with("extras.") {
        return true;
    }
    let Some(RowScope::Modified { root, .. }) =
        ctx.data(|data| data.get_temp::<RowScope>(scope_id()))
    else {
        return true;
    };
    let Ok(mut candidate) = serde_json::from_value::<RuntimeParams>((*root).clone()) else {
        return false;
    };
    spektrafilm_core::params::sources::ParameterEdits::parse(&format!("{path}={value}"))
        .and_then(|edits| edits.apply(&mut candidate))
        .is_ok()
}
#[derive(Clone, Default)]
enum RowScope {
    #[default]
    Unmodified,
    Modified {
        section: &'static str,
        root: std::sync::Arc<Value>,
        extras: std::sync::Arc<Value>,
    },
}

impl RowScope {
    fn marks(&self, path: &str) -> bool {
        let Self::Modified {
            section,
            root,
            extras,
        } = self
        else {
            return false;
        };
        section_reset::leaf_modified(section, path, root, extras)
    }
}

fn scope_id() -> egui::Id {
    egui::Id::new("row_modified_scope")
}

pub(crate) fn install(
    ctx: &Context,
    section: &str,
    params: &RuntimeParams,
    extras: &Value,
) -> ScopeGuard {
    ctx.data_mut(|data| data.insert_temp(section_id(), section.to_owned()));
    let section = section_reset::sections().find(|name| *name == section);
    let scope = match section {
        Some(section) => {
            let projection = super::control_projection(params, extras);
            RowScope::Modified {
                section,
                root: projection.runtime,
                extras: projection.extras,
            }
        }
        _ => RowScope::Unmodified,
    };
    ctx.data_mut(|data| data.insert_temp(scope_id(), scope));
    ScopeGuard { ctx: ctx.clone() }
}

pub(crate) struct ScopeGuard {
    ctx: Context,
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        self.ctx
            .data_mut(|data| data.remove_temp::<RowScope>(scope_id()));
    }
}

#[derive(Clone, Copy)]
pub(crate) struct RowMark {
    marked: bool,
}

impl RowMark {
    pub(crate) const NONE: Self = Self { marked: false };

    pub(crate) fn resolve(ctx: &Context, path: Option<&str>) -> Self {
        let Some(path) = path else { return Self::NONE };
        let canonical = current_section(ctx)
            .and_then(|section| super::inventory::path_for_label(&section, path));
        let path = canonical.unwrap_or(path);
        Self {
            marked: ctx
                .data(|data| data.get_temp::<RowScope>(scope_id()))
                .is_some_and(|scope| scope.marks(path)),
        }
    }

    pub(crate) fn resolve_extras(ctx: &Context, section: &str, key: &str) -> Self {
        Self {
            marked: ctx
                .data(|data| data.get_temp::<RowScope>(scope_id()))
                .is_some_and(|scope| scope.marks(&format!("{section}.{key}"))),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_marked(self) -> bool {
        self.marked
    }
    pub(crate) fn paint(self, ui: &Ui, label_response: &egui::Response) {
        if self.marked {
            let center = label_response.rect.left_center() - egui::vec2(MARK_RADIUS + 1.0, 0.0);
            ui.painter().circle_filled(center, MARK_RADIUS, MARK_COLOR);
        }
    }

    pub(crate) fn gold<R>(self, ui: &mut Ui, add_value: impl FnOnce(&mut Ui) -> R) -> R {
        if !self.marked {
            return add_value(ui);
        }
        let previous = ui.visuals().override_text_color;
        ui.visuals_mut().override_text_color = Some(MARK_COLOR);
        let result = add_value(ui);
        ui.visuals_mut().override_text_color = previous;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn factory() -> RuntimeParams {
        crate::state::GuiState::factory().runtime_params().unwrap()
    }

    fn extras() -> Value {
        crate::state::GuiState::factory().sections.clone()
    }

    #[test]
    fn edited_leaf_is_marked() {
        let ctx = Context::default();
        let mut params = factory();
        let baseline = extras();
        let _guard = install(&ctx, "Film base", &params, &baseline);
        assert!(!RowMark::resolve(&ctx, Some("film_render.base.scale")).is_marked());
        params.film_render.base.scale += 0.5;
        let _guard = install(&ctx, "Film base", &params, &baseline);
        assert!(RowMark::resolve(&ctx, Some("film_render.base.scale")).is_marked());
        assert!(!RowMark::resolve(&ctx, Some("film_render.base.tilt")).is_marked());
    }

    #[test]
    fn unrelated_leaf_is_not_marked() {
        let ctx = Context::default();
        let mut params = factory();
        params.camera.diffusion_filter.strength += 1.0;
        let _guard = install(&ctx, "Camera", &params, &extras());
        assert!(!RowMark::resolve(&ctx, Some("camera.diffusion_filter.strength")).is_marked());
    }

    #[test]
    fn extras_leaf_is_marked() {
        let ctx = Context::default();
        let params = factory();
        let mut document = extras();
        document["special"]["film_channel_swap"] = serde_json::json!([1, 0, 2]);
        let _guard = install(&ctx, "Convert", &params, &document);
        assert!(RowMark::resolve_extras(&ctx, "special", "film_channel_swap").is_marked());
    }
}
