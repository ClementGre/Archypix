use crate::error::Result;
use archypix_common::job::JobType;
use tracing::info;
use uuid::Uuid;

/// Placeholder for ML-based jobs: succeeds with no product. `dispatch` sends the response.
pub fn handle_stub(job_id: Uuid, job_type: &JobType) -> Result<()> {
    info!(job_id = %job_id, %job_type, "ML job received (not yet implemented); marking complete");
    Ok(())
}
