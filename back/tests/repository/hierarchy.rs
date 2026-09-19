//! `HierarchyRepository` CRUD, per-owner uniqueness and owner isolation.

use crate::MIGRATOR;
use crate::common::seed_user_bare;
use archypix_back::repository::hierarchy::HierarchyRepository;
use archypix_common::error::AppError;
use sqlx::PgPool;

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_get_update_delete_roundtrip(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let config = serde_json::json!({"version": 1, "nodes": []});

    let created = HierarchyRepository::create(&db, user, "Photos", &config)
        .await
        .unwrap();
    assert_eq!(created.name, "Photos");
    assert!(created.enabled);

    let fetched = HierarchyRepository::get_by_owner_and_id(&db, user, created.id)
        .await
        .unwrap()
        .expect("exists");
    assert_eq!(fetched.id, created.id);

    let new_config = serde_json::json!({"version": 1, "nodes": [], "writeBack": false});
    let updated = HierarchyRepository::update(
        &db,
        user,
        created.id,
        Some("Renamed"),
        Some(false),
        Some(&new_config),
    )
    .await
    .unwrap()
    .expect("updated");
    assert_eq!(updated.name, "Renamed");
    assert!(!updated.enabled);
    assert_eq!(updated.config["writeBack"], serde_json::json!(false));

    let list = HierarchyRepository::list_by_owner(&db, user).await.unwrap();
    assert_eq!(list.len(), 1);

    assert!(
        HierarchyRepository::delete(&db, user, created.id)
            .await
            .unwrap()
    );
    assert!(
        HierarchyRepository::get_by_owner_and_id(&db, user, created.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unique_name_per_owner_conflicts(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let config = serde_json::json!({"version": 1, "nodes": []});
    HierarchyRepository::create(&db, user, "Photos", &config)
        .await
        .unwrap();
    let err = HierarchyRepository::create(&db, user, "Photos", &config).await;
    assert!(matches!(err, Err(AppError::Conflict(_))));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn other_owner_cannot_get_or_delete(db: PgPool) {
    let alice = seed_user_bare(&db).await;
    let bob = seed_user_bare(&db).await;
    let config = serde_json::json!({"version": 1, "nodes": []});
    let h = HierarchyRepository::create(&db, alice, "Photos", &config)
        .await
        .unwrap();

    assert!(
        HierarchyRepository::get_by_owner_and_id(&db, bob, h.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(!HierarchyRepository::delete(&db, bob, h.id).await.unwrap());
}
