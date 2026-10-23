fn main() {
    println!("cargo:rerun-if-env-changed=SPEKTRAFILM_EMBED_MANIFEST");
    println!("cargo:rustc-check-cfg=cfg(embed_bundle)");
    if std::env::var_os("SPEKTRAFILM_EMBED_MANIFEST").is_some() {
        println!("cargo:rustc-cfg=embed_bundle");
    }
    // Catalogs are embedded by rust-i18n. Record missing locale entries once at
    // build time so defensive fallback is visible without parsing during paint.
    println!("cargo:rerun-if-changed=locales/en.json");
    println!("cargo:rerun-if-changed=locales/zh-CN.json");
    let catalogs: Vec<_> = ["en", "zh-CN"]
        .into_iter()
        .map(|locale| {
            let source = std::fs::read_to_string(format!("locales/{locale}.json"))
                .expect("read embedded GUI catalog");
            let values: std::collections::BTreeMap<String, String> = serde_json::from_str(&source)
                .expect("GUI catalog must contain flat semantic string keys");
            (locale, values)
        })
        .collect();
    let keys: std::collections::BTreeSet<_> = catalogs
        .iter()
        .flat_map(|(_, catalog)| catalog.keys())
        .collect();
    let mut diagnostics = String::from("const MISSING_TRANSLATIONS: &[(&str, &str)] = &[\n");
    for (locale, catalog) in &catalogs {
        for key in &keys {
            if !catalog.contains_key(*key) {
                diagnostics.push_str(&format!("    ({locale:?}, {key:?}),\n"));
            }
        }
    }
    diagnostics.push_str("];\n");
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    std::fs::write(output.join("catalog_diagnostics.rs"), diagnostics)
        .expect("write GUI catalog diagnostics");
}
