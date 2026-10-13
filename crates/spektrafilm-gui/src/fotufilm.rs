use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::Ui;
use serde_json::{Value, json};
use spektrafilm_core::params::RuntimeParams;

#[derive(Default)]
pub(super) struct CatalogueCache {
    key: Option<(PathBuf, String)>,
    value: Option<Result<Value, String>>,
    stocks: Option<(
        PathBuf,
        Result<Vec<spektrafilm_fotufilm::StockInfo>, String>,
    )>,
}

impl CatalogueCache {
    fn get(&mut self, directory: &Path, stock: &str) -> Result<&Value, &str> {
        let key = (directory.to_owned(), stock.to_owned());
        if self.key.as_ref() != Some(&key) {
            self.value = Some(spektrafilm_fotufilm::catalogue(directory, stock));
            self.key = Some(key);
        }
        self.value
            .as_ref()
            .expect("catalogue loaded")
            .as_ref()
            .map_err(String::as_str)
    }
}

pub(super) fn show(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    directory: &Path,
    cache: &mut CatalogueCache,
) -> bool {
    let mut changed = false;
    let mut seed_value =
        BTreeMap::from([("Simulation seed".to_owned(), json!(params.random_seed))]);
    if override_json(ui, "Simulation seed", &json!(0u64), &mut seed_value) {
        if let Some(seed) = seed_value
            .get("Simulation seed")
            .unwrap_or(&json!(0u64))
            .as_u64()
        {
            params.random_seed = seed;
            changed = true;
        } else {
            ui.colored_label(egui::Color32::RED, "Simulation seed requires UInt64.");
        }
    }
    if !cache
        .stocks
        .as_ref()
        .is_some_and(|(path, _)| path == directory)
    {
        cache.stocks = Some((
            directory.to_owned(),
            spektrafilm_fotufilm::stocks(directory),
        ));
    }
    match &cache.stocks.as_ref().expect("stocks loaded").1 {
        Ok(stocks) => {
            egui::ComboBox::from_label("Fotufilm stock")
                .selected_text(&params.fotufilm.stock)
                .show_ui(ui, |ui| {
                    for stock in stocks {
                        changed |= ui
                            .selectable_value(
                                &mut params.fotufilm.stock,
                                stock.id.clone(),
                                &stock.name,
                            )
                            .changed();
                    }
                });
        }
        Err(error) => {
            ui.colored_label(egui::Color32::RED, error);
        }
    }
    let catalogue = match cache.get(directory, &params.fotufilm.stock) {
        Ok(catalogue) => catalogue,
        Err(error) => {
            ui.colored_label(egui::Color32::RED, error);
            return changed;
        }
    };
    if let (Some(sections), Some(controls)) = (
        catalogue["manifest"]["sections"].as_array(),
        catalogue["manifest"]["controls"].as_array(),
    ) {
        for section in sections {
            let title = format!(
                "{} · {}",
                section["groupTitle"].as_str().unwrap_or(""),
                section["title"].as_str().unwrap_or("")
            );
            ui.collapsing(title, |ui| {
                for control in controls
                    .iter()
                    .filter(|control| control["section"] == section["id"])
                {
                    let Some(field) = control["field"].as_str() else {
                        continue;
                    };
                    if ["stock", "seed"].contains(&field)
                        || !catalogue["simulationControls"]
                            .as_array()
                            .is_some_and(|fields| {
                                fields
                                    .iter()
                                    .any(|candidate| candidate.as_str() == Some(field))
                            })
                    {
                        continue;
                    }
                    let media = params
                        .fotufilm
                        .native
                        .get("paper")
                        .or_else(|| params.fotufilm.controls.get("paper"))
                        .and_then(Value::as_str)
                        .or_else(|| catalogue["nativeDefaults"]["paper"].as_str());
                    let media_available = media
                        .and_then(|media| {
                            catalogue["media"].as_array().and_then(|items| {
                                items.iter().find(|item| item["id"].as_str() == Some(media))
                            })
                        })
                        .map(|item| {
                            item["available"].as_array().is_some_and(|fields| {
                                fields
                                    .iter()
                                    .any(|candidate| candidate.as_str() == Some(field))
                            })
                        })
                        .unwrap_or(true);
                    let available = catalogue["stockApplicability"][field]
                        .as_bool()
                        .unwrap_or(false)
                        && media_available;
                    ui.push_id(field, |ui| {
                        ui.add_enabled_ui(available, |ui| {
                            changed |= editor_control(
                                ui,
                                control,
                                catalogue,
                                &mut params.fotufilm.controls,
                            );
                        });
                        if !available && params.fotufilm.controls.contains_key(field) {
                            ui.colored_label(
                                egui::Color32::YELLOW,
                                "This override is unavailable for this stock.",
                            );
                            if ui.button("Use stock default").clicked() {
                                params.fotufilm.controls.remove(field);
                                changed = true;
                            }
                        }
                    });
                }
            });
        }
    }
    ui.collapsing("Original native options", |ui| {
        ui.label("Unset options keep the original stock defaults. JSON fields show source examples; press Apply to enable an example.");
        if let Some(extensions) = catalogue["nativeExtensions"].as_object() {
            for (group, descriptor) in extensions {
                if group == "seed" { continue; }
                if ["scalars", "flags", "arrays"].contains(&group.as_str()) {
                    if let Some(fields) = descriptor.as_array() {
                        for field in fields.iter().filter_map(Value::as_str) {
                            ui.label(format!("{group}.{field}"));
                            changed |= native_option(ui, group, field, &catalogue["nativeDefaults"][group][field], &mut params.fotufilm.native);
                        }
                    }
                } else {
                    ui.label(format!("{group}: {descriptor}"));
                    changed |= native_option(ui, "", group, &catalogue["nativeDefaults"][group], &mut params.fotufilm.native);
                }
            }
        }
        if let Some(fields) = catalogue["nativeStructured"].as_array() {
            for field in fields.iter().filter_map(Value::as_str) {
                changed |= native_option(ui, "", field, &catalogue["nativeDefaults"][field], &mut params.fotufilm.native);
            }
        }
    });
    changed
}

fn editor_control(
    ui: &mut Ui,
    control: &Value,
    catalogue: &Value,
    values: &mut BTreeMap<String, Value>,
) -> bool {
    let field = control["field"].as_str().unwrap_or("");
    let title = control["title"].as_str().unwrap_or(field);
    let detail = control["detail"].as_str().unwrap_or("");
    let mut changed = false;
    let mut value = values
        .get(field)
        .cloned()
        .unwrap_or_else(|| catalogue["defaults"][field].clone());
    let kind = control["kind"].as_str().unwrap_or("");
    ui.horizontal(|ui| {
        ui.label(title).on_hover_text(detail);
        if ui.small_button("Reset").clicked() {
            values.remove(field);
            value = Value::Null;
            changed = true;
        }
    });
    if changed {
        return true;
    }
    match kind {
        "toggle" => {
            let mut on = value
                .as_bool()
                .unwrap_or_else(|| control["restingOn"].as_bool().unwrap_or(false));
            if ui.checkbox(&mut on, title).on_hover_text(detail).changed() {
                value = json!(on);
                changed = true;
            }
        }
        "slider" | "chips" => {
            let scale = &control["scale"];
            let mut number = value
                .as_f64()
                .unwrap_or_else(|| scale["neutral"].as_f64().unwrap_or(0.0));
            let min = scale["admittedMin"].as_f64().unwrap_or(f64::MIN);
            let max = scale["admittedMax"].as_f64().unwrap_or(f64::MAX);
            if crate::numeric::numeric(ui, title, &mut number, min, max, 0.01, 3, detail) {
                value = json!(number);
                changed = true;
            }
        }
        "menu" => {
            let choices = control["choices"].as_array().or_else(|| {
                control["dynamicMenu"]
                    .as_str()
                    .and_then(|menu| catalogue["menus"][menu].as_array())
            });
            if let Some(choices) = choices {
                egui::ComboBox::from_label(title)
                    .selected_text(value.as_str().unwrap_or("Original default"))
                    .show_ui(ui, |ui| {
                        for choice in choices {
                            let candidate = choice["id"].clone();
                            changed |= ui
                                .selectable_value(
                                    &mut value,
                                    candidate,
                                    choice["label"].as_str().unwrap_or(""),
                                )
                                .on_hover_text(choice["detail"].as_str().unwrap_or(""))
                                .changed();
                        }
                    });
            } else {
                return override_json(ui, field, &value, values);
            }
        }
        _ => return override_json(ui, field, &value, values),
    }
    if changed && !value.is_null() {
        values.insert(field.to_owned(), value);
    }
    changed
}
fn native_option(
    ui: &mut Ui,
    group: &str,
    field: &str,
    default: &Value,
    values: &mut BTreeMap<String, Value>,
) -> bool {
    let mut fields: BTreeMap<String, Value> = if group.is_empty() {
        values.clone()
    } else {
        values
            .get(group)
            .and_then(Value::as_object)
            .map(|object| {
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let changed = ui
        .push_id(group, |ui| override_json(ui, field, default, &mut fields))
        .inner;
    if changed {
        if group.is_empty() {
            *values = fields;
        } else if fields.is_empty() {
            values.remove(group);
        } else {
            values.insert(group.to_owned(), json!(fields));
        }
    }
    changed
}

fn override_json(
    ui: &mut Ui,
    field: &str,
    default: &Value,
    values: &mut BTreeMap<String, Value>,
) -> bool {
    ui.push_id(
        (
            "json",
            field,
            values.get(field).unwrap_or(default).to_string(),
        ),
        |ui| {
            let id = ui.make_persistent_id("text");
            let mut text = ui
                .ctx()
                .data_mut(|data| data.get_temp::<String>(id))
                .unwrap_or_else(|| values.get(field).unwrap_or(default).to_string());
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label(field);
                if ui.small_button("Reset").clicked() {
                    changed |= values.remove(field).is_some();
                    text = default.to_string();
                }
            });
            let response = ui.text_edit_singleline(&mut text);
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_label(field));
            let apply = ui.small_button(format!("Apply {field}")).clicked();
            match serde_json::from_str::<Value>(&text) {
                Ok(value) => {
                    if apply {
                        if value.is_null() {
                            values.remove(field);
                        } else {
                            values.insert(field.to_owned(), value);
                        }
                        changed = true;
                    }
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::RED, format!("Invalid JSON: {error}"));
                }
            }
            ui.ctx().data_mut(|data| data.insert_temp(id, text));
            changed
        },
    )
    .inner
}
