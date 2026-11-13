//! Profile-dependent control applicability and the inspector's material cache.
use std::sync::Arc;

use egui::Context;
use spektrafilm_core::params::RuntimeParams;
use spektrafilm_core::profile::Profile;

const MATERIAL_CONTEXT_ID: &str = "inspector-material-context";

pub(crate) struct InspectorMaterials {
    pub(crate) film_name: String,
    pub(crate) print_name: String,
    pub(crate) film: Arc<Profile>,
    pub(crate) print: Option<Arc<Profile>>,
}

/// Materials the inspector last loaded for applicability checks.
pub(crate) fn materials(ctx: &Context) -> Option<Arc<InspectorMaterials>> {
    ctx.data(|data| data.get_temp::<Arc<InspectorMaterials>>(egui::Id::new(MATERIAL_CONTEXT_ID)))
}

/// Publish the loaded materials for the controls rendered later this frame.
pub(crate) fn install_material_context(
    ctx: &Context,
    film_name: String,
    print_name: String,
    film: Arc<Profile>,
    print: Option<Arc<Profile>>,
) {
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new(MATERIAL_CONTEXT_ID),
            Arc::new(InspectorMaterials {
                film_name,
                print_name,
                film,
                print,
            }),
        )
    });
}

pub(crate) fn grain_control_inactive_reason(
    ctx: &Context,
    path: &str,
    params: &RuntimeParams,
    component: Option<usize>,
) -> Option<&'static str> {
    match materials(ctx) {
        Some(materials) => {
            spektrafilm_core::params::sources::grain_control_inactive_reason_with_profile(
                path,
                params,
                &materials.film,
                component,
            )
        }
        None => spektrafilm_core::params::sources::grain_control_inactive_reason(
            path, params, component,
        ),
    }
}

pub(crate) fn control_inactive_reason(
    ctx: &Context,
    path: &str,
    params: &RuntimeParams,
) -> Option<String> {
    match materials(ctx) {
        Some(materials) => spektrafilm_core::params::sources::inactive_reason_with_materials(
            path,
            params,
            Some(&materials.film),
            materials.print.as_deref(),
        ),
        None => spektrafilm_core::params::sources::inactive_reason_without_profiles(path, params)
            .map(str::to_owned),
    }
}
