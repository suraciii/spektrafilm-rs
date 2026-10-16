//! Crate-wide error type: machine-readable code + verbatim human message.
//!
//! Shape aligns with `spektrafilm_benchmark::common::Error` (`code`/`message`).
//! All existing error messages are preserved verbatim in `message`;
//! `code` categorizes errors for upstream handling (retry, fix param, report bug).

use std::fmt;

/// Crate-wide error: machine-readable code + verbatim human message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpektraError {
    pub code: SpektraErrorCode,
    pub message: String,
}

/// Error code categories for `SpektraError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SpektraErrorCode {
    /// Operation was cancelled.
    Cancelled,
    /// Filesystem/stream/lock/serialization write failure.
    Io,
    /// JSON/TOML/inline syntax/container format decode failure.
    Parse,
    /// Leaf value invalid (path-prefixed message).
    InvalidParam,
    /// Parameter path/field/group does not exist or is empty.
    UnknownParam,
    /// Referenced resource missing (wire/LUT/color space/preset…).
    NotFound,
    /// Input valid but unsupported in current context (algorithm/topology/GPU capability/env switch).
    Unsupported,
    /// Non-finite value, overflow, singular, unrepresentable, out of bounds.
    Numeric,
    /// Cross-field state precondition not satisfied.
    Precondition,
    /// Duplicate/legacy field collision.
    Conflict,
    /// CLI argument syntax error.
    Usage,
    /// Cache poisoning, backend without error, other invariant violations.
    Internal,
}

impl SpektraErrorCode {
    /// Stable snake_case string for JSON output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Io => "io",
            Self::Parse => "parse",
            Self::InvalidParam => "invalid_param",
            Self::UnknownParam => "unknown_param",
            Self::NotFound => "not_found",
            Self::Unsupported => "unsupported",
            Self::Numeric => "numeric",
            Self::Precondition => "precondition",
            Self::Conflict => "conflict",
            Self::Usage => "usage",
            Self::Internal => "internal",
        }
    }
}

impl SpektraError {
    /// Create a new `SpektraError` with the given code and message.
    pub fn new(code: SpektraErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for SpektraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SpektraError {}

/// Convenience alias for `Result<T, SpektraError>`.
pub type SpektraResult<T> = Result<T, SpektraError>;

impl From<String> for SpektraError {
    fn from(message: String) -> Self {
        Self::new(SpektraErrorCode::Internal, message)
    }
}

impl From<&str> for SpektraError {
    fn from(message: &str) -> Self {
        Self::new(SpektraErrorCode::Internal, message)
    }
}

impl From<std::io::Error> for SpektraError {
    fn from(error: std::io::Error) -> Self {
        Self::new(SpektraErrorCode::Io, error.to_string())
    }
}
