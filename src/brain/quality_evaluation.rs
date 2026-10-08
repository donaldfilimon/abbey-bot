//! Frozen synthetic lexical-support evaluation, separate from provider quality.
//! All evidence, contradiction labels and correction authority are supplied.
//! This module never calls providers, reads state, or changes policy weights.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    correction::{CorrectionDecision, evaluate_correction},
    dqn::DqnAgent,
    intent,
    reward::FeedbackAttribution,
    state::{self, StateInput},
};
use crate::grounding::{self, Grounding};

pub const SEED: u64 = 0x51a7_2026;
pub const LABEL_PROVENANCE: &str =
    "synthetic-agent-authored; pending independent human adjudication; not provider evaluation";
pub const MAX_CASES: usize = 100;
const MAX_SOURCES: usize = 8;
const MAX_SOURCE_BYTES: usize = 4096;
const MAX_CLAIM_BYTES: usize = 2048;
const MAX_TOKEN_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum QualityClass {
    Supported,
    Unsupported,
    ContradictorySource,
    Correction,
    EmptyRetrieval,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum QualityDecision {
    Accept,
    Abstain,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualitySource {
    pub id: String,
    pub revision: String,
    pub current: bool,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityClaim {
    pub text: String,
    pub cited_source_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub enum Attribution {
    ExactReply,
    UniqueScoped,
    Duplicate,
    Ambiguous,
    Expired,
    Unsupported,
}
impl From<Attribution> for FeedbackAttribution {
    fn from(value: Attribution) -> Self {
        match value {
            Attribution::ExactReply => Self::ExactReply,
            Attribution::UniqueScoped => Self::UniqueScoped,
            Attribution::Duplicate => Self::Duplicate,
            Attribution::Ambiguous => Self::Ambiguous,
            Attribution::Expired => Self::Expired,
            Attribution::Unsupported => Self::Unsupported,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionFacts {
    pub attribution: Attribution,
    pub quoted: bool,
    pub source_turn: Option<u64>,
    pub authorized: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum CorrectionLabel {
    Repair,
    Ignore,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityCase {
    pub id: String,
    pub class: QualityClass,
    pub sources: Vec<QualitySource>,
    pub claim: QualityClaim,
    pub contradictory: bool,
    pub correction: Option<CorrectionFacts>,
    pub expected_correction: Option<CorrectionLabel>,
    pub expected: QualityDecision,
    pub rationale: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityCorpus {
    pub schema: u32,
    pub label_provenance: String,
    pub cases: Vec<QualityCase>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub total: usize,
    pub true_positive: usize,
    pub false_positive: usize,
    pub true_negative: usize,
    pub false_negative: usize,
}
impl Counts {
    fn record(&mut self, expected: QualityDecision, actual: QualityDecision) {
        self.total += 1;
        match (expected, actual) {
            (QualityDecision::Accept, QualityDecision::Accept) => self.true_positive += 1,
            (QualityDecision::Abstain, QualityDecision::Accept) => self.false_positive += 1,
            (QualityDecision::Abstain, QualityDecision::Abstain) => self.true_negative += 1,
            (QualityDecision::Accept, QualityDecision::Abstain) => self.false_negative += 1,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct CorrectionCounts {
    pub repair: usize,
    pub ignore: usize,
    pub mismatches: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct QualityReport {
    pub total: Counts,
    pub by_class: BTreeMap<QualityClass, Counts>,
    pub corrections: CorrectionCounts,
    /// Untrained seeded policy over synthetic claim text, not quality scores.
    pub seed: u64,
    pub topology: Vec<usize>,
    pub action_counts: [usize; 3],
}

/// Presence is lexical support only. Invented, stale or absent citations and
/// fixture-declared contradictions always abstain, even for shape-free prose.
pub fn evaluate_case(case: &QualityCase) -> QualityDecision {
    if validate_limits(case).is_err() {
        return QualityDecision::Abstain;
    }
    let cited: Vec<_> = case
        .claim
        .cited_source_ids
        .iter()
        .filter_map(|id| case.sources.iter().find(|source| &source.id == id))
        .collect();
    let grounding = Grounding::from_sources(
        cited
            .iter()
            .filter(|source| source.current)
            .map(|source| source.text.as_str()),
    );
    let verdict = grounding::check(&case.claim.text, &grounding);
    if !case.claim.cited_source_ids.is_empty()
        && cited.len() == case.claim.cited_source_ids.len()
        && cited.iter().all(|source| source.current)
        && !case.contradictory
        && verdict.is_grounded()
    {
        QualityDecision::Accept
    } else {
        QualityDecision::Abstain
    }
}

/// The grounding scanner expands dotted version prefixes. Bound every text
/// fragment and nonwhitespace token before calling it, including direct single
/// case callers. These limits bound expansion rather than only serialized bytes.
fn validate_limits(case: &QualityCase) -> Result<(), &'static str> {
    if case.sources.len() > MAX_SOURCES || case.claim.cited_source_ids.len() > MAX_SOURCES {
        return Err("quality_case_limit");
    }
    if case.id.len() > 64
        || case.rationale.len() > 512
        || case
            .sources
            .iter()
            .any(|source| source.id.len() > 64 || source.revision.len() > 64)
        || case.claim.cited_source_ids.iter().any(|id| id.len() > 64)
    {
        return Err("quality_metadata_limit");
    }
    let bounded_text = |text: &str, bytes: usize| {
        text.len() <= bytes
            && text
                .split_whitespace()
                .all(|token| token.len() <= MAX_TOKEN_BYTES)
    };
    if !bounded_text(&case.claim.text, MAX_CLAIM_BYTES)
        || case
            .sources
            .iter()
            .any(|source| !bounded_text(&source.text, MAX_SOURCE_BYTES))
    {
        return Err("quality_text_limit");
    }
    Ok(())
}

fn validate_case(case: &QualityCase) -> Result<(), &'static str> {
    validate_limits(case)?;
    if case.id.trim().is_empty()
        || case.claim.text.trim().is_empty()
        || case.rationale.trim().is_empty()
    {
        return Err("missing_case_metadata");
    }
    let mut sources = BTreeSet::new();
    for source in &case.sources {
        if source.id.trim().is_empty()
            || source.revision.trim().is_empty()
            || source.text.trim().is_empty()
            || !sources.insert(source.id.as_str())
        {
            return Err("invalid_source_identity");
        }
    }
    let mut citations = BTreeSet::new();
    for id in &case.claim.cited_source_ids {
        if !sources.contains(id.as_str()) {
            return Err("unknown_source_reference");
        }
        if !citations.insert(id) {
            return Err("duplicate_source_reference");
        }
    }
    if (case.class == QualityClass::Correction) != case.correction.is_some()
        || case.correction.is_some() != case.expected_correction.is_some()
    {
        return Err("invalid_correction_labels");
    }
    Ok(())
}

pub fn evaluate_corpus(cases: &[QualityCase]) -> Result<QualityReport, &'static str> {
    if cases.len() != MAX_CASES {
        return Err("invalid_corpus_class_counts");
    }
    let mut class_counts = BTreeMap::new();
    for case in cases {
        *class_counts.entry(case.class).or_insert(0) += 1;
    }
    if class_counts.len() != 5 || class_counts.values().any(|&count| count != 20) {
        return Err("invalid_corpus_class_counts");
    }
    evaluate_subset(cases)
}

// Full-corpus shape is enforced at the public boundary. Small regression
// fixtures exercise the same validation and evaluation through this helper.
fn evaluate_subset(cases: &[QualityCase]) -> Result<QualityReport, &'static str> {
    if cases.len() > MAX_CASES {
        return Err("quality_case_limit");
    }
    let mut ids = BTreeSet::new();
    for case in cases {
        validate_case(case)?;
        if !ids.insert(case.id.as_str()) {
            return Err("duplicate_case_id");
        }
    }
    let mut report = QualityReport {
        total: Counts::default(),
        by_class: BTreeMap::new(),
        corrections: CorrectionCounts::default(),
        seed: SEED,
        topology: crate::runtime::TOPOLOGY.to_vec(),
        action_counts: [0; 3],
    };
    let mut policy = DqnAgent::new(&crate::runtime::TOPOLOGY, 128, SEED);
    for case in cases {
        let actual = evaluate_case(case);
        report.total.record(case.expected, actual);
        report
            .by_class
            .entry(case.class)
            .or_default()
            .record(case.expected, actual);
        if let Some(facts) = &case.correction {
            let decision = evaluate_correction(
                facts.attribution.into(),
                facts.quoted,
                facts.source_turn,
                facts.authorized,
            );
            let label = match decision {
                CorrectionDecision::Repair { .. } => {
                    report.corrections.repair += 1;
                    CorrectionLabel::Repair
                }
                CorrectionDecision::Ignore => {
                    report.corrections.ignore += 1;
                    CorrectionLabel::Ignore
                }
            };
            report.corrections.mismatches += usize::from(Some(label) != case.expected_correction);
        }
        let encoded = state::encode(&StateInput {
            text: &case.claim.text,
            intent: intent::classify(&case.claim.text),
            reputation: 0.5,
            channel_heat: 0,
            mentions_bot: false,
            has_image: false,
            hour_of_day: 12,
        });
        report.action_counts[policy.select_action(&encoded)] += 1;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::super::{
        dqn::ImportError,
        registry::{Brain, BrainRegistry, BrainStore},
        replay::Experience,
    };
    use super::*;
    const FIXTURE: &str = include_str!("../../tests/fixtures/learning-quality-v1.json");

    fn corpus() -> QualityCorpus {
        serde_json::from_str(FIXTURE).unwrap()
    }

    #[test]
    fn public_corpus_requires_all_five_complete_classes_before_evaluation() {
        let cases = corpus().cases;
        for subset in [&cases[..0], &cases[..1], &cases[..99]] {
            assert_eq!(
                evaluate_corpus(subset).err(),
                Some("invalid_corpus_class_counts")
            );
        }
        let mut wrong_class = cases.clone();
        wrong_class[1].class = QualityClass::Supported;
        assert_eq!(
            evaluate_corpus(&wrong_class).err(),
            Some("invalid_corpus_class_counts")
        );
        let mut too_many = cases.clone();
        too_many.push(cases[0].clone());
        assert_eq!(
            evaluate_corpus(&too_many).err(),
            Some("invalid_corpus_class_counts")
        );
    }

    #[test]
    fn corpus_has_five_disjoint_classes() {
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(FIXTURE.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            hash, "20237fe9320d68dd1ff60af329c308ec7f67322817e9cf86087fabd308b0fb0f",
            "corpus edits require a deliberate version and review"
        );
        let corpus = corpus();
        assert_eq!(corpus.schema, 1);
        assert_eq!(corpus.label_provenance, LABEL_PROVENANCE);
        let report = evaluate_corpus(&corpus.cases).unwrap();
        assert_eq!(report.total.total, 100);
        assert_eq!(report.by_class.len(), 5);
        assert!(report.by_class.values().all(|counts| counts.total == 20));
    }

    #[test]
    fn heldout_claims_have_support_or_abstain() {
        let cases = corpus().cases;
        let report = evaluate_corpus(&cases).unwrap();
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert_eq!(report.total.false_positive, 0);
        assert_eq!(report.total.false_negative, 0);
        assert_eq!(report.total.true_positive, 30);
        assert_eq!(report.total.true_negative, 70);
        assert_eq!(report.corrections.mismatches, 0);
        assert_eq!(report.corrections.repair, 6);
        assert_eq!(report.corrections.ignore, 14);
        assert_eq!(evaluate_corpus(&cases).unwrap(), report);
        assert_eq!(report.action_counts.iter().sum::<usize>(), 100);
    }

    #[test]
    fn invented_stale_contradictory_and_uncited_sources_abstain() {
        let mut case = corpus().cases.remove(0);
        case.claim.cited_source_ids[0] = "invented".into();
        assert_eq!(evaluate_case(&case), QualityDecision::Abstain);
        assert_eq!(
            evaluate_subset(&[case.clone()]),
            Err("unknown_source_reference")
        );
        case.claim.cited_source_ids[0] = case.sources[0].id.clone();
        case.sources[0].current = false;
        assert_eq!(evaluate_case(&case), QualityDecision::Abstain);
        case.sources[0].current = true;
        case.contradictory = true;
        assert_eq!(evaluate_case(&case), QualityDecision::Abstain);
        case.contradictory = false;
        case.claim.cited_source_ids.clear();
        assert_eq!(evaluate_case(&case), QualityDecision::Abstain);
        // The matching shape exists only in an uncited source.
        case.sources.push(QualitySource {
            id: "unrelated".into(),
            revision: "r1".into(),
            current: true,
            text: "No parser revision was supplied.".into(),
        });
        case.claim.cited_source_ids.push("unrelated".into());
        assert_eq!(evaluate_case(&case), QualityDecision::Abstain);
    }

    #[test]
    fn invalid_identity_and_labels_are_rejected() {
        let case = corpus().cases.remove(0);
        assert_eq!(
            evaluate_subset(&[case.clone(), case.clone()]),
            Err("duplicate_case_id")
        );
        let mut missing = case.clone();
        missing.id.clear();
        assert_eq!(evaluate_subset(&[missing]), Err("missing_case_metadata"));
        let mut duplicate = case.clone();
        duplicate.sources.push(case.sources[0].clone());
        assert_eq!(
            evaluate_subset(&[duplicate]),
            Err("invalid_source_identity")
        );
        let mut labels = case;
        labels.class = QualityClass::Correction;
        assert_eq!(evaluate_subset(&[labels]), Err("invalid_correction_labels"));
    }

    #[test]
    fn counts_expose_both_false_positive_and_false_negative() {
        let mut cases = corpus().cases;
        cases[0].expected = QualityDecision::Abstain;
        cases[1].expected = QualityDecision::Accept;
        let report = evaluate_corpus(&cases).unwrap();
        assert_eq!(report.total.false_positive, 1);
        assert_eq!(report.total.false_negative, 1);
        assert_eq!(report.by_class[&QualityClass::Supported].false_positive, 1);
        assert_eq!(
            report.by_class[&QualityClass::Unsupported].false_negative,
            1
        );
    }

    #[test]
    fn deterministic_seed_and_topology_import_preserve_policy() {
        let mut a = DqnAgent::new(&crate::runtime::TOPOLOGY, 128, SEED);
        let mut b = DqnAgent::new(&crate::runtime::TOPOLOGY, 128, SEED);
        let state = vec![0.5; state::STATE_DIMENSIONS];
        assert_eq!(a.export_weights(), b.export_weights());
        for _ in 0..100 {
            assert_eq!(a.select_action(&state), b.select_action(&state));
        }
        let before = a.export_weights();
        let invalid = DqnAgent::new(&[2, 3], 128, SEED).export_weights();
        assert!(matches!(
            a.import_weights(&invalid),
            Err(ImportError::TopologyMismatch { .. })
        ));
        assert_eq!(a.export_weights(), before);
        assert_eq!(a.select_action(&state), b.select_action(&state));
    }

    #[test]
    fn rollback_preserves_canonical_facts() {
        const A: &str = "discord:10";
        const B: &str = "discord:20";
        let mut stores = crate::persist::Stores::default();
        stores
            .memory
            .remember(A, "discord:7", "Synthetic amber fact", 1);
        stores
            .memory
            .remember(B, "discord:8", "Synthetic birch fact", 1);
        let facts_before = serde_json::to_vec(&stores.memory).unwrap();
        let mut registry = BrainRegistry::new(|| DqnAgent::new(&[2, 4, 3], 32, SEED), 3600);
        let baseline = registry.brain(A, &stores, 1).export_json();
        registry.brain(B, &stores, 1);
        registry.persist_all(&mut stores, 1);
        let other = stores.brains[B].clone();
        for i in 0..8 {
            registry.remember(
                A,
                Experience {
                    state: vec![0.25, 0.75],
                    action: i % 3,
                    reward: 1.0,
                    next_state: vec![0.5, 0.5],
                    done: true,
                },
            );
        }
        registry.learn_all(|scope| scope == A);
        registry.persist_all(&mut stores, 2);
        assert_ne!(stores.brains[A].snapshot_json, baseline);
        // Roll back one persisted policy and evict its loaded copy. Canonical
        // facts and the other scope must survive the real store round trip.
        registry.reset(A);
        BrainStore::save(&mut stores, A, &baseline, 0);
        let restored: crate::persist::Stores =
            serde_json::from_slice(&serde_json::to_vec(&stores).unwrap()).unwrap();
        assert_eq!(registry.brain(A, &restored, 3).export_json(), baseline);
        assert_eq!(restored.brains[B], other);
        assert_eq!(serde_json::to_vec(&restored.memory).unwrap(), facts_before);
        // The actual manager-confirmed reset shares the same preservation boundary.
        let mut stores = restored;
        let mut rewards = super::super::reward::RewardCollector::new();
        let mut social = super::super::social::SocialBrain::new();
        let report = super::super::erasure::reset_scope(
            &mut super::super::erasure::LearningEraseState {
                stores: &mut stores,
                rewards: &mut rewards,
                social: &mut social,
            },
            A,
            true,
            10,
        )
        .unwrap();
        assert!(report.aggregate_reset);
        assert!(!stores.brains.contains_key(A));
        assert_eq!(stores.brains[B], other);
        assert_eq!(serde_json::to_vec(&stores.memory).unwrap(), facts_before);
    }

    #[test]
    fn nonfinite_snapshot_import_is_atomic() {
        let mut agent = DqnAgent::new(&[2, 4, 3], 32, 41);
        agent.remember(Experience {
            state: vec![0.25, 0.75],
            action: 1,
            reward: 1.0,
            next_state: vec![0.5, 0.5],
            done: true,
        });
        let before = agent.export_weights();
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for field in 0..6 {
                let mut invalid = before.clone();
                match field {
                    0 => invalid.epsilon = value,
                    1 => invalid.layers[0].weights[0] = value,
                    2 => invalid.layers[1].biases[0] = value,
                    3 => invalid.experiences[0].reward = value,
                    4 => invalid.experiences[0].state[0] = value,
                    _ => invalid.experiences[0].next_state[1] = value,
                }
                assert_eq!(
                    agent.import_weights(&invalid),
                    Err(ImportError::NonFiniteValue),
                    "field {field}"
                );
                assert_eq!(agent.export_weights(), before, "field {field}");
            }
        }
    }

    #[test]
    fn json_nonfinite_import_leaves_the_existing_snapshot_untouched() {
        let mut agent = DqnAgent::new(&[2, 4, 3], 32, SEED);
        let before = agent.export_json();
        let mut value: serde_json::Value = serde_json::from_str(&before).unwrap();
        value["epsilon"] = serde_json::Value::Null;
        assert!(!agent.import_json(&value.to_string()));
        let overflow = before.replacen("\"epsilon\":0.1", "\"epsilon\":1e100", 1);
        assert_ne!(overflow, before);
        assert!(!agent.import_json(&overflow));
        assert_eq!(agent.export_json(), before);
    }

    #[test]
    fn excessive_grounding_text_is_refused_before_expansion() {
        let mut case = corpus().cases.remove(0);
        // Small enough to reproduce safely on the old code; a 20,000-segment
        // version below the CLI byte ceiling follows this same expansion path.
        case.sources[0].text = format!("v{}1", "1.".repeat(128));
        assert_eq!(evaluate_subset(&[case.clone()]), Err("quality_text_limit"));
        assert_eq!(evaluate_case(&case), QualityDecision::Abstain);
    }

    #[test]
    fn grounding_limits_cover_text_tokens_and_case_dimensions() {
        let original = corpus().cases.remove(0);
        let mut case = original.clone();
        case.sources[0].text = "a ".repeat(MAX_SOURCE_BYTES / 2);
        case.claim.text = "a ".repeat(MAX_CLAIM_BYTES / 2);
        assert_eq!(validate_limits(&case), Ok(()));
        case.sources[0].text.push('a');
        assert_eq!(validate_limits(&case), Err("quality_text_limit"));
        case.sources[0].text.pop();
        case.claim.text.push('a');
        assert_eq!(validate_limits(&case), Err("quality_text_limit"));
        case = original.clone();
        case.sources[0].text = "a".repeat(MAX_TOKEN_BYTES);
        assert_eq!(validate_limits(&case), Ok(()));
        case.sources[0].text.push('a');
        assert_eq!(validate_limits(&case), Err("quality_text_limit"));
        case = original.clone();
        case.sources = vec![case.sources[0].clone(); MAX_SOURCES + 1];
        assert_eq!(validate_limits(&case), Err("quality_case_limit"));
        case = original.clone();
        case.claim.cited_source_ids = vec!["source".into(); MAX_SOURCES + 1];
        assert_eq!(validate_limits(&case), Err("quality_case_limit"));
        case = original.clone();
        case.sources[0].revision = "r".repeat(65);
        assert_eq!(validate_limits(&case), Err("quality_metadata_limit"));
        assert_eq!(
            evaluate_subset(&vec![original; MAX_CASES + 1]),
            Err("quality_case_limit")
        );
    }
}
