use super::FederationClient;
use crate::clients::federation::models::{
    PictureEditRequest, PicturesAnnouncementRequest, PicturesUnannouncementRequest, PresignRequest,
    PresignRequestItem, PresignResponse, ShareAcceptRequest, ShareAnnouncementRequest,
    ShareAnnouncementResponse, ShareRejectRequest, ShareRevokeRequest,
};
use crate::infra::error::AppError;
use std::collections::HashMap;
use tracing::{debug, warn};
use uuid::Uuid;

impl FederationClient {
    /// Request presigned URLs for a batch of pictures stored on a remote instance, authorised
    /// by per-picture tokens. A single HTTP call is made per owner backend. The owner identity
    /// is only used to resolve the backend URL — the request body carries just the tokens, which
    /// are self-resolving on the owner's side. Returns a map of `picture_token → url`.
    #[tracing::instrument(
        skip(self, pictures),
        fields(otel.kind = "client", owner_username = %owner_username, owner_global_domain = %owner_global_domain, picture_count = pictures.len())
    )]
    pub async fn presign_remote_pictures(
        &self,
        owner_username: &str,
        owner_global_domain: &str,
        pictures: &[(Uuid, &str)],
    ) -> Result<HashMap<Uuid, String>, AppError> {
        let backend_base_url = self
            .resolve_backend_url(owner_username, owner_global_domain)
            .await?;
        let url = format!("{}/api/federation/pictures/presign", backend_base_url);

        let items: Vec<PresignRequestItem> = pictures
            .iter()
            .map(|(token, variant)| PresignRequestItem {
                picture_token: *token,
                variant: Some(variant.to_string()),
            })
            .collect();

        let resp = self
            .http
            .post(&url)
            .headers(self.trace_headers_for(owner_global_domain))
            .json(&PresignRequest { pictures: items })
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        let body: PresignResponse = resp
            .json()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        Ok(body
            .urls
            .into_iter()
            .map(|r| (r.picture_token, r.url))
            .collect())
    }

    /// Notify the recipient's backend that an outgoing share has been revoked.
    ///
    /// Identified by `outgoing_share_id` so the recipient can look up their `IncomingShare`
    /// without Alice needing to know Bob's internal IDs.
    #[tracing::instrument(
        skip(self),
        fields(otel.kind = "client", sender_username = %sender_username, recipient_username = %recipient_username, recipient_global_domain = %recipient_global_domain, %outgoing_share_id
        )
    )]
    pub async fn send_revocation(
        &self,
        sender_username: &str,
        recipient_username: &str,
        recipient_global_domain: &str,
        outgoing_share_id: Uuid,
    ) -> Result<(), AppError> {
        let token = self
            .get_or_wait_federation_token(
                sender_username,
                recipient_username,
                recipient_global_domain,
            )
            .await?;
        let backend_base_url = self
            .resolve_backend_url(recipient_username, recipient_global_domain)
            .await?;
        debug!(backend_base_url, "federation: sending share revocation");
        let url = format!("{}/api/federation/shares/revoke", backend_base_url);
        self.http
            .post(&url)
            .bearer_auth(&token)
            .headers(self.trace_headers_for(recipient_global_domain))
            .json(&ShareRevokeRequest { outgoing_share_id })
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: revocation delivery failed");
                AppError::InternalServerError(e.to_string())
            })?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }

    /// Announce a new outgoing share to the recipient's backend.
    ///
    /// Returns `auto_accepted`: `true` when the recipient auto-accepted the share (a verified
    /// ShareBack). In that case the caller — still inside its share-creation transaction — must
    /// itself announce its pictures to the recipient (the recipient does *not* call back, so the
    /// flow stays linear and within one transaction; see the federation consistency rules in
    /// `03_BACKEND_ARCHITECTURE.md`).
    #[tracing::instrument(
        skip(self, token, announcement),
        fields(otel.kind = "client", recipient_username = %recipient_username, recipient_global_domain = %recipient_global_domain, tag_path = %announcement.tag_path
        )
    )]
    pub async fn announce_share(
        &self,
        recipient_username: &str,
        recipient_global_domain: &str,
        token: &str,
        announcement: &ShareAnnouncementRequest,
    ) -> Result<bool, AppError> {
        let backend_base_url = self
            .resolve_backend_url(recipient_username, recipient_global_domain)
            .await?;
        debug!(backend_base_url, "federation: announcing share");
        let url = format!("{}/api/federation/shares/announce", backend_base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .headers(self.trace_headers_for(recipient_global_domain))
            .json(announcement)
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: share announcement delivery failed");
                AppError::InternalServerError(e.to_string())
            })?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        let body: ShareAnnouncementResponse = resp
            .json()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(body.auto_accepted)
    }

    /// Send a share-acceptance notification to the sender's backend.
    ///
    /// Called by the recipient (Bob) after accepting an incoming share. The sender (Alice) will
    /// respond by announcing all current pictures under the shared tag.
    #[tracing::instrument(
        skip(self),
        fields(otel.kind = "client", acceptor_username = %acceptor_username, sender_username = %sender_username, sender_global_domain = %sender_global_domain, %outgoing_share_id
        )
    )]
    pub async fn send_share_accept(
        &self,
        acceptor_username: &str,
        sender_username: &str,
        sender_global_domain: &str,
        outgoing_share_id: Uuid,
    ) -> Result<(), AppError> {
        let token = self
            .get_or_wait_federation_token(acceptor_username, sender_username, sender_global_domain)
            .await?;
        let backend_base_url = self
            .resolve_backend_url(sender_username, sender_global_domain)
            .await?;
        debug!(backend_base_url, "federation: sending share accept");
        let url = format!("{}/api/federation/shares/accept", backend_base_url);
        self.http
            .post(&url)
            .bearer_auth(&token)
            .headers(self.trace_headers_for(sender_global_domain))
            .json(&ShareAcceptRequest { outgoing_share_id })
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: share accept delivery failed");
                AppError::InternalServerError(e.to_string())
            })?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }

    /// Send a share-rejection notification to the sender's backend.
    ///
    /// Called by the recipient (Bob) after rejecting an incoming share. The sender (Alice) will
    /// tombstone her OutgoingShare so it no longer appears as pending/active on her side.
    #[tracing::instrument(
        skip(self),
        fields(otel.kind = "client", rejector_username = %rejector_username, sender_username = %sender_username, sender_global_domain = %sender_global_domain, %outgoing_share_id
        )
    )]
    pub async fn send_share_reject(
        &self,
        rejector_username: &str,
        sender_username: &str,
        sender_global_domain: &str,
        outgoing_share_id: Uuid,
    ) -> Result<(), AppError> {
        let token = self
            .get_or_wait_federation_token(rejector_username, sender_username, sender_global_domain)
            .await?;
        let backend_base_url = self
            .resolve_backend_url(sender_username, sender_global_domain)
            .await?;
        debug!(backend_base_url, "federation: sending share reject");
        let url = format!("{}/api/federation/shares/reject", backend_base_url);
        self.http
            .post(&url)
            .bearer_auth(&token)
            .headers(self.trace_headers_for(sender_global_domain))
            .json(&ShareRejectRequest { outgoing_share_id })
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: share reject delivery failed");
                AppError::InternalServerError(e.to_string())
            })?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }

    /// Announce a batch of pictures to the recipient's backend after share acceptance.
    ///
    /// Called by the sender (Alice) to push all pictures currently under the shared tag to Bob.
    #[tracing::instrument(
        skip(self, payload),
        fields(otel.kind = "client", sender_username = %sender_username, recipient_username = %recipient_username, recipient_global_domain = %recipient_global_domain, picture_count = payload.pictures.len()
        )
    )]
    pub async fn announce_pictures_to_backend(
        &self,
        sender_username: &str,
        recipient_username: &str,
        recipient_global_domain: &str,
        payload: &PicturesAnnouncementRequest,
    ) -> Result<(), AppError> {
        let token = self
            .get_or_wait_federation_token(
                sender_username,
                recipient_username,
                recipient_global_domain,
            )
            .await?;
        let backend_base_url = self
            .resolve_backend_url(recipient_username, recipient_global_domain)
            .await?;
        debug!(backend_base_url, "federation: announcing pictures");
        let url = format!("{}/api/federation/pictures/announce", backend_base_url);
        self.http
            .post(&url)
            .bearer_auth(&token)
            .headers(self.trace_headers_for(recipient_global_domain))
            .json(payload)
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: pictures announcement delivery failed");
                AppError::InternalServerError(e.to_string())
            })?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }

    /// Send a recipient's EXIF edit proposal to the **owner's** backend (10 §4.2). The proposing
    /// recipient (`requester_username`) holds the federation token toward the owner. The owner
    /// re-verifies the grant and applies the edit; a non-2xx (e.g. 403 grant revoked, 409 still
    /// processing) surfaces as an error the caller can relay to the recipient.
    #[tracing::instrument(
        skip(self, payload),
        fields(otel.kind = "client", requester_username = %requester_username, owner_username = %owner_username, owner_global_domain = %owner_global_domain, picture_id = %payload.picture_id
        )
    )]
    pub async fn send_picture_edit_request(
        &self,
        requester_username: &str,
        owner_username: &str,
        owner_global_domain: &str,
        payload: &PictureEditRequest,
    ) -> Result<(), AppError> {
        let token = self
            .get_or_wait_federation_token(requester_username, owner_username, owner_global_domain)
            .await?;
        let backend_base_url = self
            .resolve_backend_url(owner_username, owner_global_domain)
            .await?;
        debug!(backend_base_url, "federation: sending picture edit request");
        let url = format!("{}/api/federation/pictures/edit_request", backend_base_url);
        self.http
            .post(&url)
            .bearer_auth(&token)
            .headers(self.trace_headers_for(owner_global_domain))
            .json(payload)
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: picture edit request delivery failed");
                AppError::InternalServerError(e.to_string())
            })?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }

    /// Unannounce a batch of pictures from the recipient's backend (pictures left a share's
    /// coverage while the share remains active).
    #[tracing::instrument(
        skip(self, payload),
        fields(otel.kind = "client", sender_username = %sender_username, recipient_username = %recipient_username, recipient_global_domain = %recipient_global_domain, picture_count = payload.picture_ids.len()
        )
    )]
    pub async fn unannounce_pictures_to_backend(
        &self,
        sender_username: &str,
        recipient_username: &str,
        recipient_global_domain: &str,
        payload: &PicturesUnannouncementRequest,
    ) -> Result<(), AppError> {
        let token = self
            .get_or_wait_federation_token(
                sender_username,
                recipient_username,
                recipient_global_domain,
            )
            .await?;
        let backend_base_url = self
            .resolve_backend_url(recipient_username, recipient_global_domain)
            .await?;
        debug!(backend_base_url, "federation: unannouncing pictures");
        let url = format!("{}/api/federation/pictures/unannounce", backend_base_url);
        self.http
            .post(&url)
            .bearer_auth(&token)
            .headers(self.trace_headers_for(recipient_global_domain))
            .json(payload)
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "federation: pictures unannouncement delivery failed");
                AppError::InternalServerError(e.to_string())
            })?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }
}
