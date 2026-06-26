pub mod edit_picture;
pub mod ml;
pub mod thumbnail;

use crate::backend::BackendClient;
use crate::config::Config;
use crate::observability;
use archypix_common::job::JobConfig;
use archypix_common::transfer::ClaimJobResponse;
use opentelemetry::trace::TraceContextExt;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::time::{Duration, sleep};
use tracing::Instrument;
use tracing::{error, info, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Poll one backend for jobs, competing with other backend loops for slots on the shared semaphore.
///
/// When a job is claimed the loop immediately tries for another (no sleep), so burst workloads
/// fill the slot pool as fast as the backend can issue claims.  When idle the loop backs off to
/// `poll_interval_ms`.  On error it backs off to `5 × poll_interval_ms`.
pub async fn run_job_loop(config: Arc<Config>, client: Arc<BackendClient>, sem: Arc<Semaphore>) {
    info!(
        worker_id = %config.worker_id,
        backend = %client.back_domain(),
        poll_interval_ms = config.poll_interval_ms,
        max_concurrent_jobs = config.max_concurrent_jobs,
        job_types = ?config.job_types,
        "job runner started"
    );

    loop {
        // Block until a concurrency slot is free, then poll immediately.
        let permit = sem.clone().acquire_owned().await.expect("semaphore closed");

        match client.claim_next_job().await {
            Ok(None) => {
                drop(permit);
                sleep(Duration::from_millis(config.poll_interval_ms)).await;
            }
            Ok(Some(job)) => {
                let job_id = job.job_id;
                let job_type = job.job_type.clone();
                let client_clone = client.clone();

                tokio::spawn(async move {
                    info!(job_id = %job_id, %job_type, "starting job");
                    dispatch(client_clone.as_ref(), job).await;
                    drop(permit);
                });
                // No sleep — immediately compete for the next slot so burst workloads
                // keep all concurrent slots saturated.
            }
            Err(e) => {
                warn!(error = ?e, backend = %client.back_domain(), "error polling for jobs");
                drop(permit);
                sleep(Duration::from_millis(config.poll_interval_ms * 5)).await;
            }
        }
    }
}

/// Decompose the claim response and dispatch to the appropriate handler.
///
/// Errors are reported to the backend via `fail_job` before returning.
/// The function itself is infallible — callers never need to check its result.
async fn dispatch(client: &BackendClient, job: ClaimJobResponse) {
    let job_id = job.job_id;
    let job_type = job.job_type.clone();
    let claim_token = job.claim_token;
    let presigned_read = job.presigned_read;
    let presigned_writes = job.presigned_writes;
    let mime_type = job.mime_type;
    let trace_context = job.trace_context.clone();

    // Create a **new root** span for this job and **link** it back to the enqueueing trace. A link
    // (not parent-child) is correct because the job is decoupled in time from the enqueue: it may be
    // claimed long after enqueue, and may be claimed more than once (watchdog reset → re-claim), so
    // it is genuinely not the same trace. `otel.name` sets the Jaeger operation name to the job type
    // (e.g. `gen_thumbnail`, `edit_picture`) so operations group by kind of work — bounded
    // cardinality — rather than collapsing into one generic `job` operation.
    let job_span = tracing::info_span!(
        "job",
        "otel.name" = %job_type,
        job_id = %job_id,
        job_type = %job_type,
        picture_id = ?job.picture_id,
    );
    if let Some(ctx_map) = &trace_context {
        let remote_cx = observability::extract_context(ctx_map);
        let remote_sc = remote_cx.span().span_context().clone();
        if remote_sc.is_valid() {
            job_span.add_link(remote_sc);
        }
    }

    async move {
        let result = match job.config {
            JobConfig::GenThumbnail(config) => {
                thumbnail::handle(
                    client,
                    job_id,
                    claim_token,
                    config,
                    presigned_read,
                    presigned_writes,
                    mime_type,
                )
                .await
            }
            JobConfig::EditPicture(config) => {
                edit_picture::handle(
                    client,
                    job_id,
                    claim_token,
                    config,
                    presigned_read,
                    presigned_writes,
                    mime_type,
                )
                .await
            }
            JobConfig::MlStyle | JobConfig::MlPeople | JobConfig::MlGroupLocation => {
                ml::handle_stub(client, job_id, claim_token, job_type).await
            }
        };

        if let Err(ref e) = result {
            let permanent = !e.is_retriable();
            error!(job_id = %job_id, permanent, error = ?e, "job failed");
            if let Err(report_err) = client
                .fail_job(job_id, claim_token, &e.to_string(), permanent)
                .await
            {
                error!(
                    job_id = %job_id,
                    error = ?report_err,
                    "failed to report job failure to backend"
                );
            }
        }
    }
    .instrument(job_span)
    .await
}
