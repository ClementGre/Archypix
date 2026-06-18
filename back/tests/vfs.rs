//! End-to-end tests for the WebDAV `VirtualFs` over a hierarchy (06_webdav.md §18).
//!
//! These drive `services::vfs::Vfs` directly against a seeded Postgres DB and an in-memory
//! `MockStorage`, covering list/stat/read and the full write taxonomy (PUT new/overwrite/
//! dedupe/un-delete, MOVE/COPY/DELETE), versioning-on-overwrite, and case-insensitive tag reuse.

mod common;

use archypix_back::domain::user_settings::VersioningMode;
use archypix_back::infra::config::Config;
use archypix_back::infra::s3;
use archypix_back::repository::picture::PictureRepository;
use archypix_back::repository::picture_version::PictureVersionRepository;
use archypix_back::repository::tag::TagRepository;
use archypix_back::repository::user_settings::UserSettingsRepository;
use archypix_back::services::hierarchy;
use archypix_back::services::vfs::{ReadTarget, Vfs};
use archypix_back::state::AppState;
use common::MockStorage;
use sqlx::PgPool;
use std::io::Write;
use std::sync::Arc;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn config() -> Config {
    Config::test_defaults()
}

/// Build an `AppState` whose storage is an inspectable `MockStorage`.
fn state_with_storage(db: PgPool) -> (AppState, Arc<MockStorage>) {
    let cfg = config();
    let storage = Arc::new(MockStorage::new());
    let cache: Arc<dyn archypix_back::infra::redis::Cache> = Arc::new(common::InMemoryCache::new());
    let dyn_storage: Arc<dyn archypix_back::infra::s3::Storage> = storage.clone();
    let state = common::test_app_state_with_storage(db, &cfg, cache, dyn_storage);
    (state, storage)
}

/// Insert a fully-populated owned picture (filename, mime, size, hash) with `bytes` stored in the
/// pictures bucket, assign `tag`, and return its id.
async fn seed_full_picture(
    state: &AppState,
    user: Uuid,
    filename: &str,
    mime: &str,
    bytes: &[u8],
    tag: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    let hash = archypix_common::hash::hash_bytes(&bytes.to_vec()).unwrap();
    sqlx::query!(
        "INSERT INTO pictures (id, local_user_id, filename, mime_type, file_size, file_hash) \
         VALUES ($1, $2, $3, $4, $5, $6)",
        id,
        user,
        filename,
        mime,
        bytes.len() as i64,
        hash,
    )
    .execute(&state.db)
    .await
    .unwrap();
    TagRepository::batch_assign(&state.db, user, &[id], &[tag.to_string()])
        .await
        .unwrap();
    state
        .storage
        .put_object(
            &state.config.s3_bucket_pictures,
            &s3::picture_key(user, id),
            bytes.to_vec(),
            Some(mime),
        )
        .await
        .unwrap();
    id
}

fn seg(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// PUT helper: stream `bytes` to a temp file (as the handler does), hash it, and call `put_file`.
async fn put(
    vfs: &Vfs<'_>,
    segments: &[&str],
    bytes: &[u8],
    ct: Option<&str>,
) -> Result<bool, archypix_back::infra::error::AppError> {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    tmp.write_all(bytes).unwrap();
    tmp.flush().unwrap();
    let path = tmp.path().to_path_buf();
    let hash = archypix_common::hash::hash_file(&path).unwrap();
    vfs.put_file(&seg(segments), &path, &hash, bytes.len() as i64, ct)
        .await
}

/// A mirror of `Photos` (keepDir=true) plus a writable `query` node `Lower` used for the
/// case-fold test. `safeDeleteMode` lets each test pick its delete semantics.
fn mirror_config(safe_delete: &str) -> serde_json::Value {
    serde_json::json!({
        "safeDeleteMode": safe_delete,
        "nodes": [
            {"id": "n1", "kind": "mirror", "name": "Photos", "tagRoot": "Photos", "keepDir": true}
        ]
    })
}

async fn make_hierarchy(db: &PgPool, user: Uuid, cfg: serde_json::Value) -> Uuid {
    hierarchy::create_hierarchy(db, user, "Photos", &cfg)
        .await
        .unwrap()
        .id
}

async fn tags_of(db: &PgPool, user: Uuid, pic: Uuid) -> Vec<String> {
    let mut paths: Vec<String> = TagRepository::list_for_picture(db, user, pic)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.tag_path)
        .collect();
    paths.sort();
    paths
}

/// The owned, non-deleted picture currently carrying `hash`.
async fn pic_by_hash(
    db: &PgPool,
    user: Uuid,
    bytes: &[u8],
) -> archypix_back::domain::picture::Picture {
    let hash = archypix_common::hash::hash_bytes(&bytes.to_vec()).unwrap();
    PictureRepository::find_owned_by_hash(db, user, &hash, false)
        .await
        .unwrap()
        .expect("picture for hash")
}

// ── Reads / listing ─────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn list_dir_and_stat_project_files(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let pic = seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"hello",
        "Photos.Travel",
    )
    .await;

    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    // Root lists the "Photos" mirror directory.
    let root = vfs.list_dir(&[]).await.unwrap();
    assert!(root.iter().any(|e| e.is_dir && e.name == "Photos"));

    // Photos lists the "Travel" subdir.
    let photos = vfs.list_dir(&seg(&["Photos"])).await.unwrap();
    assert!(photos.iter().any(|e| e.is_dir && e.name == "Travel"));

    // Photos/Travel lists the file with its projected name, size and ETag.
    let travel = vfs.list_dir(&seg(&["Photos", "Travel"])).await.unwrap();
    let file = travel.iter().find(|e| !e.is_dir).expect("a file");
    assert_eq!(file.name, "a.jpg");
    assert_eq!(file.size, 5);
    assert_eq!(file.picture_id, Some(pic));
    assert!(file.etag.is_some());

    // stat the file directly.
    let st = vfs
        .stat(&seg(&["Photos", "Travel", "a.jpg"]))
        .await
        .unwrap();
    assert!(!st.is_dir);
    assert_eq!(st.size, 5);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn read_proxy_returns_bytes_and_redirect_mode_redirects(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"PNGDATA",
        "Photos.Travel",
    )
    .await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;

    // Proxy mode (use_redirect=false): backend streams the bytes.
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();
    match vfs
        .read_file(&seg(&["Photos", "Travel", "a.jpg"]))
        .await
        .unwrap()
    {
        ReadTarget::Bytes { data, mime } => {
            assert_eq!(data, b"PNGDATA");
            assert_eq!(mime.as_deref(), Some("image/jpeg"));
        }
        ReadTarget::Redirect(_) => panic!("expected proxied bytes"),
    }

    // Redirect mode (use_redirect=true): 302 to a presigned URL.
    let vfs = Vfs::load(&state, user, h, true).await.unwrap();
    match vfs
        .read_file(&seg(&["Photos", "Travel", "a.jpg"]))
        .await
        .unwrap()
    {
        ReadTarget::Redirect(url) => assert!(url.contains("mock-s3")),
        ReadTarget::Bytes { .. } => panic!("expected redirect"),
    }
}

// ── Writes: PUT ───────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn put_new_picture_ingests_and_tags(db: PgPool) {
    let (state, storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    // Existing tag so the Photos/Travel mirror directory exists.
    seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"existing",
        "Photos.Travel",
    )
    .await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    let created = put(
        &vfs,
        &["Photos", "Travel", "new.jpg"],
        b"brandnewbytes",
        Some("image/jpeg"),
    )
    .await
    .unwrap();
    assert!(created, "new path PUT creates a resource");

    // A new owned picture exists, tagged Photos.Travel (mirror auto-tag), with hash set inline.
    let hash = archypix_common::hash::hash_bytes(&b"brandnewbytes".to_vec()).unwrap();
    let pic = PictureRepository::find_owned_by_hash(&state.db, user, &hash, false)
        .await
        .unwrap()
        .expect("new picture with inline hash");
    assert_eq!(pic.filename.as_deref(), Some("new.jpg"));
    assert_eq!(pic.file_size, Some(13));
    assert_eq!(
        tags_of(&state.db, user, pic.id).await,
        vec!["Photos.Travel"]
    );
    // Bytes streamed to the pictures bucket.
    assert_eq!(
        storage.get(
            &state.config.s3_bucket_pictures,
            &s3::picture_key(user, pic.id)
        ),
        Some(b"brandnewbytes".to_vec())
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn put_empty_body_is_noop(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"existing",
        "Photos.Travel",
    )
    .await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    let before: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM pictures WHERE local_user_id = $1",
        user
    )
    .fetch_one(&state.db)
    .await
    .unwrap()
    .unwrap_or(0);
    let created = put(&vfs, &["Photos", "Travel", "empty.jpg"], b"", None)
        .await
        .unwrap();
    assert!(created);
    let after: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM pictures WHERE local_user_id = $1",
        user
    )
    .fetch_one(&state.db)
    .await
    .unwrap()
    .unwrap_or(0);
    assert_eq!(before, after, "a zero-byte PUT must not create a picture");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn put_overwrite_replaces_bytes_no_version_when_none(db: PgPool) {
    let (state, storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let pic = seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"v1bytes",
        "Photos.Travel",
    )
    .await;
    UserSettingsRepository::upsert(&state.db, user, VersioningMode::None)
        .await
        .unwrap();
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    let created = put(
        &vfs,
        &["Photos", "Travel", "a.jpg"],
        b"v2-newer-bytes",
        Some("image/jpeg"),
    )
    .await
    .unwrap();
    assert!(!created, "overwrite returns false (no new resource)");

    // Bytes replaced in place; hash updated inline; no version snapshot under `none`.
    assert_eq!(
        storage.get(
            &state.config.s3_bucket_pictures,
            &s3::picture_key(user, pic)
        ),
        Some(b"v2-newer-bytes".to_vec())
    );
    let row = PictureRepository::find_by_id(&state.db, pic)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.file_size, Some(14));
    assert_eq!(
        row.file_hash,
        archypix_common::hash::hash_bytes(&b"v2-newer-bytes".to_vec())
    );
    assert!(
        !PictureVersionRepository::has_versions(&state.db, pic)
            .await
            .unwrap(),
        "versioning_mode=none never snapshots"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn put_overwrite_snapshots_version_full_versioning(db: PgPool) {
    let (state, storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let pic = seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"original-v1",
        "Photos.Travel",
    )
    .await;
    UserSettingsRepository::upsert(&state.db, user, VersioningMode::FullVersioning)
        .await
        .unwrap();
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    put(
        &vfs,
        &["Photos", "Travel", "a.jpg"],
        b"overwrite-v2",
        Some("image/jpeg"),
    )
    .await
    .unwrap();

    // A version row exists and the pre-overwrite bytes were copied to the versions bucket.
    let versions = PictureVersionRepository::list_by_picture(&state.db, pic)
        .await
        .unwrap();
    assert_eq!(
        versions.len(),
        1,
        "FullVersioning snapshots before overwrite"
    );
    let v = &versions[0];
    assert_eq!(
        storage.get(
            &state.config.s3_bucket_versions,
            &s3::version_key(user, pic, v.id)
        ),
        Some(b"original-v1".to_vec()),
        "the version holds the pristine pre-overwrite bytes"
    );
    // Live picture now has the new bytes.
    assert_eq!(
        storage.get(
            &state.config.s3_bucket_pictures,
            &s3::picture_key(user, pic)
        ),
        Some(b"overwrite-v2".to_vec())
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn put_overwrite_identical_hash_is_noop(db: PgPool) {
    // A dumb sync client re-PUTs byte-identical content. Even under FullVersioning (which snapshots
    // on every real overwrite), an identical-hash PUT must short-circuit: no version, no re-upload.
    let (state, storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let pic = seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"identical-bytes",
        "Photos.Travel",
    )
    .await;
    UserSettingsRepository::upsert(&state.db, user, VersioningMode::FullVersioning)
        .await
        .unwrap();
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    let created = put(
        &vfs,
        &["Photos", "Travel", "a.jpg"],
        b"identical-bytes",
        Some("image/jpeg"),
    )
    .await
    .unwrap();
    assert!(!created, "identical re-PUT returns false (no new resource)");

    assert!(
        !PictureVersionRepository::has_versions(&state.db, pic)
            .await
            .unwrap(),
        "identical-hash re-PUT must not snapshot a version even under FullVersioning"
    );
    // Bytes untouched in place.
    assert_eq!(
        storage.get(
            &state.config.s3_bucket_pictures,
            &s3::picture_key(user, pic)
        ),
        Some(b"identical-bytes".to_vec())
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn put_dedupe_existing_hash_retags_without_new_picture(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    // An existing picture tagged elsewhere; its mirror dir Photos/Beach exists.
    let existing = seed_full_picture(
        &state,
        user,
        "shared.jpg",
        "image/jpeg",
        b"dedupe-me",
        "Photos.Beach",
    )
    .await;
    // Another tag so Photos/Travel dir exists to drop into.
    seed_full_picture(
        &state,
        user,
        "filler.jpg",
        "image/jpeg",
        b"filler",
        "Photos.Travel",
    )
    .await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    let before: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM pictures WHERE local_user_id = $1",
        user
    )
    .fetch_one(&state.db)
    .await
    .unwrap()
    .unwrap_or(0);

    // PUT the SAME bytes into Photos/Travel — a dumb client's relocate expressed as a fresh upload.
    let created = put(
        &vfs,
        &["Photos", "Travel", "copy.jpg"],
        b"dedupe-me",
        Some("image/jpeg"),
    )
    .await
    .unwrap();
    assert!(
        !created,
        "hash hit on a live picture is a retag, not a new resource"
    );

    let after: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM pictures WHERE local_user_id = $1",
        user
    )
    .fetch_one(&state.db)
    .await
    .unwrap()
    .unwrap_or(0);
    assert_eq!(before, after, "no new picture row created");
    // The existing picture gained the target directory's tag.
    assert_eq!(
        tags_of(&state.db, user, existing).await,
        vec!["Photos.Beach", "Photos.Travel"]
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn put_undeletes_trashed_hash_match(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let trashed = seed_full_picture(
        &state,
        user,
        "old.jpg",
        "image/jpeg",
        b"resurrect",
        "Photos.Beach",
    )
    .await;
    // Trash it (simulating a delete+upload rename under fullDelete).
    PictureRepository::set_deleted(&state.db, user, trashed, true)
        .await
        .unwrap();
    // Filler so Photos/Travel exists.
    seed_full_picture(
        &state,
        user,
        "filler.jpg",
        "image/jpeg",
        b"filler",
        "Photos.Travel",
    )
    .await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    let created = put(
        &vfs,
        &["Photos", "Travel", "new.jpg"],
        b"resurrect",
        Some("image/jpeg"),
    )
    .await
    .unwrap();
    assert!(created, "un-delete reinstates the resource");

    let row = PictureRepository::find_by_id(&state.db, trashed)
        .await
        .unwrap()
        .unwrap();
    assert!(row.deleted_at.is_none(), "trashed picture was un-deleted");
    assert!(
        tags_of(&state.db, user, trashed)
            .await
            .contains(&"Photos.Travel".to_string())
    );
}

// ── Writes: MOVE / COPY / DELETE ─────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn move_rename_within_dir_sets_filename(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let pic = seed_full_picture(
        &state,
        user,
        "old.jpg",
        "image/jpeg",
        b"bytes",
        "Photos.Travel",
    )
    .await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    vfs.move_(
        &seg(&["Photos", "Travel", "old.jpg"]),
        &seg(&["Photos", "Travel", "renamed.jpg"]),
    )
    .await
    .unwrap();
    let row = PictureRepository::find_by_id(&state.db, pic)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.filename.as_deref(), Some("renamed.jpg"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn move_across_dirs_refiles_tags(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let pic = seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"bytes",
        "Photos.Travel",
    )
    .await;
    // Destination dir must already exist.
    seed_full_picture(
        &state,
        user,
        "b.jpg",
        "image/jpeg",
        b"other",
        "Photos.Beach",
    )
    .await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    vfs.move_(
        &seg(&["Photos", "Travel", "a.jpg"]),
        &seg(&["Photos", "Beach", "a.jpg"]),
    )
    .await
    .unwrap();
    // Travel tag removed, Beach tag added.
    assert_eq!(tags_of(&state.db, user, pic).await, vec!["Photos.Beach"]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn copy_adds_destination_tag_keeping_source(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let pic = seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"bytes",
        "Photos.Travel",
    )
    .await;
    seed_full_picture(
        &state,
        user,
        "b.jpg",
        "image/jpeg",
        b"other",
        "Photos.Beach",
    )
    .await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    vfs.copy(
        &seg(&["Photos", "Travel", "a.jpg"]),
        &seg(&["Photos", "Beach", "a.jpg"]),
    )
    .await
    .unwrap();
    assert_eq!(
        tags_of(&state.db, user, pic).await,
        vec!["Photos.Beach", "Photos.Travel"]
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn delete_single_branch_removes_only_accessed_tag(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let pic = seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"bytes",
        "Photos.Travel",
    )
    .await;
    // Give it a second tag so the picture survives the single-branch delete.
    TagRepository::batch_assign(&state.db, user, &[pic], &["Photos.Beach".to_string()])
        .await
        .unwrap();
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    vfs.delete(&seg(&["Photos", "Travel", "a.jpg"]))
        .await
        .unwrap();
    let row = PictureRepository::find_by_id(&state.db, pic)
        .await
        .unwrap()
        .unwrap();
    assert!(
        row.deleted_at.is_none(),
        "picture survives singleBranch delete"
    );
    assert_eq!(tags_of(&state.db, user, pic).await, vec!["Photos.Beach"]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn delete_full_delete_trashes_picture(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let pic = seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"bytes",
        "Photos.Travel",
    )
    .await;
    let h = make_hierarchy(&state.db, user, mirror_config("fullDelete")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    vfs.delete(&seg(&["Photos", "Travel", "a.jpg"]))
        .await
        .unwrap();
    let row = PictureRepository::find_by_id(&state.db, pic)
        .await
        .unwrap()
        .unwrap();
    assert!(row.deleted_at.is_some(), "fullDelete trashes the picture");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn delete_single_branch_conflicts_on_non_manual_tag(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    let pic = seed_full_picture(
        &state,
        user,
        "a.jpg",
        "image/jpeg",
        b"bytes",
        "Photos.Travel",
    )
    .await;
    // A live rule service also asserts Photos.Travel — a singleBranch delete must 409.
    sqlx::query(
        "INSERT INTO tags (picture_id, tag_path, source, source_id) \
         VALUES ($1, 'Photos.Travel'::text::ltree, 'rule'::tag_source, $2)",
    )
    .bind(pic)
    .bind(Uuid::new_v4())
    .execute(&state.db)
    .await
    .unwrap();
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    let err = vfs
        .delete(&seg(&["Photos", "Travel", "a.jpg"]))
        .await
        .unwrap_err();
    assert!(
        matches!(err, archypix_back::infra::error::AppError::Conflict(_)),
        "expected 409 when a service still asserts the tag, got {err:?}"
    );
}

// ── Case-insensitive write-side tag reuse (§10c) ────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn copy_folds_onto_existing_case_variant_tag(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    // Existing canonical-cased tag.
    seed_full_picture(&state, user, "a.jpg", "image/jpeg", b"a", "Photos.Travel").await;
    // The picture we will COPY into the lowercase-writeBack node.
    let pic = seed_full_picture(&state, user, "b.jpg", "image/jpeg", b"b", "Photos.Beach").await;

    // A writable query node whose writeBack assigns the lowercase `photos.travel`.
    let cfg = serde_json::json!({
        "nodes": [
            {"id": "n1", "kind": "mirror", "name": "Photos", "tagRoot": "Photos", "keepDir": true},
            {"id": "low", "kind": "query", "name": "Lower", "match": "all",
             "include": ["photos.travel"],
             "writeBack": {
                 "onAdd": [{"op": "assign", "path": "photos.travel"}],
                 "onRemove": [{"op": "remove", "path": "photos.travel"}]
             }}
        ]
    });
    let h = make_hierarchy(&state.db, user, cfg).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    vfs.copy(
        &seg(&["Photos", "Beach", "b.jpg"]),
        &seg(&["Lower", "b.jpg"]),
    )
    .await
    .unwrap();

    // §10c: the lowercase `photos.travel` is folded onto the existing-cased `Photos.Travel`;
    // a case-only duplicate is never minted.
    let tags = tags_of(&state.db, user, pic).await;
    assert!(
        tags.contains(&"Photos.Travel".to_string()),
        "folded to existing case: {tags:?}"
    );
    assert!(
        !tags.contains(&"photos.travel".to_string()),
        "no case-only duplicate: {tags:?}"
    );
}

// ── Brand-new mirror subdir auto-tag (§9) ────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn put_into_brand_new_mirror_subdir_mints_deep_tag(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    // Existing tag so the Photos/Travel mirror directory exists.
    seed_full_picture(&state, user, "a.jpg", "image/jpeg", b"a", "Photos.Travel").await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    // No MKCOL — PUT straight into a path whose parent dir does not yet exist.
    let created = put(
        &vfs,
        &["Photos", "Travel", "NewPlace", "x.jpg"],
        b"deepbytes",
        Some("image/jpeg"),
    )
    .await
    .unwrap();
    assert!(created);

    let pic = pic_by_hash(&state.db, user, b"deepbytes").await;
    assert_eq!(
        tags_of(&state.db, user, pic.id).await,
        vec!["Photos.Travel.NewPlace"],
        "deepest tag = tagRoot + new segments"
    );
    // Reloading the hierarchy, the new directory now resolves from the live tag.
    let vfs2 = Vfs::load(&state, user, h, false).await.unwrap();
    let travel = vfs2.list_dir(&seg(&["Photos", "Travel"])).await.unwrap();
    assert!(travel.iter().any(|e| e.is_dir && e.name == "NewPlace"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn put_into_multi_level_new_path_mints_deepest_tag(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    seed_full_picture(&state, user, "a.jpg", "image/jpeg", b"a", "Photos.Travel").await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    put(
        &vfs,
        &["Photos", "Travel", "A", "B", "x.jpg"],
        b"multilevel",
        Some("image/jpeg"),
    )
    .await
    .unwrap();
    let pic = pic_by_hash(&state.db, user, b"multilevel").await;
    assert_eq!(
        tags_of(&state.db, user, pic.id).await,
        vec!["Photos.Travel.A.B"]
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn mkcol_then_put_mints_tag_and_lists_pending(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    seed_full_picture(&state, user, "a.jpg", "image/jpeg", b"a", "Photos.Travel").await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    // MKCOL records a transient pending directory, surfaced by PROPFIND/list before any file.
    vfs.mkcol(&seg(&["Photos", "Travel", "Pending"]))
        .await
        .unwrap();
    let listed = vfs.list_dir(&seg(&["Photos", "Travel"])).await.unwrap();
    assert!(
        listed.iter().any(|e| e.is_dir && e.name == "Pending"),
        "pending MKCOL dir shows in the listing"
    );
    // stat the pending directory directly.
    let st = vfs
        .stat(&seg(&["Photos", "Travel", "Pending"]))
        .await
        .unwrap();
    assert!(st.is_dir);
    // A second MKCOL on the same path conflicts.
    assert!(matches!(
        vfs.mkcol(&seg(&["Photos", "Travel", "Pending"]))
            .await
            .unwrap_err(),
        archypix_back::infra::error::AppError::Conflict(_)
    ));

    // A file landing in it mints the real tag.
    put(
        &vfs,
        &["Photos", "Travel", "Pending", "p.jpg"],
        b"pendingbytes",
        Some("image/jpeg"),
    )
    .await
    .unwrap();
    let pic = pic_by_hash(&state.db, user, b"pendingbytes").await;
    assert_eq!(
        tags_of(&state.db, user, pic.id).await,
        vec!["Photos.Travel.Pending"]
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn finder_untitled_folder_flow_mkcol_rename_then_put_slugifies(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    seed_full_picture(&state, user, "a.jpg", "image/jpeg", b"a", "Photos.Travel").await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    // Finder creates "dossier sans titre" — MKCOL must NOT 409 on the spaces.
    vfs.mkcol(&seg(&["Photos", "dossier sans titre"]))
        .await
        .unwrap();
    let listed = vfs.list_dir(&seg(&["Photos"])).await.unwrap();
    assert!(
        listed
            .iter()
            .any(|e| e.is_dir && e.name == "dossier sans titre"),
        "the untitled folder shows with its original name"
    );

    // The user renames it (Finder MOVE on the empty pending directory).
    vfs.move_(
        &seg(&["Photos", "dossier sans titre"]),
        &seg(&["Photos", "Mes Vacances"]),
    )
    .await
    .unwrap();
    let listed = vfs.list_dir(&seg(&["Photos"])).await.unwrap();
    assert!(!listed.iter().any(|e| e.name == "dossier sans titre"));
    assert!(listed.iter().any(|e| e.is_dir && e.name == "Mes Vacances"));

    // Dropping a photo in mints a slugified tag.
    put(
        &vfs,
        &["Photos", "Mes Vacances", "p.jpg"],
        b"slugme",
        Some("image/jpeg"),
    )
    .await
    .unwrap();
    let pic = pic_by_hash(&state.db, user, b"slugme").await;
    assert_eq!(
        tags_of(&state.db, user, pic.id).await,
        vec!["Photos.Mes_Vacances"]
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn put_into_invalid_named_dir_slugifies_tag(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    seed_full_picture(&state, user, "a.jpg", "image/jpeg", b"a", "Photos.Travel").await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    // No MKCOL — a direct PUT into a spaced path slugifies the new segment.
    put(
        &vfs,
        &["Photos", "Bad Name!", "x.jpg"],
        b"directslug",
        Some("image/jpeg"),
    )
    .await
    .unwrap();
    let pic = pic_by_hash(&state.db, user, b"directslug").await;
    assert_eq!(
        tags_of(&state.db, user, pic.id).await,
        vec!["Photos.Bad_Name"]
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn mkcol_outside_mirror_is_forbidden(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    seed_full_picture(&state, user, "a.jpg", "image/jpeg", b"a", "Photos.Travel").await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    // The mount root is a container, not a mirror — cannot create a brand-new top-level directory.
    let err = vfs.mkcol(&seg(&["NewTop"])).await.unwrap_err();
    assert!(
        matches!(err, archypix_back::infra::error::AppError::Forbidden(_)),
        "got {err:?}"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn copy_into_brand_new_mirror_subdir_mints_tag(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    // Source is under Photos/Beach; the new subdir is under a different branch (Photos/Travel),
    // so the source tag is kept and the minted deep tag is added (no ancestor subsumption).
    let pic = seed_full_picture(&state, user, "a.jpg", "image/jpeg", b"a", "Photos.Beach").await;
    seed_full_picture(&state, user, "b.jpg", "image/jpeg", b"b", "Photos.Travel").await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    vfs.copy(
        &seg(&["Photos", "Beach", "a.jpg"]),
        &seg(&["Photos", "Travel", "Sub", "a.jpg"]),
    )
    .await
    .unwrap();
    assert_eq!(
        tags_of(&state.db, user, pic).await,
        vec!["Photos.Beach", "Photos.Travel.Sub"]
    );
}

// ── OS-junk sidecars (§11) ───────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn sidecar_is_stored_listed_read_and_deleted(db: PgPool) {
    let (state, _storage) = state_with_storage(db);
    let user = common::seed_user(&state.db, "alice", "pw").await;
    seed_full_picture(&state, user, "a.jpg", "image/jpeg", b"a", "Photos.Travel").await;
    let h = make_hierarchy(&state.db, user, mirror_config("singleBranch")).await;
    let vfs = Vfs::load(&state, user, h, false).await.unwrap();

    let path = seg(&["Photos", "Travel", ".DS_Store"]);
    vfs.put_sidecar(&path, b"\x00\x01junk", Some("application/octet-stream"))
        .await
        .unwrap();

    // It round-trips in the directory listing and via stat, but is never a picture.
    let listed = vfs.list_dir(&seg(&["Photos", "Travel"])).await.unwrap();
    let entry = listed
        .iter()
        .find(|e| e.name == ".DS_Store")
        .expect("sidecar listed");
    assert!(!entry.is_dir);
    assert_eq!(entry.size, 6);
    assert!(vfs.stat(&path).await.unwrap().picture_id.is_none());

    // Its bytes are served back.
    let (data, mime) = vfs.read_sidecar(&path).await.unwrap().unwrap();
    assert_eq!(data, b"\x00\x01junk");
    assert_eq!(mime.as_deref(), Some("application/octet-stream"));

    // No picture was created for it.
    let pics: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM pictures WHERE local_user_id = $1",
        user
    )
    .fetch_one(&state.db)
    .await
    .unwrap()
    .unwrap_or(0);
    assert_eq!(pics, 1, "sidecar never becomes a picture");

    // DELETE removes it.
    vfs.delete_sidecar(&path).await.unwrap();
    assert!(vfs.read_sidecar(&path).await.unwrap().is_none());
    let listed = vfs.list_dir(&seg(&["Photos", "Travel"])).await.unwrap();
    assert!(!listed.iter().any(|e| e.name == ".DS_Store"));
}
