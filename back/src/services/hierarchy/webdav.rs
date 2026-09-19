use crate::infra::settings;
use crate::infra::settings::keys;
use crate::repository::hierarchy::HierarchyRepository;
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use sqlx::PgPool;
use uuid::Uuid;


/// The WebDAV mount info returned to the owner.
pub struct WebdavInfo {
    /// `{scheme}://{back_domain}/webdav/{slug}` — the mount URL to paste into a client.
    pub url: String,
    /// The plaintext token (Basic-auth password). Decrypted for display.
    pub token: String,
    pub use_redirect: bool,
    pub enabled: bool,
}

fn webdav_url(settings: &Settings, name: &str) -> String {
    format!(
        "{}://{}/webdav/{}",
        settings::back_scheme(&settings),
        settings.get(keys::BACK_DOMAIN),
        crate::domain::hierarchy::slugify(name),
    )
}

/// Get the WebDAV mount info, minting a token on first access (so `GET …/webdav` always
/// returns a usable credential).
#[tracing::instrument(skip(db, settings), fields(user_id = %user_id, hierarchy_id = %hierarchy_id))]
pub async fn get_webdav_info(
    db: &PgPool,
    settings: &Settings,
    user_id: Uuid,
    hierarchy_id: Uuid,
) -> Result<WebdavInfo, AppError> {
    let row = HierarchyRepository::get_webdav(db, user_id, hierarchy_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let token = match row.webdav_token_enc {
        Some(blob) => {
            crate::infra::crypto::decrypt_webdav_token(&settings.get(keys::JWT_SECRET), &blob)?
        }
        None => {
            let token = crate::infra::crypto::generate_webdav_token();
            let blob = crate::infra::crypto::encrypt_webdav_token(
                &settings.get(keys::JWT_SECRET),
                &token,
            )?;
            HierarchyRepository::set_webdav_token(db, user_id, hierarchy_id, &blob).await?;
            token
        }
    };
    Ok(WebdavInfo {
        url: webdav_url(settings, &row.name),
        token,
        use_redirect: row.webdav_use_redirect,
        enabled: row.enabled,
    })
}

/// Rotate the WebDAV token (invalidates any mounted client).
#[tracing::instrument(skip(db, settings), fields(user_id = %user_id, hierarchy_id = %hierarchy_id))]
pub async fn regenerate_webdav_token(
    db: &PgPool,
    settings: &Settings,
    user_id: Uuid,
    hierarchy_id: Uuid,
) -> Result<WebdavInfo, AppError> {
    let row = HierarchyRepository::get_webdav(db, user_id, hierarchy_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let token = crate::infra::crypto::generate_webdav_token();
    let blob = crate::infra::crypto::encrypt_webdav_token(&settings.get(keys::JWT_SECRET), &token)?;
    HierarchyRepository::set_webdav_token(db, user_id, hierarchy_id, &blob).await?;
    Ok(WebdavInfo {
        url: webdav_url(settings, &row.name),
        token,
        use_redirect: row.webdav_use_redirect,
        enabled: row.enabled,
    })
}

/// Toggle the WebDAV read strategy (presigned redirect vs backend proxy).
#[tracing::instrument(skip(db), fields(user_id = %user_id, hierarchy_id = %hierarchy_id))]
pub async fn set_webdav_use_redirect(
    db: &PgPool,
    user_id: Uuid,
    hierarchy_id: Uuid,
    use_redirect: bool,
) -> Result<(), AppError> {
    let updated =
        HierarchyRepository::set_webdav_use_redirect(db, user_id, hierarchy_id, use_redirect)
            .await?;
    if updated {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}
