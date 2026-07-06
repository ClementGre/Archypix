//! Instance-selection strategies (feature 23 §7). Metric strategies read the heartbeat-reported
//! counts (authoritative over the drift-prone mapping count). Capacity + reachability are **hard**
//! gates (a full/closed/unreachable backend is never chosen, even when pinned). An invite's
//! `instance_pin` is a suggestion weighted by `pin_importance`.

use crate::config::{Config, SelectionStrategy, setting_keys as sk};
use crate::repository::{self, Backend};
use archypix_common::error::AppError;
use sqlx::PgPool;

fn metric(strategy: SelectionStrategy, b: &Backend) -> i64 {
    match strategy {
        SelectionStrategy::LeastPictures => b.picture_count,
        SelectionStrategy::LeastStorage => b.storage_bytes,
        _ => b.user_count, // LeastUsers + fallback for non-metric strategies
    }
}

/// Choose an eligible backend for a new user, honouring `instance_pin` per `pin_importance`.
/// Returns `503` when no backend is eligible.
pub async fn pick_backend(
    db: &PgPool,
    config: &Config,
    instance_pin: Option<&str>,
) -> Result<Backend, AppError> {
    let eligible: Vec<Backend> = repository::list_backends(db)
        .await?
        .into_iter()
        .filter(Backend::is_eligible)
        .collect();
    if eligible.is_empty() {
        return Err(AppError::ServiceUnavailable(
            "no backend is eligible for registration (all full, closed, or unreachable)"
                .to_string(),
        ));
    }

    let strategy = config.get(sk::SELECTION_STRATEGY);
    let importance = config.get(sk::PIN_IMPORTANCE);
    let pinned = instance_pin.and_then(|p| eligible.iter().find(|b| b.back_domain == p));

    let chosen: &Backend = match strategy {
        SelectionStrategy::RoundRobin => {
            // Follow the pin iff importance ≥ 1; else least-recently-selected (never-selected first).
            match pinned {
                Some(b) if importance >= 1 => b,
                _ => eligible.iter().min_by_key(|b| b.last_selected_at).unwrap(),
            }
        }
        SelectionStrategy::Static => {
            let static_backend = config.get(sk::STATIC_BACKEND);
            let target = match (pinned, importance >= 1) {
                (Some(b), true) => Some(b),
                _ => static_backend
                    .as_deref()
                    .and_then(|d| eligible.iter().find(|b| b.back_domain == d)),
            };
            target.unwrap_or(&eligible[0])
        }
        metric_strategy => {
            let best = eligible
                .iter()
                .min_by_key(|b| metric(metric_strategy, b))
                .unwrap();
            match pinned {
                // Honour the pin iff metric(pinned) − min(others) ≤ pin_importance (§7.2).
                Some(p)
                if metric(metric_strategy, p) - metric(metric_strategy, best) <= importance =>
                    {
                        p
                    }
                _ => best,
            }
        }
    };

    let chosen = chosen.clone();
    repository::touch_selected(db, &chosen.back_domain).await?;
    Ok(chosen)
}
