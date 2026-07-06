//! `AppError` now lives in `archypix_common::error` (feature 23 §8/9 follow-up), shared with the
//! resolver. Re-exported here so existing `archypix_common::error::AppError` imports keep working.
pub use archypix_common::error::{map_sqlx_error, AppError};
