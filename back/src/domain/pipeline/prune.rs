//! Shadowed manual tag pruning (feature 39 §4): the gate history of one picture through a run, and
//! the prune plan derived from it.

use crate::domain::tag::TagPath;
use crate::domain::tagging::{GateDecision, TaggingService};

/// Where a gating tag came from — decides whether it survives a prune.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Manual,
    IncomingShare,
    /// Produced earlier in this run by the service at this index (pipeline order).
    Service(usize),
}

/// A service's prune options, indexed like the run's service list.
#[derive(Debug, Clone, Copy)]
pub struct PruneOptions<'a> {
    pub prune: bool,
    pub keep: &'a [String],
}

impl<'a> From<&'a TaggingService> for PruneOptions<'a> {
    fn from(s: &'a TaggingService) -> Self {
        Self {
            prune: s.prune_shadowed_manual_tags,
            keep: &s.keep_manual_tags,
        }
    }
}

/// Manual rows to delete and to insert for one picture (ltree text).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunePlan {
    pub remove: Vec<String>,
    pub assign: Vec<String>,
}

impl PrunePlan {
    pub fn is_empty(&self) -> bool {
        self.remove.is_empty() && self.assign.is_empty()
    }
}

/// The gating set of one picture through a run, plus every gate decision and output on the way.
#[derive(Debug, Default)]
pub struct GateHistory {
    tags: Vec<TagPath>,
    origins: Vec<Vec<Origin>>,
    gates: Vec<(usize, GateDecision)>,
    produced: Vec<(usize, Vec<TagPath>)>,
}

impl GateHistory {
    /// Seed with the run's gating base: `manual` + `incoming_share` rows.
    pub fn new(base: impl IntoIterator<Item = (TagPath, Origin)>) -> Self {
        let mut h = Self::default();
        for (tag, origin) in base {
            h.add(tag, origin);
        }
        h
    }

    fn add(&mut self, tag: TagPath, origin: Origin) {
        match self.tags.iter().position(|t| t == &tag) {
            Some(i) => self.origins[i].push(origin),
            None => {
                self.tags.push(tag);
                self.origins.push(vec![origin]);
            }
        }
    }

    /// Gate the service at `idx`, record the decision, and return whether it fired.
    pub fn gate(&mut self, idx: usize, service: &TaggingService) -> bool {
        let decision = service.gate(&self.tags);
        let fired = decision.fired();
        self.gates.push((idx, decision));
        fired
    }

    /// Record the (deepest-folded) output of the service at `idx`; downstream services gate on it.
    pub fn produce(&mut self, idx: usize, tags: Vec<TagPath>) {
        for tag in &tags {
            self.add(tag.clone(), Origin::Service(idx));
        }
        self.produced.push((idx, tags));
    }

    fn origins_of(&self, tag: &TagPath) -> &[Origin] {
        self.tags
            .iter()
            .position(|t| t == tag)
            .map(|i| self.origins[i].as_slice())
            .unwrap_or(&[])
    }

    /// The prune plan for this picture (§4.4). `opts` is indexed like the run's services.
    pub fn plan(&self, opts: &[PruneOptions]) -> PrunePlan {
        let opted = |idx: usize| opts.get(idx).is_some_and(|o| o.prune);
        let manual: Vec<&TagPath> = self
            .tags
            .iter()
            .zip(&self.origins)
            .filter(|(_, o)| o.contains(&Origin::Manual))
            .map(|(t, _)| t)
            .collect();
        // A candidate is a manual row whose exact path an opted-in service produced (§4.2).
        let candidates: Vec<&TagPath> = manual
            .iter()
            .copied()
            .filter(|m| {
                self.produced
                    .iter()
                    .any(|(idx, out)| opted(*idx) && out.contains(m))
            })
            .collect();
        if candidates.is_empty() {
            return PrunePlan::default();
        }

        // Whether `tag` still reaches the service at `before` once every candidate's manual row is
        // gone: another source holds it, or an upstream service (anchored, so still firing) emits it.
        let survives = |tag: &TagPath, before: usize| {
            self.origins_of(tag).iter().any(|o| match o {
                Origin::IncomingShare => true,
                Origin::Manual => !candidates.contains(&tag),
                Origin::Service(j) => *j < before,
            })
        };

        // Sole blockers of an otherwise-firing service are kept unreduced (§4.4 row 4).
        let mut kept: Vec<&TagPath> = Vec::new();
        for (idx, d) in &self.gates {
            if !d.enabled || d.fired() || !d.requires_met() {
                continue;
            }
            let blocking = d.excludes.iter().filter(|(_, hits)| !hits.is_empty());
            if blocking
                .clone()
                .all(|(_, hits)| !hits.iter().any(|t| survives(t, *idx)))
            {
                for (_, hits) in blocking {
                    kept.extend(candidates.iter().copied().filter(|c| hits.contains(c)));
                }
            }
        }

        let mut remove: Vec<TagPath> = Vec::new();
        let mut assign: Vec<TagPath> = Vec::new();
        for m in candidates.iter().copied().filter(|c| !kept.contains(c)) {
            // Gate arms `m` alone carries, over services that fired…
            let mut anchors: Vec<TagPath> = self
                .gates
                .iter()
                .filter(|(_, d)| d.fired())
                .flat_map(|(idx, d)| {
                    d.requires
                        .iter()
                        .filter(|(_, hits)| hits.contains(m))
                        .filter(|(_, hits)| !hits.iter().any(|t| survives(t, *idx)))
                        .map(|(arm, _)| arm.clone())
                })
                .collect();
            // …plus the keep-list floors of the services that made it a candidate (§4.5).
            for (idx, out) in &self.produced {
                if opted(*idx) && out.contains(m) {
                    anchors.extend(
                        opts[*idx]
                            .keep
                            .iter()
                            .map(|k| TagPath::from_ltree(k.as_str()))
                            .filter(|k| k.is_ancestor_or_self(m)),
                    );
                }
            }
            // All anchors are ancestors-or-self of `m`, so the deepest satisfies every other one.
            match anchors.into_iter().max_by_key(TagPath::depth) {
                Some(a) if &a == m => {}
                Some(a) => {
                    remove.push(m.clone());
                    assign.push(a);
                }
                None => remove.push(m.clone()),
            }
        }
        normalize(&manual, remove, assign)
    }
}

/// Keep the manual rows a minimal antichain (the `assign_inner` invariant): drop an anchor a
/// remaining row already implies, and never delete and re-insert the same path in one statement.
fn normalize(manual: &[&TagPath], remove: Vec<TagPath>, assign: Vec<TagPath>) -> PrunePlan {
    let remaining: Vec<&TagPath> = manual
        .iter()
        .copied()
        .filter(|m| !remove.contains(m))
        .collect();
    let mut assign = TagPath::fold_deepest(assign);
    assign.retain(|a| !remaining.iter().any(|r| a.is_ancestor_or_self(r)));
    let mut remove: Vec<TagPath> = remove.into_iter().filter(|r| !assign.contains(r)).collect();
    // A manual ancestor of a new anchor would be redundant beside it.
    remove.extend(
        remaining
            .iter()
            .filter(|r| assign.iter().any(|a| r.is_ancestor_of(a)))
            .map(|r| (*r).clone()),
    );
    assign.retain(|a| !manual.contains(&a));
    PrunePlan {
        remove: remove
            .into_iter()
            .map(|t| t.as_ltree().to_string())
            .collect(),
        assign: assign
            .into_iter()
            .map(|t| t.as_ltree().to_string())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::tagging::ServiceType;
    use chrono::NaiveDateTime;
    use uuid::Uuid;

    fn svc(requires: &[&str], excludes: &[&str]) -> TaggingService {
        let t = NaiveDateTime::default();
        TaggingService {
            id: Uuid::new_v4(),
            owner_id: Uuid::nil(),
            name: String::new(),
            service_type: ServiceType::Rule,
            requires: requires.iter().map(|s| s.to_string()).collect(),
            excludes: excludes.iter().map(|s| s.to_string()).collect(),
            enabled: true,
            position: 0,
            config: serde_json::json!({}),
            prune_shadowed_manual_tags: true,
            keep_manual_tags: vec![],
            last_invalidated_at: t,
            last_error_at: None,
            last_error_msg: None,
            created_at: t,
            updated_at: t,
        }
    }

    fn p(s: &str) -> TagPath {
        TagPath::from_ltree(s)
    }

    fn manual(paths: &[&str]) -> GateHistory {
        GateHistory::new(paths.iter().map(|s| (p(s), Origin::Manual)))
    }

    /// Gate + produce each `(service, output)` in order, as the evaluator does.
    fn run(h: &mut GateHistory, services: &[(&TaggingService, &[&str])]) {
        for (idx, (s, out)) in services.iter().enumerate() {
            if h.gate(idx, s) {
                h.produce(idx, out.iter().map(|o| p(o)).collect());
            }
        }
    }

    fn opts<'a>(services: &[&'a TaggingService]) -> Vec<PruneOptions<'a>> {
        services.iter().map(|s| PruneOptions::from(*s)).collect()
    }

    fn plan(remove: &[&str], assign: &[&str]) -> PrunePlan {
        PrunePlan {
            remove: remove.iter().map(|s| s.to_string()).collect(),
            assign: assign.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn reduces_to_the_gate_arm() {
        // §4.4 worked example: segmentation gated on Era produces Era.2026 over a manual Era.2026.
        let seg = svc(&["Era"], &[]);
        let mut h = manual(&["Era.2026"]);
        run(&mut h, &[(&seg, &["Era.2026"])]);
        assert_eq!(h.plan(&opts(&[&seg])), plan(&["Era.2026"], &["Era"]));
    }

    #[test]
    fn prunes_with_no_anchor() {
        let rule = svc(&[], &[]);
        let mut h = manual(&["Travel"]);
        run(&mut h, &[(&rule, &["Travel"])]);
        assert_eq!(h.plan(&opts(&[&rule])), plan(&["Travel"], &[]));
    }

    #[test]
    fn opt_out_prunes_nothing() {
        let mut rule = svc(&[], &[]);
        rule.prune_shadowed_manual_tags = false;
        let mut h = manual(&["Travel"]);
        run(&mut h, &[(&rule, &["Travel"])]);
        assert_eq!(h.plan(&opts(&[&rule])), PrunePlan::default());
    }

    #[test]
    fn ancestor_or_descendant_is_not_a_candidate() {
        let rule = svc(&[], &[]);
        let mut h = manual(&["Era", "Travel.Asia.Vietnam"]);
        run(&mut h, &[(&rule, &["Era.2026", "Travel.Asia"])]);
        assert_eq!(h.plan(&opts(&[&rule])), PrunePlan::default());
    }

    #[test]
    fn two_gates_at_different_depths_pick_the_deepest() {
        // Edge case 3: S1 requires Era, S2 requires Era.2026, both fired on M alone.
        let s1 = svc(&["Era"], &[]);
        let s2 = svc(&["Era.2026"], &[]);
        let producer = svc(&[], &[]);
        let mut h = manual(&["Era.2026.Vietnam"]);
        run(
            &mut h,
            &[
                (&s1, &["A"]),
                (&s2, &["B"]),
                (&producer, &["Era.2026.Vietnam"]),
            ],
        );
        assert_eq!(
            h.plan(&opts(&[&s1, &s2, &producer])),
            plan(&["Era.2026.Vietnam"], &["Era.2026"])
        );
    }

    #[test]
    fn a_gate_on_its_own_upstream_output_needs_no_anchor() {
        // S2's Era.2026 arm is also satisfied by S1's output, which keeps firing on the Era anchor.
        let s1 = svc(&["Era"], &[]);
        let s2 = svc(&["Era.2026"], &[]);
        let mut h = manual(&["Era.2026.Vietnam"]);
        run(&mut h, &[(&s1, &["Era.2026.Vietnam"]), (&s2, &[])]);
        assert_eq!(
            h.plan(&opts(&[&s1, &s2])),
            plan(&["Era.2026.Vietnam"], &["Era"])
        );
    }

    #[test]
    fn candidate_that_is_its_own_anchor_is_a_no_op() {
        // Edge case 4.
        let s = svc(&["Era.2026"], &[]);
        let mut h = manual(&["Era.2026"]);
        run(&mut h, &[(&s, &["Era.2026"])]);
        assert_eq!(h.plan(&opts(&[&s])), PrunePlan::default());
    }

    #[test]
    fn another_satisfier_frees_the_arm() {
        let s = svc(&["Era"], &[]);
        let mut h = manual(&["Era.2026", "Era.2025.Japan"]);
        run(&mut h, &[(&s, &["Era.2026"])]);
        assert_eq!(h.plan(&opts(&[&s])), plan(&["Era.2026"], &[]));
    }

    #[test]
    fn upstream_output_counts_as_a_satisfier() {
        // An earlier service also asserts the path, so the gate does not need the manual row.
        let up = svc(&[], &[]);
        let down = svc(&["Era"], &[]);
        let mut h = manual(&["Era.2026"]);
        run(&mut h, &[(&up, &["Era.2026"]), (&down, &["Era.2026"])]);
        assert_eq!(h.plan(&opts(&[&up, &down])), plan(&["Era.2026"], &[]));
    }

    #[test]
    fn competing_candidates_both_anchor() {
        // Each satisfier of the arm is itself a candidate, so neither may count on the other.
        let s = svc(&["Era"], &[]);
        let other = svc(&[], &[]);
        let mut h = manual(&["Era.2026", "Era.2025"]);
        run(&mut h, &[(&s, &["Era.2026"]), (&other, &["Era.2025"])]);
        assert_eq!(
            h.plan(&opts(&[&s, &other])),
            plan(&["Era.2026", "Era.2025"], &["Era"])
        );
    }

    #[test]
    fn not_fired_for_a_failing_requires_imposes_nothing() {
        // Edge case 5.
        let rule = svc(&[], &[]);
        let gated = svc(&["Missing"], &["Travel"]);
        let mut h = manual(&["Travel"]);
        run(&mut h, &[(&rule, &["Travel"]), (&gated, &["X"])]);
        assert_eq!(h.plan(&opts(&[&rule, &gated])), plan(&["Travel"], &[]));
    }

    #[test]
    fn sole_blocker_is_kept_unreduced() {
        // Edge case 6: removing M would let `blocked` fire.
        let rule = svc(&[], &[]);
        let blocked = svc(&[], &["Travel"]);
        let mut h = manual(&["Travel.Asia"]);
        run(&mut h, &[(&blocked, &["X"]), (&rule, &["Travel.Asia"])]);
        assert_eq!(h.plan(&opts(&[&blocked, &rule])), PrunePlan::default());
    }

    #[test]
    fn blocker_with_another_surviving_match_is_pruned() {
        let rule = svc(&[], &[]);
        let blocked = svc(&[], &["Travel"]);
        let mut h = manual(&["Travel.Asia", "Travel.Europe"]);
        run(&mut h, &[(&blocked, &["X"]), (&rule, &["Travel.Asia"])]);
        assert_eq!(
            h.plan(&opts(&[&blocked, &rule])),
            plan(&["Travel.Asia"], &[])
        );
    }

    #[test]
    fn keep_list_is_a_floor() {
        let mut seg = svc(&["Era"], &[]);
        seg.keep_manual_tags = vec!["Era.2026".to_string()];
        let mut h = manual(&["Era.2026.Vietnam"]);
        run(&mut h, &[(&seg, &["Era.2026.Vietnam"])]);
        assert_eq!(
            h.plan(&opts(&[&seg])),
            plan(&["Era.2026.Vietnam"], &["Era.2026"])
        );
    }

    #[test]
    fn keep_list_unrelated_entry_is_inert() {
        let mut rule = svc(&[], &[]);
        rule.keep_manual_tags = vec!["Other".to_string()];
        let mut h = manual(&["Travel"]);
        run(&mut h, &[(&rule, &["Travel"])]);
        assert_eq!(h.plan(&opts(&[&rule])), plan(&["Travel"], &[]));
    }

    #[test]
    fn incoming_share_rows_are_never_candidates() {
        let rule = svc(&[], &[]);
        let mut h = GateHistory::new([(p("SharedToMe.a.X"), Origin::IncomingShare)]);
        run(&mut h, &[(&rule, &["SharedToMe.a.X"])]);
        assert_eq!(h.plan(&opts(&[&rule])), PrunePlan::default());
    }

    #[test]
    fn anchor_implied_by_a_remaining_row_is_not_written() {
        let mut seg = svc(&[], &[]);
        seg.keep_manual_tags = vec!["Era".to_string()];
        let mut h = manual(&["Era.2026", "Era.2025.Japan"]);
        run(&mut h, &[(&seg, &["Era.2026"])]);
        assert_eq!(h.plan(&opts(&[&seg])), plan(&["Era.2026"], &[]));
    }
}
