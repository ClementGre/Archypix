pub mod federation;

use archypix_back::clients::federation::FederationClient;
use archypix_back::clients::resolver::ResolverClient;
use archypix_back::domain::tag::encode_sender_label;
use archypix_back::infra::config::Config;
use archypix_back::infra::crypto::JwtService;
use archypix_back::infra::error::AppError;
use archypix_back::infra::pipeline::PipelineWaker;
use archypix_back::infra::redis::{Cache, RedisKey};
use archypix_back::infra::s3::Storage;
use archypix_back::infra::tasks;
use archypix_back::state::AppState;
use async_trait::async_trait;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// ── InMemoryCache ─────────────────────────────────────────────────────────────

pub struct InMemoryCache {
    store: Mutex<HashMap<String, String>>,
}

impl InMemoryCache {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl Cache for InMemoryCache {
    async fn get_str(&self, key: RedisKey<'_>) -> Result<Option<String>, AppError> {
        Ok(self.store.lock().unwrap().get(&key.build()).cloned())
    }

    async fn set_str_ex(
        &self,
        key: RedisKey<'_>,
        value: &str,
        _ttl_secs: u64,
    ) -> Result<(), AppError> {
        self.store
            .lock()
            .unwrap()
            .insert(key.build(), value.to_string());
        Ok(())
    }

    async fn del(&self, key: RedisKey<'_>) -> Result<(), AppError> {
        self.store.lock().unwrap().remove(&key.build());
        Ok(())
    }

    async fn scan_keys(&self, pattern: &str) -> Result<Vec<String>, AppError> {
        Ok(self
            .store
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.contains(pattern))
            .cloned()
            .collect::<Vec<_>>()
            .into())
    }
}

// ── MockStorage ───────────────────────────────────────────────────────────────

pub struct MockStorage;

#[async_trait]
impl Storage for MockStorage {
    async fn presign_get(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        Ok(format!("http://mock-s3/{bucket}/{key}?sig=get"))
    }
    async fn presign_put(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        Ok(format!("http://mock-s3/{bucket}/{key}?sig=put"))
    }
    async fn presign_get_worker(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        Ok(format!("http://mock-s3-worker/{bucket}/{key}?sig=get"))
    }
    async fn presign_put_worker(&self, bucket: &str, key: &str) -> Result<String, AppError> {
        Ok(format!("http://mock-s3-worker/{bucket}/{key}?sig=put"))
    }
    async fn copy_object(
        &self,
        _src_bucket: &str,
        _src_key: &str,
        _dst_bucket: &str,
        _dst_key: &str,
    ) -> Result<(), AppError> {
        Ok(())
    }
    async fn delete_object(&self, _bucket: &str, _key: &str) -> Result<(), AppError> {
        Ok(())
    }

    async fn get_object(&self, bucket: &str, key: &str) -> Result<Vec<u8>, AppError> {
        todo!()
    }

    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<(), AppError> {
        todo!()
    }
}

// ── Federation helper ─────────────────────────────────────────────────────────

/// Build a FederationClient backed by a fresh InMemoryCache.
/// Returns the client and its underlying cache so tests can inspect/mutate it.
pub fn make_federation(config: &Config) -> (FederationClient, Arc<InMemoryCache>) {
    let cache = Arc::new(InMemoryCache::new());
    let fed = FederationClient::new(
        reqwest::Client::new(),
        config.clone(),
        JwtService::new(&config.jwt_secret, &config.back_domain),
        cache.clone(),
    );
    (fed, cache)
}

/// Build a `TaskQueue` (with its runner spawned) and a `PipelineWaker` for tests that exercise
/// share announce/unannounce delivery. The runner executes same-backend tasks against the DB;
/// cross-instance tasks attempt HTTP and fail harmlessly (errors are logged). The waker is
/// disconnected — tests drive the pipeline explicitly via `run_once_for_user`.
pub fn test_task_queue(db: &PgPool, config: &Config) -> (tasks::TaskQueue, PipelineWaker) {
    let (fed, _cache) = make_federation(config);
    test_task_queue_with_federation(db, config, fed)
}

/// Like [`test_task_queue`] but uses the supplied `FederationClient` — pass one that shares a
/// server's cache (e.g. from `federation::make_client`) so cross-instance announce/unannounce
/// delivery can resolve the remote backend and reuse granted tokens.
pub fn test_task_queue_with_federation(
    db: &PgPool,
    config: &Config,
    federation: FederationClient,
) -> (tasks::TaskQueue, PipelineWaker) {
    let waker = PipelineWaker::disconnected();
    let (queue, runner) = tasks::create(db.clone(), federation, config.clone(), waker.clone(), 1);
    tokio::spawn(runner);
    (queue, waker)
}

// ── Full AppState helper ──────────────────────────────────────────────────────

/// Build a test `AppState` with an externally supplied `cache`.
///
/// Useful when the test needs to inspect or pre-seed the cache before and after
/// requests (e.g., federation contract tests where backend URLs are pre-seeded
/// so WebFinger calls are bypassed).
pub fn test_app_state_with_cache(db: PgPool, config: &Config, cache: Arc<dyn Cache>) -> AppState {
    let storage: Arc<dyn Storage> = Arc::new(MockStorage);
    let jwt = JwtService::new(&config.jwt_secret, &config.back_domain);
    let worker_jwt = JwtService::new(&config.worker_jwt_secret, &config.back_domain);
    let resolver_jwt = JwtService::new(&config.resolver_jwt_secret, &config.back_domain);

    let federation = FederationClient::new(
        reqwest::Client::new(),
        config.clone(),
        jwt.clone(),
        cache.clone(),
    );
    let resolver = ResolverClient::new(reqwest::Client::new(), config.clone(), resolver_jwt);

    let pipeline_waker = PipelineWaker::disconnected();
    let (task_queue, runner) = tasks::create(
        db.clone(),
        federation.clone(),
        config.clone(),
        pipeline_waker.clone(),
        1,
    );
    // Run the task runner so same-backend / cross-instance announce-unannounce tasks execute.
    tokio::spawn(runner);

    AppState::new(
        config.clone(),
        db,
        cache,
        jwt,
        worker_jwt,
        storage,
        federation,
        resolver,
        task_queue,
        pipeline_waker,
    )
}

/// Build a test `AppState` with a fresh `InMemoryCache`.
///
/// Uses `MockStorage` (no S3). The task-queue runner is dropped immediately —
/// tasks submitted during tests are silently ignored.
pub fn test_app_state(db: PgPool, config: &Config) -> AppState {
    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
    test_app_state_with_cache(db, config, cache)
}

// ── DB helpers ────────────────────────────────────────────────────────────────

pub async fn seed_user(db: &PgPool, username: &str, password: &str) -> Uuid {
    archypix_back::services::users::create_user(
        db,
        username,
        &format!("{username}@test.com"),
        username,
        password,
        false,
    )
    .await
    .unwrap()
    .id
}

/// Insert a bare picture row for `user_id` and return its ID.
pub async fn seed_picture(db: &PgPool, user_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO pictures (id, local_user_id) VALUES ($1, $2)",
        id,
        user_id,
    )
    .execute(db)
    .await
    .unwrap();
    id
}

/// Insert a picture for `user_id` and assign it `tag` (ltree format, e.g. `"vacation"`).
pub async fn seed_picture_with_tag(db: &PgPool, user_id: Uuid, tag: &str) -> Uuid {
    use archypix_back::repository::tag::TagRepository;
    let pic_id = seed_picture(db, user_id).await;
    TagRepository::batch_assign(db, user_id, &[pic_id], &[tag.to_string()])
        .await
        .unwrap();
    pic_id
}

/// Count received (non-owned) picture rows for `user_id`.
pub async fn count_received_pictures(db: &PgPool, user_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM pictures WHERE local_user_id = $1 AND remote_picture_id IS NOT NULL",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
    .unwrap()
}

/// Return all tag paths on all received pictures for `user_id`.
pub async fn received_picture_tags(db: &PgPool, user_id: Uuid) -> Vec<String> {
    let rows: Vec<Option<String>> = sqlx::query_scalar(
        r#"SELECT t.tag_path::text
           FROM tags t
           JOIN pictures p ON p.id = t.picture_id
           WHERE p.local_user_id = $1 AND p.remote_picture_id IS NOT NULL"#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await
    .unwrap();
    rows.into_iter().flatten().collect()
}

/// The ltree path of a SharedToMe tag for a share from `sender@sender_instance` of `shared_tag`.
pub fn shared_to_me_tag(sender_username: &str, sender_instance: &str, shared_tag: &str) -> String {
    let label = encode_sender_label(sender_username, sender_instance);
    if shared_tag.is_empty() {
        format!("SharedToMe.{label}")
    } else {
        format!("SharedToMe.{label}.{shared_tag}")
    }
}
