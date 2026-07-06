//! Background-work runtime — the **Routine framework**. The generic core (the [`Routine`] trait,
//! [`RoutineHandle`], the per-key debounce/coalesce/rerun scheduler, [`spawn`], monitoring) was
//! lifted to [`archypix_common::routine`] in feature 23 §8 and is re-exported here; only the concrete
//! backend routines live in the submodules below.
//!
//! See `doc/features/17_unified_routine_framework.md` and `doc/features/23_*`.

pub mod exif_drain;
pub mod job_watchdog;
pub mod pipeline;
pub mod purge_sweep;
pub mod resolver_heartbeat;
pub mod storage_reconcile;
pub mod tag_rename;
pub mod unannounce;

pub use archypix_common::routine::*;
