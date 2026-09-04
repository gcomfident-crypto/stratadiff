use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use stratadiff::{
    Language, analyze_bytes,
    review::{ReviewLane, ReviewPriority, classify_priority, classify_report},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    name: String,
    description: String,
    label_policy: LabelPolicy,
    known_limitations: Vec<String>,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LabelPolicy {
    syntax_preserved: String,
    behavior_sensitive: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Category {
    SyntaxPreserved,
    BehaviorSensitive,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    language: Language,
    category: Category,
    mutation_operator: String,
    before_path: String,
    after_path: String,
    before: String,
    after: String,
    expected_lane: ReviewLane,
    rationale: String,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!("../benchmarks/reviewbench-seed-v1.json")).unwrap()
}

#[test]
fn seed_corpus_is_well_formed_and_paired_across_languages() {
    let corpus = corpus();
    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.name, "reviewbench-seed-v1");
    assert!(!corpus.description.trim().is_empty());
    assert!(!corpus.label_policy.syntax_preserved.trim().is_empty());
    assert!(!corpus.label_policy.behavior_sensitive.trim().is_empty());
    assert!(corpus.known_limitations.len() >= 4);
    assert!(corpus.cases.len() >= 13);

    let mut ids = BTreeSet::new();
    let mut mutation_operators = BTreeSet::new();
    let mut categories_by_language = BTreeMap::<String, BTreeSet<&str>>::new();

    for case in &corpus.cases {
        assert!(ids.insert(&case.id), "duplicate case id: {}", case.id);
        assert!(!case.before_path.trim().is_empty(), "{}", case.id);
        assert!(!case.after_path.trim().is_empty(), "{}", case.id);
        assert_ne!(case.before, case.after, "{} has identical inputs", case.id);
        assert!(!case.rationale.trim().is_empty(), "{}", case.id);
        mutation_operators.insert(case.mutation_operator.as_str());

        let category = match case.category {
            Category::SyntaxPreserved => {
                assert_eq!(
                    case.expected_lane,
                    ReviewLane::SyntaxPreserved,
                    "{}",
                    case.id
                );
                "syntax_preserved"
            }
            Category::BehaviorSensitive => "behavior_sensitive",
        };
        categories_by_language
            .entry(format!("{:?}", case.language))
            .or_default()
            .insert(category);
    }

    assert!(categories_by_language.len() >= 7);
    assert!(
        categories_by_language
            .values()
            .filter(|categories| categories.len() == 2)
            .count()
            >= 6,
        "at least six languages must have both a control and a behavior-sensitive case"
    );
    assert!(mutation_operators.len() >= 5);
}

#[test]
fn behavior_sensitive_mutations_never_leave_the_first_pass() {
    let corpus = corpus();
    let mut behavior_sensitive_cases = 0;

    for case in corpus.cases {
        let report = analyze_bytes(
            case.before.into_bytes(),
            case.after.into_bytes(),
            case.before_path,
            case.after_path,
            case.language,
        )
        .unwrap_or_else(|error| panic!("{} failed analysis: {error:#}", case.id));
        let actual_lane = classify_report(&report);
        let actual_priority = classify_priority(&report);

        if case.category == Category::BehaviorSensitive {
            behavior_sensitive_cases += 1;
            assert_eq!(
                actual_priority,
                ReviewPriority::ReviewFirst,
                "{} escaped the first-pass queue: {}",
                case.id,
                case.rationale
            );
        }
        assert_eq!(actual_lane, case.expected_lane, "{}", case.id);
    }

    assert!(behavior_sensitive_cases >= 7);
}
