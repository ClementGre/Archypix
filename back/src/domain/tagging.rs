//! Tagging services: the per-type config model, validation, and the pure evaluation dispatch.
//!
//! Every service type stores its payload in one `tagging_services.config` JSONB column (feature 20).
//! [`ServiceConfig`] is the typed, validated form: it parses + validates raw config from the API
//! ([`ServiceConfig::parse`]), re-serializes the normalized form ([`ServiceConfig::to_value`]), and
//! evaluates a picture ([`ServiceConfig::evaluate`]). Gating lives on [`TaggingService::should_run`].

use crate::domain::pipeline::PipelineInput;
use crate::domain::predicate::Predicate;
use crate::domain::segmentation::SegmentationConfig;
use crate::domain::tag::{TagPath, TagSource};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "service_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ServiceType {
    SharedTagMapping,
    Rule,
    Segmentation,
}

/// Used only in Hierarchy JSONB config, not a direct column type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SafeDeleteMode {
    SingleBranch,
    FullDelete,
}

/// A user-defined tagging service that assigns tags to pictures. Services are ordered into a
/// pipeline; the type-specific payload lives in `config` (parse with [`ServiceConfig::parse`]).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaggingService {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// User-facing label (may be empty; the UI falls back to a type label).
    pub name: String,
    pub service_type: ServiceType,
    /// Tags that must ALL be present for this service to fire (ltree[] as text[]).
    pub requires: Vec<String>,
    /// Tags where ANY present will suppress this service (ltree[] as text[]).
    pub excludes: Vec<String>,
    pub enabled: bool,
    /// Execution order within Rule/Segmentation services (SharedTagMapping always runs first).
    pub position: i32,
    /// Type-specific payload (raw JSONB; parse with [`ServiceConfig::parse`]).
    pub config: Value,
    /// Bumped on every configuration change. Pictures with `last_pipeline_run_at` older
    /// than this value are considered dirty and will be re-evaluated.
    pub last_invalidated_at: NaiveDateTime,
    /// Set when the pipeline fails to evaluate this service; cleared on next success.
    pub last_error_at: Option<NaiveDateTime>,
    pub last_error_msg: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl TaggingService {
    /// Whether this service should run for a picture carrying `current_tags`.
    ///
    /// Checks `enabled`, `requires` (all must match), and `excludes` (none must match). Tag presence
    /// is evaluated inclusively against virtual ancestors: a picture with `Photos.Travel.Alps`
    /// satisfies `requires: [Photos]`.
    pub fn should_run(&self, current_tags: &[TagPath]) -> bool {
        if !self.enabled {
            return false;
        }
        let present = |needle: &str| {
            let path = TagPath::from_ltree(needle);
            current_tags
                .iter()
                .any(|t| t == &path || t.ancestors().contains(&path))
        };
        self.requires.iter().all(|req| present(req))
            && !self.excludes.iter().any(|exc| present(exc))
    }

    pub fn rule_config(&self) -> Result<RuleConfig, serde_json::Error> {
        serde_json::from_value(self.config.clone())
    }

    pub fn mapping_config(&self) -> Result<SharedMappingConfig, serde_json::Error> {
        serde_json::from_value(self.config.clone())
    }

    pub fn segmentation_config(&self) -> Result<SegmentationConfig, serde_json::Error> {
        serde_json::from_value(self.config.clone())
    }
}

// ── Type-specific config payloads (feature 20 §10) ──────────────────────────────

/// `service_type = "rule"` config: an ordered list of rules (array order = display order).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleConfig {
    #[serde(default)]
    pub rules: Vec<RuleItem>,
}

/// One rule: a structured JSONB predicate tree (feature 13) → an assigned tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleItem {
    pub id: Uuid,
    pub predicate: Value,
    pub assign_tag: String,
}

/// `service_type = "shared_tag_mapping"` config: one service per incoming share (§10.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedMappingConfig {
    pub incoming_share_id: Uuid,
    #[serde(default)]
    pub assign_tags: Vec<String>,
}

// ── Unified, validated service config ───────────────────────────────────────────

/// The typed, validated config for a service. Built from raw JSON via [`ServiceConfig::parse`]
/// (validating predicates, tags, and segmentation bands), serialized back via [`Self::to_value`],
/// and evaluated against a picture via [`Self::evaluate`].
#[derive(Debug, Clone)]
pub enum ServiceConfig {
    SharedTagMapping(SharedMappingConfig),
    Rule(RuleConfig),
    Segmentation(SegmentationConfig),
}

/// Rule input shape: `id` is optional (server-assigned when absent) so the editor can submit new
/// rules without minting ids.
#[derive(Debug, Deserialize)]
struct RuleInput {
    #[serde(default)]
    id: Option<Uuid>,
    predicate: Value,
    assign_tag: String,
}

#[derive(Debug, Default, Deserialize)]
struct RuleConfigInput {
    #[serde(default)]
    rules: Vec<RuleInput>,
}

impl ServiceConfig {
    /// Parse + fully validate a raw config for `service_type`. Validates rule predicates (feature
    /// 13), assigned tags (non-protected ltree), and segmentation bands (§9). Rules missing an `id`
    /// get one assigned. The returned value is normalized — [`Self::to_value`] is storage-ready.
    pub fn parse(service_type: ServiceType, value: &Value) -> Result<ServiceConfig, String> {
        match service_type {
            ServiceType::Rule => {
                let input: RuleConfigInput = serde_json::from_value(value.clone())
                    .map_err(|e| format!("invalid rule config: {e}"))?;
                let rules = input
                    .rules
                    .into_iter()
                    .enumerate()
                    .map(|(i, r)| {
                        Predicate::parse(&r.predicate)
                            .map_err(|e| format!("rules[{i}].predicate: {e}"))?;
                        let assign_tag = parse_assign_tag(&r.assign_tag)
                            .map_err(|e| format!("rules[{i}].assign_tag: {e}"))?;
                        Ok(RuleItem {
                            id: r.id.unwrap_or_else(Uuid::new_v4),
                            predicate: r.predicate,
                            assign_tag,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(ServiceConfig::Rule(RuleConfig { rules }))
            }
            ServiceType::SharedTagMapping => {
                let cfg: SharedMappingConfig = serde_json::from_value(value.clone())
                    .map_err(|e| format!("invalid mapping config: {e}"))?;
                let assign_tags = cfg
                    .assign_tags
                    .iter()
                    .enumerate()
                    .map(|(i, t)| parse_assign_tag(t).map_err(|e| format!("assign_tags[{i}]: {e}")))
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(ServiceConfig::SharedTagMapping(SharedMappingConfig {
                    incoming_share_id: cfg.incoming_share_id,
                    assign_tags,
                }))
            }
            ServiceType::Segmentation => {
                let cfg: SegmentationConfig = serde_json::from_value(value.clone())
                    .map_err(|e| format!("invalid segmentation config: {e}"))?;
                cfg.validate()?;
                Ok(ServiceConfig::Segmentation(cfg))
            }
        }
    }

    /// The normalized, storage-ready JSON for this config.
    pub fn to_value(&self) -> Value {
        match self {
            ServiceConfig::Rule(c) => serde_json::to_value(c),
            ServiceConfig::SharedTagMapping(c) => serde_json::to_value(c),
            ServiceConfig::Segmentation(c) => serde_json::to_value(c),
        }
        .expect("service config serializes")
    }

    pub fn service_type(&self) -> ServiceType {
        match self {
            ServiceConfig::SharedTagMapping(_) => ServiceType::SharedTagMapping,
            ServiceConfig::Rule(_) => ServiceType::Rule,
            ServiceConfig::Segmentation(_) => ServiceType::Segmentation,
        }
    }

    /// The `tag_source` this service's produced tags are stored under.
    pub fn source(&self) -> TagSource {
        match self {
            ServiceConfig::SharedTagMapping(_) => TagSource::ShareMapping,
            ServiceConfig::Rule(_) => TagSource::Rule,
            ServiceConfig::Segmentation(_) => TagSource::Segment,
        }
    }

    /// Evaluate this service against one picture — pure, no I/O. `incoming_share_ids` are the
    /// picture's active incoming-share ids (only consulted by `shared_tag_mapping`).
    pub fn evaluate(&self, input: &PipelineInput, incoming_share_ids: &[Uuid]) -> ServiceResult {
        let tags_to_add = match self {
            ServiceConfig::SharedTagMapping(c) => {
                // Brokenness is derived (§10.1): a revoked share has no `incoming_share` tag on the
                // picture, so its id is absent here and the mapping yields nothing.
                if incoming_share_ids.contains(&c.incoming_share_id) {
                    c.assign_tags.iter().map(TagPath::from_ltree).collect()
                } else {
                    Vec::new()
                }
            }
            ServiceConfig::Rule(c) => c
                .rules
                .iter()
                .filter_map(|rule| match Predicate::parse(&rule.predicate) {
                    Ok(pred) if pred.matches(input) => Some(TagPath::from_ltree(&rule.assign_tag)),
                    Ok(_) => None,
                    Err(e) => {
                        // Validated on create/update, so this should not happen in practice.
                        tracing::warn!(error = %e, "rule predicate failed to parse — skipping");
                        None
                    }
                })
                .collect(),
            // Calendar segmentation resolves to at most one tag (§7).
            ServiceConfig::Segmentation(c) => c.resolve(input.captured_at).into_iter().collect(),
        };
        ServiceResult { tags_to_add }
    }
}

/// Validate + normalize a service-assigned tag (non-protected ltree).
fn parse_assign_tag(raw: &str) -> Result<String, String> {
    TagPath::parse(raw, false).map(|p| p.as_ltree().to_string())
}

/// Tags to add as a result of evaluating one service against one picture.
#[derive(Debug, Clone, Default)]
pub struct ServiceResult {
    pub tags_to_add: Vec<TagPath>,
}

/// Maps a filtered view of the tag graph to a WebDAV filesystem tree.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Hierarchy {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub config: sqlx::types::Json<Value>,
    pub enabled: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;
    use serde_json::json;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn service(requires: &[&str], excludes: &[&str]) -> TaggingService {
        TaggingService {
            id: Uuid::new_v4(),
            owner_id: Uuid::new_v4(),
            name: String::new(),
            service_type: ServiceType::Rule,
            requires: requires.iter().map(|s| s.to_string()).collect(),
            excludes: excludes.iter().map(|s| s.to_string()).collect(),
            enabled: true,
            position: 0,
            config: json!({}),
            last_invalidated_at: dt("2024-01-01 00:00:00"),
            last_error_at: None,
            last_error_msg: None,
            created_at: dt("2024-01-01 00:00:00"),
            updated_at: dt("2024-01-01 00:00:00"),
        }
    }

    fn tags(items: &[&str]) -> Vec<TagPath> {
        items.iter().map(|s| TagPath::from_ltree(*s)).collect()
    }

    fn blank_input() -> PipelineInput {
        PipelineInput {
            picture_id: Uuid::new_v4(),
            captured_at: None,
            ingested_at: None,
            updated_at: None,
            gps_lat: None,
            gps_lng: None,
            gps_alt: None,
            filename: None,
            camera_brand: None,
            camera_model: None,
            focal_length_mm: None,
            f_number: None,
            iso_speed: None,
            exposure_time: None,
            orientation: None,
            mime_type: None,
            file_size: None,
            width: None,
            height: None,
            is_owned: true,
            owner: "@alice:example.test".to_string(),
            creator: None,
        }
    }

    fn dated(s: &str) -> PipelineInput {
        PipelineInput {
            captured_at: Some(dt(s)),
            ..blank_input()
        }
    }

    // ── should_run ────────────────────────────────────────────────────────────

    #[test]
    fn disabled_service_never_runs() {
        let mut svc = service(&[], &[]);
        svc.enabled = false;
        assert!(!svc.should_run(&tags(&["Photos"])));
    }

    #[test]
    fn requires_exact_or_ancestor_match() {
        let svc = service(&["Photos"], &[]);
        assert!(!svc.should_run(&tags(&["Images"])));
        assert!(svc.should_run(&tags(&["Photos"])));
        assert!(svc.should_run(&tags(&["Photos.Travel.Alps"])));
    }

    #[test]
    fn excludes_suppresses_service() {
        let svc = service(&[], &["Images"]);
        assert!(svc.should_run(&tags(&["Photos"])));
        assert!(!svc.should_run(&tags(&["Images"])));
        assert!(!svc.should_run(&tags(&["Images.Icons"])));
    }

    #[test]
    fn all_requires_must_be_present() {
        let svc = service(&["Photos", "Travel"], &[]);
        assert!(!svc.should_run(&tags(&["Photos"])));
        assert!(svc.should_run(&tags(&["Photos", "Travel"])));
    }

    // ── ServiceConfig::parse ────────────────────────────────────────────────────

    #[test]
    fn parse_rule_assigns_missing_ids_and_validates() {
        let cfg = ServiceConfig::parse(
            ServiceType::Rule,
            &json!({"rules": [{"predicate": {"field": "captured_at", "year": 2024}, "assign_tag": "Photos.Y2024"}]}),
        )
            .unwrap();
        let ServiceConfig::Rule(rc) = &cfg else {
            panic!("expected rule config")
        };
        assert_eq!(rc.rules.len(), 1);
        assert_eq!(rc.rules[0].assign_tag, "Photos.Y2024");
        // round-trips through normalized JSON.
        assert!(cfg.to_value()["rules"][0]["id"].is_string());
    }

    #[test]
    fn parse_rejects_invalid_predicate_and_protected_tag() {
        assert!(
            ServiceConfig::parse(
                ServiceType::Rule,
                &json!({"rules": [{"predicate": {"bogus": 1}, "assign_tag": "X"}]})
            )
            .is_err()
        );
        assert!(
            ServiceConfig::parse(
                ServiceType::Rule,
                &json!({"rules": [{"predicate": {"and": []}, "assign_tag": "SharedToMe.X"}]})
            )
            .is_err()
        );
    }

    #[test]
    fn parse_mapping_validates_tags() {
        let share = Uuid::new_v4();
        let cfg = ServiceConfig::parse(
            ServiceType::SharedTagMapping,
            &json!({"incoming_share_id": share, "assign_tags": ["Photos.Holidays"]}),
        )
        .unwrap();
        assert_eq!(cfg.source(), TagSource::ShareMapping);
        assert!(
            ServiceConfig::parse(
                ServiceType::SharedTagMapping,
                &json!({"incoming_share_id": share, "assign_tags": ["SharedToMe.X"]})
            )
            .is_err()
        );
    }

    #[test]
    fn parse_segmentation_validates_bands() {
        assert!(
            ServiceConfig::parse(
                ServiceType::Segmentation,
                &json!({"version": 1, "root_tag": "Photos", "bands": [{"from": null, "to": null, "template": "{year}"}]})
            )
                .is_ok()
        );
        assert!(
            ServiceConfig::parse(
                ServiceType::Segmentation,
                &json!({"version": 1, "root_tag": "Photos", "bands": [{"from": null, "to": null, "template": "{nope}"}]})
            )
                .is_err()
        );
    }

    // ── ServiceConfig::evaluate ──────────────────────────────────────────────────

    fn rule_cfg(predicate: Value, assign: &str) -> ServiceConfig {
        ServiceConfig::parse(
            ServiceType::Rule,
            &json!({"rules": [{"predicate": predicate, "assign_tag": assign}]}),
        )
        .unwrap()
    }

    #[test]
    fn evaluate_rule_matches_and_misses() {
        let cfg = rule_cfg(
            json!({"field": "captured_at", "year": 2024}),
            "Photos.Y2024",
        );
        assert_eq!(
            cfg.evaluate(&dated("2024-06-01 12:00:00"), &[]).tags_to_add,
            tags(&["Photos.Y2024"])
        );
        assert!(
            cfg.evaluate(&dated("2023-06-01 12:00:00"), &[])
                .tags_to_add
                .is_empty()
        );
    }

    #[test]
    fn evaluate_segmentation_single_tag() {
        let cfg = ServiceConfig::parse(
            ServiceType::Segmentation,
            &json!({"version": 1, "root_tag": "Photos", "bands": [
                {"from": "2024-01-01", "to": "2025-01-01", "template": "{year}"}
            ]}),
        )
        .unwrap();
        assert_eq!(
            cfg.evaluate(&dated("2024-06-01 12:00:00"), &[]).tags_to_add,
            tags(&["Photos.2024"])
        );
        assert!(
            cfg.evaluate(&dated("2030-06-01 12:00:00"), &[])
                .tags_to_add
                .is_empty()
        );
    }

    #[test]
    fn evaluate_mapping_gates_on_active_share() {
        let share = Uuid::new_v4();
        let cfg = ServiceConfig::parse(
            ServiceType::SharedTagMapping,
            &json!({"incoming_share_id": share, "assign_tags": ["Photos.Holidays"]}),
        )
        .unwrap();
        assert_eq!(
            cfg.evaluate(&blank_input(), &[share]).tags_to_add,
            tags(&["Photos.Holidays"])
        );
        // Inactive share (absent id) yields nothing.
        assert!(
            cfg.evaluate(&blank_input(), &[Uuid::new_v4()])
                .tags_to_add
                .is_empty()
        );
    }
}
