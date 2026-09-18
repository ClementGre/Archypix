use crate::error::Result;
use archypix_common::job::JobType;
use tracing::info;

/// Placeholder for ML-based jobs: succeeds with no product. `dispatch` sends the response.
pub fn handle_stub(job_type: &JobType) -> Result<()> {
    info!(%job_type, "ML job received (not yet implemented); marking complete");
    Ok(())
}
