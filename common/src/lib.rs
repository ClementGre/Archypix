pub mod hash;
pub mod job;
pub mod mime;
pub mod serde_utils;
pub mod transfer;

// ── Feature 23 shared modules (feature-gated so lean consumers like the worker opt in) ──────────
#[cfg(feature = "auth")]
pub mod auth;
#[cfg(feature = "error")]
pub mod error;
#[cfg(feature = "registration")]
pub mod registration;
#[cfg(feature = "routine")]
pub mod routine;
#[cfg(feature = "settings")]
pub mod settings;
