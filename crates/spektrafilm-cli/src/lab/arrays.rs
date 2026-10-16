//! `.npy`/`.npz` array loading for measured-spectra sets.

use std::fs::File;
use std::path::Path;

/// Read one numeric array from a `.npy` file, or the named array from a
/// `.npz` archive. Returns `(shape, f64 data)`; `<f8` data is bit-exact.
pub fn read_array(path: &Path, key: Option<&str>) -> Result<(Vec<usize>, Vec<f64>), String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    match extension {
        "npy" => {
            let file = File::open(path).map_err(|e| format!("opening {}: {e}", path.display()))?;
            spektrafilm_math::npy::load_npy_f64(file)
                .map_err(|e| format!("reading {}: {e}", path.display()))
        }
        "npz" => {
            let key = key.ok_or_else(|| {
                format!(
                    "{} is an .npz archive; the array reference needs a key",
                    path.display()
                )
            })?;
            let file = File::open(path).map_err(|e| format!("opening {}: {e}", path.display()))?;
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| format!("opening npz archive {}: {e}", path.display()))?;
            let entry = archive
                .by_name(key)
                .map_err(|e| format!("reading array {key:?} from {}: {e}", path.display()))?;
            spektrafilm_math::npy::load_npy_f64(entry)
                .map_err(|e| format!("reading array {key:?} from {}: {e}", path.display()))
        }
        other => Err(format!(
            "unsupported array format {other:?} for {}; use .npy or .npz",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn npy_bytes(values: &[f64], rows: usize, columns: usize) -> Vec<u8> {
        let header =
            format!("{{'descr': '<f8', 'fortran_order': False, 'shape': ({rows}, {columns}), }}");
        let mut out = Vec::new();
        out.extend_from_slice(b"\x93NUMPY");
        out.push(1);
        out.push(0);
        out.extend_from_slice(&(header.len() as u16).to_le_bytes());
        out.extend_from_slice(header.as_bytes());
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("spektrafilm-lab-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_exact_f64_npy() {
        let dir = temp_dir("npy");
        let path = dir.join("reflectances.npy");
        let values = [0.1f64, 0.2, 0.3, 0.4, 0.5, 0.6];
        std::fs::write(&path, npy_bytes(&values, 2, 3)).unwrap();
        let (shape, data) = read_array(&path, None).unwrap();
        assert_eq!(shape, vec![2, 3]);
        for (got, want) in data.iter().zip(values) {
            assert_eq!(got.to_bits(), want.to_bits());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_named_npz_entry() {
        let dir = temp_dir("npz");
        let path = dir.join("corpus.npz");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file("other.npy", options).unwrap();
            writer.write_all(&npy_bytes(&[9.0], 1, 1)).unwrap();
            writer.start_file("reflectances.npy", options).unwrap();
            writer.write_all(&npy_bytes(&[0.25f64, 0.5], 1, 2)).unwrap();
            writer.finish().unwrap();
        }
        let (shape, data) = read_array(&path, Some("reflectances.npy")).unwrap();
        assert_eq!(shape, vec![1, 2]);
        assert_eq!(data, vec![0.25, 0.5]);
        assert!(read_array(&path, None).is_err());
        assert!(read_array(&path, Some("missing.npy")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
