use stratadiff::{ChangeKind, Correspondence, Language, analyze_bytes, verify_report};

fn python(before: &str, after: &str) -> stratadiff::DiffReport {
    analyze_bytes(
        before.as_bytes().to_vec(),
        after.as_bytes().to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap()
}

#[test]
fn duplicate_subtrees_are_ambiguous_instead_of_guessed() {
    let source = "def same():\n    return 1\n\ndef same():\n    return 1\n";
    let report = python(source, source);

    assert!(report.ambiguities.iter().any(|group| {
        group.before.len() == 2
            && group.after.len() == 2
            && group
                .before
                .iter()
                .all(|node| node.kind == "function_definition")
    }));
    assert!(!report.changes.iter().any(|change| {
        matches!(change.kind, ChangeKind::Insert | ChangeKind::Delete)
            && change
                .before
                .as_ref()
                .or(change.after.as_ref())
                .is_some_and(|node| node.kind == "function_definition")
    }));
}

#[test]
fn a_unique_shape_change_is_only_a_suggestion() {
    let report = python(
        "def greet(name):\n    return 'hello ' + name\n",
        "def welcome(person):\n    return 'hello ' + person\n",
    );

    assert!(
        report
            .relations
            .iter()
            .any(|relation| relation.before.kind == "function_definition"
                && relation.correspondence == Correspondence::Suggested)
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.kind == ChangeKind::SuggestedUpdate)
    );
}

#[test]
fn reordered_unique_children_are_reported_at_the_parent() {
    let before = "def first():\n    return 1\n\ndef second():\n    return 2\n";
    let after = "def second():\n    return 2\n\ndef first():\n    return 1\n";
    let report = python(before, after);

    assert!(
        report
            .changes
            .iter()
            .any(|change| change.kind == ChangeKind::ChildOrderChanged)
    );
}

#[test]
fn trivia_only_edits_are_classified_without_losing_bytes() {
    let before = "answer=40+2\n";
    let after = "answer = 40 + 2\n";
    let report = python(before, after);

    assert!(
        report
            .changes
            .iter()
            .any(|change| change.kind == ChangeKind::FormattingOnly)
    );
    verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
}

#[test]
fn report_generation_is_deterministic() {
    let before = "def f(x):\n    return x + 1\n";
    let after = "def f(value):\n    return value + 2\n";
    assert_eq!(python(before, after), python(before, after));
}

#[test]
fn tampered_patch_fails_verification() {
    let before = "x = 1\n";
    let after = "x = 2\n";
    let mut report = python(before, after);
    report.patch.edits[0].replacement_base64 = "eA==".to_owned();

    assert!(verify_report(&report, before.as_bytes(), after.as_bytes()).is_err());
}

#[test]
fn tampered_structural_claim_fails_verification() {
    let before = "x = 1\n";
    let after = "x = 2\n";
    let mut report = python(before, after);
    report.summary.structural_changes += 1;

    assert!(verify_report(&report, before.as_bytes(), after.as_bytes()).is_err());
}

#[test]
fn tampered_ambiguity_fails_verification() {
    let source = "def same():\n    return 1\n\ndef same():\n    return 1\n";
    let mut report = python(source, source);
    report.ambiguities.clear();

    assert!(verify_report(&report, source.as_bytes(), source.as_bytes()).is_err());
}

#[test]
fn malformed_syntax_fails_clearly() {
    let result = analyze_bytes(
        b"def broken(:\n".to_vec(),
        b"def fixed():\n    pass\n".to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("refusing"));
}

#[test]
fn all_advertised_grammars_parse_and_replay() {
    let cases = [
        (Language::Javascript, "const x = 1;\n", "const x = 2;\n"),
        (
            Language::Typescript,
            "const x: number = 1;\n",
            "const x: number = 2;\n",
        ),
        (
            Language::Tsx,
            "const x = <p>a</p>;\n",
            "const x = <p>b</p>;\n",
        ),
        (
            Language::Rust,
            "fn main() { let x = 1; }\n",
            "fn main() { let x = 2; }\n",
        ),
        (
            Language::Java,
            "class Main { int value() { return 1; } }\n",
            "class Main { int value() { return 2; } }\n",
        ),
        (Language::Json, "{\"x\":1}\n", "{\"x\":2}\n"),
    ];
    for (language, before, after) in cases {
        let report = analyze_bytes(
            before.as_bytes().to_vec(),
            after.as_bytes().to_vec(),
            "before".to_owned(),
            "after".to_owned(),
            language,
        )
        .unwrap();
        verify_report(&report, before.as_bytes(), after.as_bytes()).unwrap();
    }
}

#[test]
fn excessive_tree_depth_fails_instead_of_overflowing() {
    let deeply_nested = format!("{}0{}", "[".repeat(600), "]".repeat(600));
    let result = analyze_bytes(
        deeply_nested.as_bytes().to_vec(),
        b"[]".to_vec(),
        "before.json".to_owned(),
        "after.json".to_owned(),
        Language::Json,
    );
    assert!(result.is_err());
}
