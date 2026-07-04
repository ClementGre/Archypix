use crate::domain::job::{JobStatus, JobType};
use crate::repository::admin::{
    AdminJob, ConsistencyStats, ErroredShare, FederationInstance, InstanceStats, UserStats,
    UserWithStorage,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Request models ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub password: String,
    pub is_admin: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub is_admin: Option<bool>,
    /// Storage quota (feature 22 §8.3). Absent = unchanged; present `null` = unlimited; present
    /// number = cap in bytes. The double-`Option` distinguishes "field omitted" from "set to null".
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub storage_quota_bytes: Option<Option<i64>>,
}

/// serde helper: a present field (value or `null`) deserializes to `Some(...)`, an absent one to
/// `None` — letting a PATCH distinguish "leave unchanged" from "clear to null".
fn deserialize_optional_field<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(de)?))
}

#[derive(Debug, Deserialize)]
pub struct ListJobsQuery {
    pub status: Option<JobStatus>,
    #[serde(rename = "type")]
    pub job_type: Option<JobType>,
    pub user_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

// ── Response models ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AdminUserResponse {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub is_admin: bool,
    pub storage_bytes: i64,
    pub quota_bytes: Option<i64>,
    pub breakdown: crate::repository::user_storage::UserStorage,
    pub usage_ratio: Option<f64>,
}

impl From<UserWithStorage> for AdminUserResponse {
    fn from(u: UserWithStorage) -> Self {
        let usage_ratio = u
            .quota_bytes
            .filter(|q| *q > 0)
            .map(|q| u.storage_bytes as f64 / q as f64);
        Self {
            id: u.id,
            username: u.username,
            email: u.email,
            display_name: u.display_name,
            is_admin: u.is_admin,
            storage_bytes: u.storage_bytes,
            quota_bytes: u.quota_bytes,
            breakdown: crate::repository::user_storage::UserStorage {
                originals_bytes: u.originals_bytes,
                originals_trashed_bytes: u.originals_trashed_bytes,
                versions_bytes: u.versions_bytes,
                versions_trashed_bytes: u.versions_trashed_bytes,
            },
            usage_ratio,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct InstanceHealthResponse {
    pub global_domain: String,
    pub back_domain: String,
    pub db_connected: bool,
    pub redis_connected: bool,
    pub last_worker_activity_at: Option<String>,
}

pub type InstanceStatsResponse = InstanceStats;
pub type UserStatsResponse = UserStats;
pub type AdminJobResponse = AdminJob;
pub type ErroredShareResponse = ErroredShare;
pub type FederationInstanceResponse = FederationInstance;
pub type ConsistencyResponse = ConsistencyStats;
