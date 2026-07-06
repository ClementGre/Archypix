//! JWT claims + token taxonomy. Lifted to `archypix_common::auth` (feature 23 §9); re-exported here
//! so existing `crate::domain::auth::{JwtClaims, TokenType}` imports keep working.
pub use archypix_common::auth::{JwtClaims, TokenType};
