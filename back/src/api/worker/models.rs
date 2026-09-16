// Re-export the shared wire types so API code can use `models::ClaimJobResponse` etc.
// without long `archypix_common::transfer::` paths.
pub use archypix_common::transfer::{
    ClaimJobResponse, ExifExtraction, JobOutcome, JobProduct, JobResponse, PictureWork,
};
