//! `archypix-resolver` library crate — exposes the modules so the binary (`main.rs`) and the
//! integration tests (`tests/`) share one compilation of the module tree.

pub mod api;
pub mod clients;
pub mod config;
pub mod repository;
pub mod routine;
pub mod services;
pub mod state;
