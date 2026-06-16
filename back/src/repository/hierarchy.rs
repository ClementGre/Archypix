use crate::infra::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use sqlx::{Executor, Postgres};
use uuid::Uuid;

/// A hierarchy row. `config` is the raw JSONB blob — the service parses it into a
/// `domain::hierarchy::HierarchyConfig`.
#[derive(Debug, Clone)]
pub struct HierarchyRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub config: serde_json::Value,
    pub enabled: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

pub struct HierarchyRepository;

impl HierarchyRepository {
    pub async fn list_by_owner<'e, E>(ex: E, owner_id: Uuid) -> Result<Vec<HierarchyRow>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            HierarchyRow,
            r#"SELECT id, owner_id, name, config as "config!: serde_json::Value",
                      enabled, created_at, updated_at
               FROM hierarchies
               WHERE owner_id = $1
               ORDER BY name"#,
            owner_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    pub async fn get_by_owner_and_id<'e, E>(
        ex: E,
        owner_id: Uuid,
        id: Uuid,
    ) -> Result<Option<HierarchyRow>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            HierarchyRow,
            r#"SELECT id, owner_id, name, config as "config!: serde_json::Value",
                      enabled, created_at, updated_at
               FROM hierarchies
               WHERE id = $1 AND owner_id = $2"#,
            id,
            owner_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    pub async fn create<'e, E>(
        ex: E,
        owner_id: Uuid,
        name: &str,
        config: &serde_json::Value,
    ) -> Result<HierarchyRow, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            HierarchyRow,
            r#"INSERT INTO hierarchies (owner_id, name, config)
               VALUES ($1, $2, $3)
               RETURNING id, owner_id, name, config as "config!: serde_json::Value",
                         enabled, created_at, updated_at"#,
            owner_id,
            name,
            config,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Update name / enabled / config (any subset). Omitted fields are left unchanged.
    pub async fn update<'e, E>(
        ex: E,
        owner_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        enabled: Option<bool>,
        config: Option<&serde_json::Value>,
    ) -> Result<Option<HierarchyRow>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            HierarchyRow,
            r#"UPDATE hierarchies
               SET name = COALESCE($3, name),
                   enabled = COALESCE($4, enabled),
                   config = COALESCE($5, config)
               WHERE id = $1 AND owner_id = $2
               RETURNING id, owner_id, name, config as "config!: serde_json::Value",
                         enabled, created_at, updated_at"#,
            id,
            owner_id,
            name,
            enabled,
            config,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
    }

    pub async fn delete<'e, E>(ex: E, owner_id: Uuid, id: Uuid) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let res = sqlx::query!(
            "DELETE FROM hierarchies WHERE id = $1 AND owner_id = $2",
            id,
            owner_id,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    async fn seed_user(db: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, username, email, display_name) VALUES ($1, $2, $3, $4)",
            id,
            format!("u_{}", &id.to_string()[..8]),
            format!("{}@t.com", id),
            "T",
        )
        .execute(db)
        .await
        .unwrap();
        id
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn create_get_update_delete_roundtrip(db: PgPool) {
        let user = seed_user(&db).await;
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
        let user = seed_user(&db).await;
        let config = serde_json::json!({"version": 1, "nodes": []});
        HierarchyRepository::create(&db, user, "Photos", &config)
            .await
            .unwrap();
        let err = HierarchyRepository::create(&db, user, "Photos", &config).await;
        assert!(matches!(err, Err(AppError::Conflict(_))));
    }

    #[sqlx::test(migrator = "MIGRATOR")]
    async fn other_owner_cannot_get_or_delete(db: PgPool) {
        let alice = seed_user(&db).await;
        let bob = seed_user(&db).await;
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
}
