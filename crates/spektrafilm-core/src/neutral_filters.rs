/// Per-stock neutral print filter database loader.
///
/// Python parity: `spektrafilm/runtime/params_builder.py:apply_database_neutral_print_filters`.
/// The JSON file maps (print_stock, illuminant, film_stock) → (c, m, y) filter CC values.
/// Without this lookup, Rust used hardcoded (0, 65, 55) which produced ~5% per-channel drift
/// from Python on common film/paper/illuminant combinations.
use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// Nested map: print_stock → illuminant → film_stock → [c, m, y].
type FilterDb = HashMap<String, HashMap<String, HashMap<String, [f64; 3]>>>;

#[derive(Debug, Clone)]
pub struct NeutralFilters {
    db: FilterDb,
}

impl NeutralFilters {
    /// Load the JSON database from `<data_dir>/filters/neutral_print_filters.json`.
    /// A missing file is an explicit no-op, matching Python's optional database.
    /// Present but malformed files are errors; silently treating them as empty
    /// would change calibrated output without telling the caller.
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let path = data_dir.join("filters").join("neutral_print_filters.json");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self { db: HashMap::new() });
            }
            Err(error) => {
                return Err(format!("reading neutral filter database {}: {error}", path.display()));
            }
        };
        let raw: RawDb = serde_json::from_str(&text)
            .map_err(|error| format!("parsing neutral filter database {}: {error}", path.display()))?;
        Ok(Self { db: raw.0 })
    }

    /// Look up filter CC values for a (print_stock, illuminant, film_stock) combination.
    /// Returns `None` if the combination isn't in the database.
    pub fn lookup(
        &self,
        print_stock: &str,
        illuminant: &str,
        film_stock: &str,
    ) -> Option<[f64; 3]> {
        self.db
            .get(print_stock)?
            .get(illuminant)?
            .get(film_stock)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_present_database_is_rejected() {
        let root = std::env::temp_dir().join(format!(
            "spektrafilm-neutral-filter-test-{}",
            std::process::id()
        ));
        let filters = root.join("filters");
        std::fs::create_dir_all(&filters).unwrap();
        std::fs::write(
            filters.join("neutral_print_filters.json"),
            b"{\"not\": \"a filter database\"",
        )
        .unwrap();

        let result = NeutralFilters::load(&root);
        assert!(result.is_err(), "malformed filter data must not become an empty database");
        let _ = std::fs::remove_dir_all(root);
    }
}

#[derive(Deserialize)]
struct RawDb(FilterDb);
