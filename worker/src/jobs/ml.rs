use crate::backend::BackendClient;
use crate::error::Result;
use archypix_common::job::JobType;
use archypix_common::transfer::CompleteJobRequest;
use tracing::info;
use uuid::Uuid;

/// Placeholder for ML-based jobs. Logs and reports success with empty result.
#[tracing::instrument(skip(client), fields(job_id = %job_id, job_type = %job_type))]
pub async fn handle_stub(
    client: &BackendClient,
    job_id: Uuid,
    claim_token: Uuid,
    job_type: JobType,
) -> Result<()> {
    info!(
        job_id = %job_id,
        %job_type,
        "ML job received (not yet implemented); marking complete"
    );
    client
        .complete_job(
            job_id,
            CompleteJobRequest {
                claim_token,
                ..Default::default()
            },
        )
        .await
}
