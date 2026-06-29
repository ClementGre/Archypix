//! Per-user tag service evaluation + reconciliation, plus the announcement step.

use crate::domain::pipeline::PipelineInput;
use crate::domain::tag::{TagPath, TagSource};
use crate::domain::tagging::ServiceConfig;
use crate::infra::error::AppError;
use crate::infra::routine::pipeline::{PipelineRun, announcement, dedup};
use crate::repository::pipeline::{PipelineRepository, PipelineTagAssignment};
use crate::repository::tag::TagRepository;
use crate::repository::tagging::TaggingServiceRepository;
use chrono::Utc;
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

pub async fn run_for_user(run: &PipelineRun<'_>, user_id: Uuid) -> Result<(), AppError> {
    let db = run.db;
    let config = run.config;
    let run_at = Utc::now().naive_utc();

    // ── Content dedup reconcile (feature 11 §5) ─────────────────────────────────
    // Serial-per-user (this loop guarantees it): keep one live survivor per content-hash group,
    // hiding the rest as `content_dedupe`, and rescue-promote when a survivor disappears. Runs first
    // so any visibility change re-dirties the affected rows (set `last_pipeline_run_at = NULL`) and
    // they flow through the tagging + announcement steps below in the same pass.
    dedup::reconcile_for_user(run, user_id).await?;

    // ── Initial / recovery announcements ────────────────────────────────────────
    // Reconcile any `pending_first_announcement` / `errored` share's full coverage (deliver inline,
    // flip to `active` on success). Runs regardless of tagging services / dirty pictures (the share
    // may be empty, or recovering from a failed delivery with no tagging changes).
    announcement::reconcile_pending_and_errored(run, user_id).await?;

    // ── Load services ─────────────────────────────────────────────────────────
    // May be empty: a user with no tagging services but an active future share still needs the
    // announcement diff for new/invalidated pictures, so we do not early-return here.
    let services = TaggingServiceRepository::list_enabled_by_owner(db, user_id).await?;

    // ── Parse each service's config once (§12.1) ──────────────────────────────
    // A service whose stored config fails to parse is skipped (logged) — it never matched anyway.
    let mut configs: HashMap<Uuid, ServiceConfig> = HashMap::new();
    for svc in &services {
        match ServiceConfig::parse(svc.service_type, &svc.config) {
            Ok(cfg) => {
                configs.insert(svc.id, cfg);
            }
            Err(e) => tracing::warn!(
                service_id = %svc.id,
                error = %e,
                "pipeline: service config failed to parse — skipping service"
            ),
        }
    }

    // ── Find dirty pictures ───────────────────────────────────────────────────
    let dirty = PipelineRepository::find_dirty_for_user(db, user_id).await?;
    if dirty.is_empty() {
        return Ok(());
    }

    tracing::debug!(
        user_id = %user_id,
        picture_count = dirty.len(),
        "pipeline: evaluating dirty pictures"
    );

    // ── Process in batches of 100 ─────────────────────────────────────────────
    for chunk in dirty.chunks(100) {
        let picture_ids: Vec<Uuid> = chunk.iter().map(|p| p.id).collect();

        // Load tags and incoming share IDs for the whole batch at once.
        let all_tags = TagRepository::list_for_pictures(db, &picture_ids).await?;
        let share_ids_by_pic =
            PipelineRepository::find_incoming_share_ids(db, &picture_ids).await?;

        // Only manual and incoming_share tags form the gating base — pipeline tags are
        // re-derived from scratch each run, so prior-run pipeline tags must not influence
        // `requires`/`excludes` (a stale tag could otherwise keep a service firing).
        let mut base_by_pic: HashMap<Uuid, Vec<TagPath>> = HashMap::new();
        for tag in all_tags {
            if matches!(tag.source, TagSource::Manual | TagSource::IncomingShare) {
                base_by_pic
                    .entry(tag.picture_id)
                    .or_default()
                    .push(TagPath::from_ltree(&tag.tag_path));
            }
        }

        let mut success_ids: Vec<Uuid> = Vec::with_capacity(chunk.len());

        for picture in chunk {
            // Gating tags accumulate this run's pipeline output (in service order) on top of
            // the base, so a downstream service can `require` an upstream service's tag.
            let mut gating_tags: Vec<TagPath> = base_by_pic.remove(&picture.id).unwrap_or_default();
            let incoming_share_ids: &[Uuid] = share_ids_by_pic
                .get(&picture.id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);

            // Complete desired pipeline output for this picture; the reconcile removes any
            // stored pipeline tag absent from it.
            let mut produced: Vec<PipelineTagAssignment> = Vec::new();

            // Camera/lens fields + the derived exposure time, shared across all services.
            let cam = &picture.exif_data.0;
            let exposure_time = match (cam.exposure_time_num, cam.exposure_time_den) {
                (Some(num), Some(den)) if den != 0 => Some(num as f64 / den as f64),
                _ => None,
            };

            for service in &services {
                let input = PipelineInput {
                    picture_id: picture.id,
                    captured_at: picture.captured_at,
                    ingested_at: Some(picture.ingested_at),
                    updated_at: Some(picture.updated_at),
                    gps_lat: picture.gps_lat,
                    gps_lng: picture.gps_lng,
                    gps_alt: picture.gps_alt,
                    filename: picture.filename.clone(),
                    camera_brand: cam.camera_brand.clone(),
                    camera_model: cam.camera_model.clone(),
                    focal_length_mm: cam.focal_length_mm,
                    f_number: cam.f_number,
                    iso_speed: cam.iso_speed,
                    exposure_time,
                    orientation: picture.orientation,
                    mime_type: picture.mime_type.clone(),
                    file_size: picture.file_size,
                    width: picture.width,
                    height: picture.height,
                    is_owned: picture.is_owned,
                };

                if !service.should_run(&gating_tags) {
                    continue;
                }

                let Some(config) = configs.get(&service.id) else {
                    continue; // config failed to parse (already logged)
                };
                let result = config.evaluate(&input, incoming_share_ids);
                let source = config.source().as_str();

                // Keep only the deepest tags this service emits (its own minimal form); a
                // shallower tag from a *different* source is kept independently by the reconcile.
                for tag in TagPath::fold_deepest(result.tags_to_add) {
                    if !gating_tags.iter().any(|t| t == &tag) {
                        gating_tags.push(tag.clone());
                    }
                    produced.push(PipelineTagAssignment {
                        tag_path: tag.as_ltree().to_string(),
                        source: source.to_string(),
                        source_id: service.id,
                    });
                }
            }

            // Atomically add the produced tags and remove stale pipeline tags.
            if let Err(e) =
                PipelineRepository::reconcile_pipeline_tags(db, picture.id, &produced).await
            {
                tracing::error!(
                    picture_id = %picture.id,
                    error = ?e,
                    "pipeline: failed to reconcile tags — picture will be retried"
                );
                continue; // do not mark this picture as run
            }

            success_ids.push(picture.id);
        }

        // Mark successfully processed pictures so they aren't re-evaluated unnecessarily.
        if let Err(e) = PipelineRepository::mark_run(db, &success_ids, run_at).await {
            tracing::error!(error = ?e, "pipeline: failed to mark pictures as run");
        }

        // Diff share coverage against the tracking table and deliver (un)announce inline.
        if let Err(e) = announcement::reconcile_active_batch(run, user_id, &success_ids).await {
            tracing::error!(error = ?e, "pipeline: announcement step failed for batch");
        }

        // Optional backpressure between batches.
        if config.pipeline_batch_sleep_ms > 0 {
            tokio::time::sleep(Duration::from_millis(config.pipeline_batch_sleep_ms)).await;
        }
    }

    tracing::debug!(user_id = %user_id, "pipeline: sweep complete for user");
    Ok(())
}
