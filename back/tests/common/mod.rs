pub mod federation;

use archypix_back::clients::federation::FederationClient;
use archypix_back::clients::resolver::ResolverClient;
use archypix_back::domain::tag::encode_sender_label;
use archypix_back::domain::validation::MIN_PASSWORD_LEN;
use archypix_back::infra::crypto::JwtService;
use archypix_back::infra::redis::{Cache, RedisKey};
use archypix_back::infra::routine::unannounce::{UnannounceInput, UnannounceRoutine};
use archypix_back::infra::routine::{self, RoutineHandle};
use archypix_back::infra::s3::Storage;
use archypix_back::infra::settings::keys;
use archypix_back::state::{AppState, RoutineRegistry, Routines};
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
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

    async fn set_str_nx_ex(
        &self,
        key: RedisKey<'_>,
        value: &str,
        _ttl_secs: u64,
    ) -> Result<bool, AppError> {
        use std::collections::hash_map::Entry;
        match self.store.lock().unwrap().entry(key.build()) {
            Entry::Occupied(_) => Ok(false),
            Entry::Vacant(e) => {
                e.insert(value.to_string());
                Ok(true)
            }
        }
    }

    async fn scan_keys(&self, pattern: &str) -> Result<Vec<String>, AppError> {
        // Support the trailing-`*` glob used in production (`prefix:*` → prefix match).
        let prefix = pattern.strip_suffix('*').unwrap_or(pattern);
        Ok(self
            .store
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect::<Vec<_>>())
    }

    async fn incr_ex(&self, key: RedisKey<'_>, _ttl_secs: u64) -> Result<u64, AppError> {
        let mut store = self.store.lock().unwrap();
        let k = key.build();
        let next = store
            .get(&k)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        store.insert(k, next.to_string());
        Ok(next)
    }

    async fn sum_int_by_prefix(&self, prefix: &str) -> Result<i64, AppError> {
        Ok(self
            .store
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .filter_map(|(_, v)| v.parse::<i64>().ok())
            .sum())
    }
}

// ── MockStorage ───────────────────────────────────────────────────────────────

/// In-memory object store keyed by `bucket/key`. Enough to exercise the WebDAV write/read
/// taxonomy (upload, overwrite, version snapshot copy, proxy read) without a real S3.
#[derive(Default)]
pub struct MockStorage {
    objects: Mutex<HashMap<String, Vec<u8>>>,
}

impl MockStorage {
    pub fn new() -> Self {
        Self::default()
    }
    fn obj_key(bucket: &str, key: &str) -> String {
        format!("{bucket}/{key}")
    }
    /// Test helper: bytes currently stored at `bucket/key`, if any.
    pub fn get(&self, bucket: &str, key: &str) -> Option<Vec<u8>> {
        self.objects
            .lock()
            .unwrap()
            .get(&Self::obj_key(bucket, key))
            .cloned()
    }
}

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
        src_bucket: &str,
        src_key: &str,
        dst_bucket: &str,
        dst_key: &str,
    ) -> Result<(), AppError> {
        let mut store = self.objects.lock().unwrap();
        if let Some(bytes) = store.get(&Self::obj_key(src_bucket, src_key)).cloned() {
            store.insert(Self::obj_key(dst_bucket, dst_key), bytes);
        }
        Ok(())
    }
    async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), AppError> {
        self.objects
            .lock()
            .unwrap()
            .remove(&Self::obj_key(bucket, key));
        Ok(())
    }

    async fn get_object(&self, bucket: &str, key: &str) -> Result<Vec<u8>, AppError> {
        self.objects
            .lock()
            .unwrap()
            .get(&Self::obj_key(bucket, key))
            .cloned()
            .ok_or(AppError::NotFound)
    }

    async fn object_size(&self, bucket: &str, key: &str) -> Result<i64, AppError> {
        self.objects
            .lock()
            .unwrap()
            .get(&Self::obj_key(bucket, key))
            .map(|b| b.len() as i64)
            .ok_or(AppError::NotFound)
    }

    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        _content_type: Option<&str>,
    ) -> Result<(), AppError> {
        self.objects
            .lock()
            .unwrap()
            .insert(Self::obj_key(bucket, key), body);
        Ok(())
    }

    async fn put_object_file(
        &self,
        bucket: &str,
        key: &str,
        path: &std::path::Path,
        _content_type: Option<&str>,
    ) -> Result<(), AppError> {
        let body = std::fs::read(path)
            .map_err(|e| AppError::InternalServerError(format!("read temp file: {e}")))?;
        self.objects
            .lock()
            .unwrap()
            .insert(Self::obj_key(bucket, key), body);
        Ok(())
    }

    async fn prefix_usage(
        &self,
        bucket: &str,
        prefix: &str,
    ) -> Result<archypix_back::infra::s3::PrefixUsage, AppError> {
        let full = format!("{bucket}/{prefix}");
        let store = self.objects.lock().unwrap();
        let mut usage = archypix_back::infra::s3::PrefixUsage::default();
        for (k, v) in store.iter() {
            if k.starts_with(&full) {
                usage.object_count += 1;
                usage.total_bytes += v.len() as i64;
            }
        }
        Ok(usage)
    }
}

// ── Federation helper ─────────────────────────────────────────────────────────

/// Build a FederationClient backed by a fresh InMemoryCache.
/// Returns the client and its underlying cache so tests can inspect/mutate it.
pub fn make_federation(settings: &Arc<Settings>) -> (FederationClient, Arc<InMemoryCache>) {
    let cache = Arc::new(InMemoryCache::new());
    let fed = FederationClient::new(
        reqwest::Client::new(),
        settings.clone(),
        JwtService::new(
            &settings.get(keys::JWT_SECRET),
            &settings.get(keys::BACK_DOMAIN),
        ),
        cache.clone(),
    );
    (fed, cache)
}

/// Build an `Unannounce` routine handle (with its runtime spawned) and a disconnected pipeline
/// handle for tests that exercise share announce/unannounce delivery. The routine executes
/// same-backend unannounces against the DB; cross-instance ones attempt HTTP and fail harmlessly
/// (errors are logged). The pipeline handle is disconnected — tests drive the pipeline explicitly
/// via `run_once_for_user`.
pub fn test_task_queue(
    db: &PgPool,
    settings: &Arc<Settings>,
) -> (RoutineHandle<UnannounceInput>, RoutineHandle<Uuid>) {
    let (fed, _cache) = make_federation(settings);
    test_task_queue_with_federation(db, settings, fed)
}

/// Like [`test_task_queue`] but uses the supplied `FederationClient` — pass one that shares a
/// server's cache (e.g. from `federation::make_client`) so cross-instance announce/unannounce
/// delivery can resolve the remote backend and reuse granted tokens.
pub fn test_task_queue_with_federation(
    db: &PgPool,
    settings: &Arc<Settings>,
    federation: FederationClient,
) -> (RoutineHandle<UnannounceInput>, RoutineHandle<Uuid>) {
    let pipeline = RoutineHandle::<Uuid>::disconnected();
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (queue, runtime) = routine::spawn(
        UnannounceRoutine::new(db.clone(), federation, settings.clone(), pipeline.clone()),
        shutdown_rx,
    );
    // Keep the runtime alive for the test process and let same-backend / cross-instance unannounces run.
    std::mem::forget(_shutdown_tx);
    tokio::spawn(runtime);
    (queue, pipeline)
}

// ── Full AppState helper ──────────────────────────────────────────────────────

/// Build a test `AppState` with an externally supplied `cache`.
///
/// Useful when the test needs to inspect or pre-seed the cache before and after
/// requests (e.g., federation contract tests where backend URLs are pre-seeded
/// so resolver calls are bypassed).
pub fn test_app_state_with_cache(
    db: PgPool,
    settings: &Arc<Settings>,
    cache: Arc<dyn Cache>,
) -> AppState {
    test_app_state_with_storage(db, settings, cache, Arc::new(MockStorage::new()))
}

/// Like [`test_app_state_with_cache`] but with a caller-supplied storage backend, so a test can
/// hold an `Arc<MockStorage>` and inspect the bytes written (used by the WebDAV VFS tests).
pub fn test_app_state_with_storage(
    db: PgPool,
    settings: &Arc<Settings>,
    cache: Arc<dyn Cache>,
    storage: Arc<dyn Storage>,
) -> AppState {
    let jwt = JwtService::new(
        &settings.get(keys::JWT_SECRET),
        &settings.get(keys::BACK_DOMAIN),
    );
    let worker_jwt = JwtService::new(
        &settings.get(keys::WORKER_JWT_SECRET),
        &settings.get(keys::BACK_DOMAIN),
    );
    let resolver_jwt = JwtService::new(
        &settings.get(keys::RESOLVER_JWT_SECRET).unwrap_or_default(),
        &settings.get(keys::BACK_DOMAIN),
    );

    let federation = FederationClient::new(
        reqwest::Client::new(),
        settings.clone(),
        jwt.clone(),
        cache.clone(),
    );
    let resolver = ResolverClient::new(
        reqwest::Client::new(),
        settings.clone(),
        resolver_jwt,
        jwt.clone(),
    );

    // Run the unannounce routine so same-backend / cross-instance unannounce triggers execute.
    let (unannounce, pipeline) = test_task_queue_with_federation(&db, settings, federation.clone());
    let routines = Routines {
        pipeline,
        exif_drain: RoutineHandle::disconnected(),
        tag_rename: RoutineHandle::disconnected(),
        unannounce,
    };

    AppState::new(
        settings.clone(),
        db,
        cache,
        jwt,
        worker_jwt,
        storage,
        federation,
        resolver,
        routines,
        RoutineRegistry::empty(),
    )
}

/// Build a test `AppState` with a fresh `InMemoryCache`.
///
/// Uses `MockStorage` (no S3). The task-queue runner is dropped immediately —
/// tasks submitted during tests are silently ignored.
pub fn test_app_state(db: PgPool, settings: &Arc<Settings>) -> AppState {
    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
    test_app_state_with_cache(db, settings, cache)
}

// ── DB helpers ────────────────────────────────────────────────────────────────

pub async fn seed_user(db: &PgPool, username: &str, password: &str) -> Uuid {
    // Pad short fixture passwords so they satisfy the production password policy. Login tests use
    // their own (already-compliant) passwords, so this only affects users that are never logged in.
    let password = if password.chars().count() < MIN_PASSWORD_LEN {
        format!("{password}_seedpw01")
    } else {
        password.to_string()
    };
    archypix_back::services::users::create_user(
        db,
        username,
        &format!("{username}@test.com"),
        username,
        &password,
        false,
        None,
        None,
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
