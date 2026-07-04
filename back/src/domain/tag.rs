use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "tag_source", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TagSource {
    Manual,
    Rule,
    Segment,
    ShareMapping,
    IncomingShare,
}

impl TagSource {
    /// The `tag_source` enum value as stored text (matches the snake_case Postgres labels).
    pub fn as_str(self) -> &'static str {
        match self {
            TagSource::Manual => "manual",
            TagSource::Rule => "rule",
            TagSource::Segment => "segment",
            TagSource::ShareMapping => "share_mapping",
            TagSource::IncomingShare => "incoming_share",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Tag {
    pub id: Uuid,
    pub picture_id: Uuid,
    /// Stored as ltree text (dot-separated, e.g. `Photos.Travel.Alps`).
    pub tag_path: String,
    pub source: TagSource,
    pub source_id: Option<Uuid>,
    pub assigned_at: NaiveDateTime,
}

/// A validated, normalized tag path in ltree format (dot-separated labels).
///
/// Human-readable form uses slashes: `/Photos/Travel/Alps`
/// Stored ltree form uses dots: `Photos.Travel.Alps`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TagPath(String);

impl TagPath {
    /// Parse and validate a user-supplied tag path in dot-separated `ltree` form
    /// (`Photos.Travel.Alps`) — the same form the API returns, so responses can be fed
    /// straight back into requests.
    ///
    /// - Trims surrounding whitespace.
    /// - Splits on `.`; each label must be non-empty and contain only `[A-Za-z0-9_]`.
    /// - Returns the validated path unchanged.
    pub fn parse(raw: &str, allow_protected_prefixes: bool) -> Result<Self, String> {
        let stripped = raw.trim();
        if stripped.is_empty() {
            return Err("tag path must not be empty".to_string());
        }
        for segment in stripped.split('.') {
            if segment.is_empty() {
                return Err(
                    "tag path must not contain empty segments (no leading, trailing, or doubled dots)"
                        .to_string(),
                );
            }
            if !segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(format!(
                    "tag segment {segment:?} contains invalid characters — only letters, digits, and underscores [A-Za-z0-9_] are allowed"
                ));
            }
        }
        if !allow_protected_prefixes && Self::is_reserved_prefix(stripped) {
            return Err(format!(
                "tag path {stripped:?} uses a reserved prefix (SharedToMe, ...)"
            ));
        }
        Ok(TagPath(stripped.to_string()))
    }

    /// Coerce an arbitrary directory name into a valid tag label (`[A-Za-z0-9_]`). Runs of
    /// disallowed characters collapse to a single `_`; leading/trailing `_` are trimmed; an
    /// otherwise-empty result becomes `untitled`. Case is preserved (tags are case-sensitive).
    ///
    /// Used by the WebDAV write path so a filesystem folder like Finder's "dossier sans titre"
    /// (`dossier_sans_titre`) can mint a tag when a picture is dropped into it (06_webdav.md §9).
    pub fn slugify_label(raw: &str) -> String {
        let mut out = String::with_capacity(raw.len());
        let mut prev_underscore = false;
        for ch in raw.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                out.push(ch);
                prev_underscore = ch == '_';
            } else if !prev_underscore {
                out.push('_');
                prev_underscore = true;
            }
        }
        let trimmed = out.trim_matches('_');
        if trimmed.is_empty() {
            "untitled".to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Parse from the human-readable slash-separated form (`/Photos/Travel/Alps`).
    /// For internal use with trusted paths — does not validate characters.
    pub fn from_slash(raw: &str) -> Self {
        let normalized = raw.trim().trim_start_matches('/').replace('/', ".");
        TagPath(normalized)
    }

    /// Wrap an already-normalized ltree string.
    pub fn from_ltree(ltree: impl Into<String>) -> Self {
        TagPath(ltree.into())
    }

    pub fn as_ltree(&self) -> &str {
        &self.0
    }

    /// Whether this path is the reserved `SharedToMe` prefix (exact or a descendant). Manual tag
    /// writes and pipeline service `assign_tag` values must not use it — only the share machinery
    /// may assign `incoming_share` tags under `SharedToMe`.
    pub fn is_reserved_prefix(path: &str) -> bool {
        path == "SharedToMe" || path.starts_with("SharedToMe.")
    }

    /// Build the `/SharedToMe/<sender>/...` path for a federation-received tag.
    ///
    /// Sender identity is encoded as `{username}_AT_{instance}` where `.` becomes `_DOT_`,
    /// giving a reversible, unambiguous LTREE label. Example:
    /// `alice@instance.com` + `Photos.Travel` → `SharedToMe.alice_AT_instance_DOT_com.Photos.Travel`
    pub fn shared_to_me(sender_username: &str, sender_instance: &str, original: &TagPath) -> Self {
        let label = encode_sender_label(sender_username, sender_instance);
        if original.0.is_empty() {
            TagPath(format!("SharedToMe.{label}"))
        } else {
            TagPath(format!("SharedToMe.{label}.{}", original.0))
        }
    }

    /// Returns true if `self` is a proper ancestor of `other` in the LTREE hierarchy.
    ///
    /// `Photos.Travel`.is_ancestor_of(`Photos.Travel.Alps`) → `true`
    /// `Photos.Travel`.is_ancestor_of(`Photos.Travel`) → `false` (not *proper*)
    pub fn is_ancestor_of(&self, other: &TagPath) -> bool {
        other.0.starts_with(&format!("{}.", self.0))
    }

    /// All ancestor paths, from root down to (but not including) self.
    pub fn ancestors(&self) -> Vec<TagPath> {
        let parts: Vec<&str> = self.0.split('.').collect();
        (0..parts.len().saturating_sub(1))
            .map(|i| TagPath(parts[..=i].join(".")))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Rewrite this path for a tag-rename cascade: if it is `old` (exact) or a descendant of `old`,
    /// return it with the `old` prefix swapped for `new` (the suffix preserved). Returns `None` when
    /// this path is unrelated to `old` (left unchanged). `Photos.Travel.Alps`.rename_under(
    /// `Photos.Travel`, `Photos.Vacation`) → `Photos.Vacation.Alps`.
    pub fn rename_under(&self, old: &TagPath, new: &TagPath) -> Option<TagPath> {
        if self == old {
            return Some(new.clone());
        }
        if old.is_ancestor_of(self) {
            let suffix = &self.0[old.0.len() + 1..];
            return Some(TagPath(format!("{}.{suffix}", new.0)));
        }
        None
    }

    /// Reduce a collection of paths to its "deepest" form: drop exact duplicates and
    /// any path that is a proper ancestor of another path in the set.
    ///
    /// Ancestors are virtual, so the surviving deepest paths fully represent the set.
    /// Used both to keep a single source's output minimal and to fold the per-source
    /// rows of a picture into a display set. Order of the input is preserved.
    pub fn fold_deepest(paths: impl IntoIterator<Item = TagPath>) -> Vec<TagPath> {
        let mut unique: Vec<TagPath> = Vec::new();
        for p in paths {
            if !unique.contains(&p) {
                unique.push(p);
            }
        }
        unique
            .iter()
            .filter(|p| !unique.iter().any(|other| p.is_ancestor_of(other)))
            .cloned()
            .collect()
    }
}

impl std::fmt::Display for TagPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for TagPath {
    fn from(s: String) -> Self {
        TagPath(s)
    }
}

impl From<&str> for TagPath {
    fn from(s: &str) -> Self {
        TagPath(s.to_string())
    }
}

/// Encode a sender identity (`username@instance`) as a single LTREE label.
///
/// `@` → `_AT_`, `.` → `_DOT_`, any remaining non-alphanumeric → `_`.
///
/// Usernames are restricted to `[a-z0-9_]` at registration, so only dots and the `_AT_`
/// separator need escaping. No collisions are possible within the username component.
pub fn encode_sender_label(username: &str, instance: &str) -> String {
    let encode = |s: &str| -> String {
        s.chars()
            .map(|c| match c {
                '.' => "_DOT_".to_string(),
                c if c.is_ascii_alphanumeric() || c == '_' => c.to_string(),
                _ => "_".to_string(),
            })
            .collect()
    };
    format!("{}_AT_{}", encode(username), encode(instance))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TagPath::ancestors ────────────────────────────────────────────────────

    #[test]
    fn ancestors_root_is_empty() {
        let t = TagPath::from_ltree("Photos");
        assert_eq!(t.ancestors(), vec![]);
    }

    #[test]
    fn ancestors_two_levels() {
        let t = TagPath::from_ltree("Photos.Travel");
        assert_eq!(t.ancestors(), vec![TagPath::from_ltree("Photos")]);
    }

    #[test]
    fn ancestors_three_levels() {
        let t = TagPath::from_ltree("Photos.Travel.Alps");
        assert_eq!(
            t.ancestors(),
            vec![
                TagPath::from_ltree("Photos"),
                TagPath::from_ltree("Photos.Travel"),
            ]
        );
    }

    #[test]
    fn ancestors_empty_path_is_empty() {
        let t = TagPath::from_ltree("");
        assert_eq!(t.ancestors(), vec![]);
    }

    // ── TagPath::rename_under ─────────────────────────────────────────────────

    #[test]
    fn rename_under_exact_match() {
        let p = TagPath::from_ltree("Photos.Travel");
        let renamed = p.rename_under(
            &TagPath::from_ltree("Photos.Travel"),
            &TagPath::from_ltree("Photos.Vacation"),
        );
        assert_eq!(renamed, Some(TagPath::from_ltree("Photos.Vacation")));
    }

    #[test]
    fn rename_under_descendant_keeps_suffix() {
        let p = TagPath::from_ltree("Photos.Travel.Alps.Hiking");
        let renamed = p.rename_under(
            &TagPath::from_ltree("Photos.Travel"),
            &TagPath::from_ltree("Trips.2024"),
        );
        assert_eq!(renamed, Some(TagPath::from_ltree("Trips.2024.Alps.Hiking")));
    }

    #[test]
    fn rename_under_unrelated_is_none() {
        let p = TagPath::from_ltree("Images.Icons");
        assert_eq!(
            p.rename_under(
                &TagPath::from_ltree("Photos.Travel"),
                &TagPath::from_ltree("Photos.Vacation")
            ),
            None
        );
        // A prefix that is not a label boundary (`Photos.Travels` vs `Photos.Travel`) must not match.
        let sibling = TagPath::from_ltree("Photos.Travels");
        assert_eq!(
            sibling.rename_under(
                &TagPath::from_ltree("Photos.Travel"),
                &TagPath::from_ltree("Photos.Vacation")
            ),
            None
        );
    }

    // ── TagPath::parse ────────────────────────────────────────────────────────

    #[test]
    fn parse_dot_path_roundtrips() {
        let t = TagPath::parse("Photos.Travel.Alps", true).unwrap();
        assert_eq!(t.as_ltree(), "Photos.Travel.Alps");
    }

    #[test]
    fn parse_trims_whitespace() {
        let t = TagPath::parse("  Photos.Travel  ", true).unwrap();
        assert_eq!(t.as_ltree(), "Photos.Travel");
    }

    #[test]
    fn parse_single_segment() {
        let t = TagPath::parse("Photos", true).unwrap();
        assert_eq!(t.as_ltree(), "Photos");
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(TagPath::parse("", true).is_err());
        assert!(TagPath::parse(".", true).is_err());
        assert!(TagPath::parse("   ", true).is_err());
    }

    #[test]
    fn parse_rejects_double_dot() {
        assert!(TagPath::parse("Photos..Travel", true).is_err());
    }

    #[test]
    fn parse_rejects_leading_and_trailing_dot() {
        assert!(TagPath::parse(".Photos", true).is_err());
        assert!(TagPath::parse("Photos.Travel.", true).is_err());
    }

    #[test]
    fn parse_rejects_slash() {
        // The slash is no longer a separator — it is just an invalid character.
        assert!(TagPath::parse("Photos/Travel", true).is_err());
    }

    #[test]
    fn parse_rejects_hyphen() {
        assert!(TagPath::parse("My-Photos", true).is_err());
    }

    #[test]
    fn parse_rejects_space() {
        assert!(TagPath::parse("My Photos", true).is_err());
    }

    #[test]
    fn parse_accepts_underscore_and_digits() {
        let t = TagPath::parse("Photos_2024.Trip_01", true).unwrap();
        assert_eq!(t.as_ltree(), "Photos_2024.Trip_01");
    }

    #[test]
    fn slugify_label_coerces_to_valid_tag() {
        assert_eq!(
            TagPath::slugify_label("dossier sans titre"),
            "dossier_sans_titre"
        );
        assert_eq!(TagPath::slugify_label("My Folder!"), "My_Folder");
        assert_eq!(TagPath::slugify_label("a   b"), "a_b"); // runs collapse
        assert_eq!(TagPath::slugify_label("  spaced  "), "spaced"); // trimmed
        assert_eq!(TagPath::slugify_label("Already_Good"), "Already_Good");
        assert_eq!(TagPath::slugify_label("***"), "untitled"); // all-invalid → default
        // The result always parses as a valid tag label.
        assert!(TagPath::parse(&TagPath::slugify_label("dossier sans titre"), true).is_ok());
    }
    #[test]
    fn parse_rejects_protected_prefix() {
        assert!(TagPath::parse("SharedToMe.Test", false).is_err());
        assert!(TagPath::parse("SharedToMe", false).is_err());
        assert!(TagPath::parse("  SharedToMe.test", false).is_err());
        assert!(TagPath::parse("SharedToMe  ", false).is_err());
    }

    // ── TagPath::from_slash ───────────────────────────────────────────────────

    #[test]
    fn from_slash_strips_leading_slash() {
        let t = TagPath::from_slash("/Photos/Travel/Alps");
        assert_eq!(t.as_ltree(), "Photos.Travel.Alps");
    }

    #[test]
    fn from_slash_no_leading_slash() {
        let t = TagPath::from_slash("Photos/Travel");
        assert_eq!(t.as_ltree(), "Photos.Travel");
    }

    // ── encode_sender_label ───────────────────────────────────────────────────

    #[test]
    fn encode_simple_domain() {
        let label = encode_sender_label("alice", "example.com");
        assert_eq!(label, "alice_AT_example_DOT_com");
    }

    #[test]
    fn encode_multi_dot_domain() {
        let label = encode_sender_label("bob", "sub.instance.org");
        assert_eq!(label, "bob_AT_sub_DOT_instance_DOT_org");
    }

    #[test]
    fn encode_username_with_underscores() {
        let label = encode_sender_label("my_user", "host.io");
        assert_eq!(label, "my_user_AT_host_DOT_io");
    }

    // ── TagPath::is_reserved_prefix ───────────────────────────────────────────

    #[test]
    fn reserved_prefix_matches_exact_and_descendants() {
        assert!(TagPath::is_reserved_prefix("SharedToMe"));
        assert!(TagPath::is_reserved_prefix("SharedToMe.alice_AT_x.Photos"));
    }

    #[test]
    fn reserved_prefix_rejects_unrelated_and_lookalikes() {
        assert!(!TagPath::is_reserved_prefix("Photos"));
        assert!(!TagPath::is_reserved_prefix("SharedToMeNot"));
        assert!(!TagPath::is_reserved_prefix("My.SharedToMe"));
    }

    // ── TagPath::shared_to_me ─────────────────────────────────────────────────

    #[test]
    fn shared_to_me_basic() {
        let original = TagPath::from_ltree("Photos.Travel.Alps");
        let shared = TagPath::shared_to_me("alice", "example.com", &original);
        assert_eq!(
            shared.as_ltree(),
            "SharedToMe.alice_AT_example_DOT_com.Photos.Travel.Alps"
        );
    }

    #[test]
    fn shared_to_me_empty_original() {
        let original = TagPath::from_ltree("");
        let shared = TagPath::shared_to_me("alice", "example.com", &original);
        assert_eq!(shared.as_ltree(), "SharedToMe.alice_AT_example_DOT_com");
    }

    // ── ancestor satisfaction (used by TaggingService::should_run) ─────────────────

    #[test]
    fn ancestor_of_self_is_not_ancestor() {
        let t = TagPath::from_ltree("Photos");
        assert!(!t.ancestors().contains(&t));
    }

    #[test]
    fn deep_tag_satisfies_ancestor_require() {
        // A picture with /Photos/Travel/Alps satisfies requires: [/Photos]
        let stored = TagPath::from_ltree("Photos.Travel.Alps");
        let required = TagPath::from_ltree("Photos");
        let satisfied = stored == required || stored.ancestors().contains(&required);
        assert!(satisfied);
    }

    // ── TagPath::fold_deepest ─────────────────────────────────────────────────

    fn paths(items: &[&str]) -> Vec<TagPath> {
        items.iter().map(|s| TagPath::from_ltree(*s)).collect()
    }

    #[test]
    fn fold_deepest_drops_ancestors() {
        let folded =
            TagPath::fold_deepest(paths(&["Photos", "Photos.Travel", "Photos.Travel.Alps"]));
        assert_eq!(folded, paths(&["Photos.Travel.Alps"]));
    }

    #[test]
    fn fold_deepest_dedups_exact() {
        let folded = TagPath::fold_deepest(paths(&["Photos.Travel", "Photos.Travel"]));
        assert_eq!(folded, paths(&["Photos.Travel"]));
    }

    #[test]
    fn fold_deepest_keeps_disjoint_branches() {
        let folded = TagPath::fold_deepest(paths(&["Photos.Travel", "Images.Icons"]));
        assert_eq!(folded, paths(&["Photos.Travel", "Images.Icons"]));
    }

    #[test]
    fn fold_deepest_keeps_siblings() {
        // Siblings under a common parent are both deepest — neither is an ancestor of the other.
        let folded = TagPath::fold_deepest(paths(&["Photos.Travel.Alps", "Photos.Travel.Jura"]));
        assert_eq!(folded, paths(&["Photos.Travel.Alps", "Photos.Travel.Jura"]));
    }
}
