//! Scanner setup is measured from the complete decoded frame and applied to
//! the modeled scan interpretation. The public implementation lives at the
//! crate boundary so Preview, Scan, Export, and provenance use one type.

pub use crate::scanner_setup::ScannerSetup;
