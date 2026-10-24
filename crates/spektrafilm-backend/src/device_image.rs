//! One retained computation image carrier shared by every backend
//! (`specs/gpu-execution/design.md`, Device residency).
//!
//! A stage chain that carries host `ImageBuf`s between numerical stages
//! materializes an intermediate on the host to execute the next stage on the
//! device. This type is the single carrier a production route holds instead:
//! dimensions, sample domain, arithmetic precision, colour state, and working
//! pixel pitch travel with the samples, and the samples stay where they were
//! computed.
//!
//! The carrier names no device toolkit. Backend ownership is an opaque
//! payload: a backend that owns device resources installs an
//! [`DeviceImagePayload`], and every consumer re-checks the payload's backend
//! identity before touching it, so a carrier cannot cross a backend or device
//! boundary silently. A carrier without a payload is host-side, which is the
//! CPU reference execution path under the same domain contracts.

use crate::ComputeBackend;
use spektrafilm_math::colorspace::RgbColorSpace;
use spektrafilm_math::image::ImageBuf;
use std::any::Any;
use std::sync::Arc;

/// What the samples in a carrier mean, so a stage cannot reinterpret them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleDomain {
    /// Scene-referred linear samples in `space`'s primaries.
    Linear,
    /// Samples already encoded with `space`'s transfer function.
    Encoded,
    /// Base-10 film or paper channel exposure, rather than RGB samples.
    LogExposure,
    /// Dye density (film or print) rather than RGB.
    Density,
}

/// Arithmetic precision the carrier's samples were produced with. A carrier
/// that left a device pass is f32; the CPU reference may hold f64. The
/// distinction is reported, never used to select a different algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// Binary32 samples (every device pass today).
    F32,
    /// Binary64 samples (CPU f64 reference).
    F64,
}

impl Precision {
    /// Precision of host `Scalar` samples in this build.
    pub fn host() -> Self {
        if cfg!(feature = "precision-f64") {
            Precision::F64
        } else {
            Precision::F32
        }
    }
}

/// Metadata describing retained samples. Independent of any payload, so a
/// backend can construct a carrier from host samples without a device.
#[derive(Debug, Clone, Copy)]
pub struct RetainedImageMeta {
    pub width: u32,
    pub height: u32,
    pub domain: SampleDomain,
    pub precision: Precision,
    /// Output primaries and transfer the samples carry. Borrowed as
    /// `&'static` because every registered space is a process-wide static.
    pub space: &'static RgbColorSpace,
    /// Working pixel pitch in microns, when the route has derived one.
    pub pixel_pitch_um: Option<f64>,
}

/// Opaque backend payload installed on a carrier that owns device samples.
///
/// The payload is the only device-typed thing a carrier may hold, and it is
/// reachable only through this trait. `backend_id` identifies the backend
/// *and* the device instance that produced the samples, so a consumer can
/// refuse a foreign carrier instead of reading another device's memory.
pub trait DeviceImagePayload: Send + Sync + Any {
    /// Identity of the owning backend instance. A payload is consumable only
    /// by a backend reporting the same id.
    fn backend_id(&self) -> u64;
    /// Erased downcast handle for the backend that owns the payload.
    fn as_any(&self) -> &dyn Any;
}

/// One retained computation image: samples plus the metadata a numerical
/// stage needs to consume them, and an optional backend-owned payload.
///
/// Construction is deliberately one-sided: a backend installs its payload,
/// and every other path builds a host carrier. Dropping the carrier drops the
/// backend's last reference to its samples, so device lifetime follows the
/// route's own lifetime.
#[derive(Clone)]
pub struct DeviceImage {
    meta: RetainedImageMeta,
    payload: Option<Arc<dyn DeviceImagePayload>>,
    host: Option<Arc<ImageBuf>>,
}

impl DeviceImage {
    /// A host-side carrier: the CPU reference execution path.
    pub fn host(image: ImageBuf, meta: RetainedImageMeta) -> Self {
        debug_assert_eq!(meta.width, image.width);
        debug_assert_eq!(meta.height, image.height);
        Self {
            meta,
            payload: None,
            host: Some(Arc::new(image)),
        }
    }

    /// A backend-owned carrier. `meta` must describe the payload's samples.
    pub fn retained(meta: RetainedImageMeta, payload: Arc<dyn DeviceImagePayload>) -> Self {
        Self {
            meta,
            payload: Some(payload),
            host: None,
        }
    }

    pub fn width(&self) -> u32 {
        self.meta.width
    }

    pub fn height(&self) -> u32 {
        self.meta.height
    }

    pub fn pixel_count(&self) -> usize {
        self.meta.width as usize * self.meta.height as usize
    }

    pub fn domain(&self) -> SampleDomain {
        self.meta.domain
    }

    pub fn precision(&self) -> Precision {
        self.meta.precision
    }

    pub fn space(&self) -> &'static RgbColorSpace {
        self.meta.space
    }

    pub fn pixel_pitch_um(&self) -> Option<f64> {
        self.meta.pixel_pitch_um
    }

    pub fn meta(&self) -> RetainedImageMeta {
        self.meta
    }

    /// True when a backend owns these samples.
    pub fn is_retained(&self) -> bool {
        self.payload.is_some()
    }

    /// The backend payload, when this carrier is backend-owned.
    pub fn payload(&self) -> Option<&Arc<dyn DeviceImagePayload>> {
        self.payload.as_ref()
    }

    /// Borrow the backend payload, or report that this is a host carrier.
    pub fn payload_ref(&self) -> Result<&Arc<dyn DeviceImagePayload>, String> {
        self.payload
            .as_ref()
            .ok_or_else(|| "image is not device-retained (host samples)".to_owned())
    }

    /// The host samples, when this carrier holds them.
    pub fn host_image(&self) -> Option<&ImageBuf> {
        self.host.as_deref()
    }

    /// Explicit host materialization: the carrier's samples on the host.
    ///
    /// This is the permitted final handoff — a native writer, a diagnostic
    /// tap, or a CPU reference run — not an internal stage boundary. A
    /// backend-owned carrier materializes through `backend`, so the samples
    /// cross the device boundary once, at the boundary that asked for them.
    pub fn to_host(&self, backend: &dyn ComputeBackend) -> Result<ImageBuf, String> {
        if let Some(image) = self.host.as_deref() {
            return Ok(image.clone());
        }
        backend.materialize_retained(self)
    }

    /// Take the host samples out of a host carrier, leaving nothing behind.
    /// Fails for a backend-owned carrier, which has no host samples to take.
    pub fn into_host(self) -> Result<ImageBuf, String> {
        self.host
            .map(|image| Arc::try_unwrap(image).unwrap_or_else(|arc| (*arc).clone()))
            .ok_or_else(|| "device-retained image has no host samples".to_owned())
    }

    /// Same metadata, different samples: a stage that replaces the payload
    /// (a device pass) keeps the domain contract it was given.
    pub fn with_meta(&self, meta: RetainedImageMeta) -> Self {
        Self {
            meta,
            payload: self.payload.clone(),
            host: self.host.clone(),
        }
    }
}

impl std::fmt::Debug for DeviceImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceImage")
            .field("width", &self.meta.width)
            .field("height", &self.meta.height)
            .field("domain", &self.meta.domain)
            .field("precision", &self.meta.precision)
            .field("space", &self.meta.space.name)
            .field("pixel_pitch_um", &self.meta.pixel_pitch_um)
            .field("retained", &self.payload.is_some())
            .finish()
    }
}
