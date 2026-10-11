use super::QaReport;
use std::path::{Path, PathBuf};

impl QaReport {
    /// Relative paths of the report documents and successful scenario figures.
    pub fn artifact_paths(&self) -> impl Iterator<Item = PathBuf> + '_ {
        std::iter::once(PathBuf::from("report.json")).chain(self.prints.iter().flat_map(|print| {
            let folder = Path::new(&print.folder);
            print
                .results
                .iter()
                .filter_map(|result| result.figure_path.as_ref())
                .map(move |figure| folder.join(figure))
                .chain([folder.join("report.md"), folder.join("report.html")])
        }))
    }
}

/// Write report documents without rerunning the measurements. Returned paths
/// include PNG assets, relative to `output_dir`, for delivery artifact registration.
pub fn write_report(report: &QaReport, output_dir: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    std::fs::write(
        output_dir.join("report.json"),
        serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    for p in &report.prints {
        let dir = output_dir.join(&p.folder);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut md = format!(
            "# QA report — {}\n\nPrint: `{}` (index {})\n\nReference: `{}` / Python 0.3.4\n\nBackend: {} / {}. Topology: {}, resolution: {}³.\n\nInput: {}. Output: {}. Off-grid: {} points, NumPy PCG64 seed {}.\n\n| Scenario | Status | Metrics |\n|---|---|---|\n",
            report.bundle_name,
            p.print_name,
            p.print_index,
            report.reference_commit,
            report.backend,
            report.precision,
            report.topology,
            report.resolution,
            report.input_color_space,
            report.output_color_space,
            report.offgrid_samples,
            report.rng_seed
        );
        let mut html = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>QA {}</title><style>body{{font:16px system-ui;max-width:1100px;margin:auto;padding:30px;background:#111;color:#eee}}table{{border-collapse:collapse}}td,th{{border:1px solid #777;padding:8px}}img{{max-width:100%}}code{{overflow-wrap:anywhere}}</style></head><body><h1>QA report — {}</h1><p>Print: {}. Reference: {}. Backend: {} / {}. {} {}³. Input: {}. Output: {}. Off-grid: {} / seed {}.</p>",
            escape(&report.bundle_name),
            escape(&report.bundle_name),
            escape(&p.print_name),
            report.reference_commit,
            escape(&report.backend),
            report.precision,
            report.topology,
            report.resolution,
            escape(&report.input_color_space),
            escape(&report.output_color_space),
            report.offgrid_samples,
            report.rng_seed
        );
        for r in &p.results {
            let status = match r.passed {
                Some(true) => "PASS",
                Some(false) => "FAIL",
                None => "INFO",
            };
            let metrics = r
                .summary
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            md.push_str(&format!("| {} | {} | {} |\n", r.name, status, metrics));
        }
        for r in &p.results {
            md.push_str(&format!(
                "\n## {}\n\nStatus: **{}**. Units: {}.\n\n{}\n\n",
                r.name,
                match r.passed {
                    Some(true) => "PASS",
                    Some(false) => "FAIL",
                    None => "INFO",
                },
                r.units,
                r.interpretation
            ));
            html.push_str(&format!("<h2>{}</h2><p>Status: <b>{}</b>. Units: {}.</p><p>{}</p><table><tr><th>Metric</th><th>Value</th><th>Reference</th></tr>",escape(&r.name),match r.passed{Some(true)=>"PASS",Some(false)=>"FAIL",None=>"INFO"},escape(&r.units),escape(&r.interpretation)));
            md.push_str("| Metric | Value | Reference |\n|---|---|---|\n");
            for (key, value) in &r.summary {
                let target = r
                    .reference_values
                    .get(key)
                    .map(String::as_str)
                    .unwrap_or("informational");
                md.push_str(&format!("| {} | {} | {} |\n", key, value, target));
                html.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                    escape(key),
                    escape(&value.to_string()),
                    escape(target)
                ));
            }
            html.push_str("</table>");
            if let Some(fig) = &r.figure_path {
                md.push_str(&format!("\n![{}]({})\n", r.name, fig));
                html.push_str(&format!(
                    "<p><img alt=\"{}\" src=\"{}\"></p>",
                    escape(&r.name),
                    escape(fig)
                ));
            }
        }
        html.push_str("</body></html>");
        for (name, text) in [("report.md", md), ("report.html", html)] {
            std::fs::write(dir.join(name), text).map_err(|e| e.to_string())?;
        }
    }
    Ok(report.artifact_paths().collect())
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
