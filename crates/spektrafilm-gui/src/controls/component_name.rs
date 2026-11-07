//! One authoritative composition of a control label and one of its components.

use crate::i18n::Locale;
use std::borrow::Cow;

/// Compose the display name shared by the inspector and accessible labels.
pub(crate) fn component_name(
    locale: Locale,
    label: Cow<'_, str>,
    component: Cow<'_, str>,
) -> Cow<'static, str> {
    crate::i18n::report_catalog_fallbacks();
    rust_i18n::t!(
        "controls.component.name",
        locale = locale.as_str(),
        label = label,
        component = component
    )
}
