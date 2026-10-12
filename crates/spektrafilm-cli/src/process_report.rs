use serde_json::Value;
use spektrafilm_core::process_params::{EditFeedback, EditStatus};

pub(crate) fn warn_edits(edits: &[EditFeedback]) {
    for edit in edits {
        if edit.status == EditStatus::Active {
            continue;
        }
        let source = serde_json::to_value(&edit.source).expect("source serialization");
        eprintln!(
            "Warning: {}={} ({}): {}",
            edit.path,
            edit.requested,
            source_label(&source),
            edit.reason.as_deref().unwrap_or("edit is not active")
        );
    }
}

fn source_label(source: &Value) -> String {
    let option = source["option"].as_str().unwrap_or("RAW adapter");
    let index = source["index"]
        .as_u64()
        .map(|n| format!(" #{n}"))
        .unwrap_or_default();
    let file = source["path"]
        .as_str()
        .map(|p| format!(" {p}"))
        .unwrap_or_default();
    format!("{option}{index}{file}")
}

fn value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "not applicable".into(),
        other => other.to_string(),
    }
}

fn record(label: &str, fields: &Value) {
    println!("{label}");
    if let Some(fields) = fields.as_object() {
        for (name, field) in fields {
            println!("  {name}: {}", value(field));
        }
    }
}

pub(crate) fn print_text(report: &Value) {
    println!("INPUT");
    println!("  kind: {}", value(&report["input"]["kind"]));
    let declaration = &report["input"]["declaration"];
    for component in ["color_space", "cctf_decoding"] {
        println!(
            "  {component}: {} (source: {})",
            value(&declaration[component]),
            source_label(&declaration["sources"][component])
        );
    }
    if !report["input"]["raw_loading"].is_null() {
        record("RAW LOADING", &report["input"]["raw_loading"]);
    }
    record("DATA", &report["data"]);
    println!("LOOK AND WORKFLOW");
    println!("  film: {}", value(&report["film_profile"]));
    println!("  print: {}", value(&report["print_profile"]));
    let params = &report["parameters"];
    println!("  route: {}", value(&params["workflow"]["route"]));
    println!(
        "  scan output: {}",
        value(&params["scanner"]["scan_output"])
    );
    println!("  seed: {}", value(&report["seed"]));
    record("SCANNER", &params["scanner"]);
    record(
        "SCANNER RECEIVER",
        &report["scanner_provenance"]["receiver"],
    );
    record("SCANNER PROVENANCE", &report["scanner_provenance"]);
    println!(
        "  camera exposure: {} EV",
        value(&params["camera"]["exposure_compensation_ev"])
    );
    println!(
        "  auto exposure: {}",
        value(&params["camera"]["auto_exposure"])
    );
    println!(
        "  full-source long edge: {} mm",
        value(&params["camera"]["film_format_mm"])
    );
    record(
        "GRAIN (effective controls; conditions from describe --module film_render.grain)",
        &params["film_render"]["grain"],
    );
    record("GRAIN V3 ROUTING", &report["film_grain_v3"]);
    record(
        "BACKEND (requested, not device availability)",
        &report["backend"],
    );
    record("OUTPUT", &report["output"]);
    println!("EXPLICIT EDITS");
    if let Some(edits) = report["edits"].as_array() {
        for edit in edits {
            println!(
                "  {}: requested {} -> effective {} [{}; {}]{}",
                value(&edit["path"]),
                value(&edit["requested"]),
                value(&edit["effective"]),
                value(&edit["status"]),
                source_label(&edit["source"]),
                edit["reason"]
                    .as_str()
                    .map(|r| format!(" — {r}"))
                    .unwrap_or_default()
            );
        }
    }
    println!("NOT CHECKED");
    println!(
        "  Pixel decoding; compute-device availability; metered exposure; final pixel geometry"
    );
}
