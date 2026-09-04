use std::fmt;

use anyhow::Result;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::limits::VerificationLimits;

const MAX_RELATION_EVIDENCE: usize = 4;

pub(crate) fn preflight_json_collections(bytes: &[u8], limits: &VerificationLimits) -> Result<()> {
    let mut scanner = Scanner::new(limits);
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    ObjectSeed {
        scanner: &mut scanner,
        kind: ObjectKind::Report,
    }
    .deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(())
}

struct Scanner<'a> {
    limits: &'a VerificationLimits,
    relations: usize,
    ambiguity_groups: usize,
    ambiguity_endpoints: usize,
    ambiguity_pairs: usize,
    changes: usize,
    patch_edits: usize,
}

impl<'a> Scanner<'a> {
    fn new(limits: &'a VerificationLimits) -> Self {
        Self {
            limits,
            relations: 0,
            ambiguity_groups: 0,
            ambiguity_endpoints: 0,
            ambiguity_pairs: 0,
            changes: 0,
            patch_edits: 0,
        }
    }

    fn count<E>(&mut self, kind: SequenceKind, relation_evidence: &mut usize) -> Result<(), E>
    where
        E: de::Error,
    {
        match kind {
            SequenceKind::Relations => {
                count_element(&mut self.relations, self.limits.max_relations, "relations")
            }
            SequenceKind::Ambiguities => count_element(
                &mut self.ambiguity_groups,
                self.limits.max_ambiguity_groups,
                "ambiguity groups",
            ),
            SequenceKind::Changes => {
                count_element(&mut self.changes, self.limits.max_changes, "changes")
            }
            SequenceKind::PatchEdits => count_element(
                &mut self.patch_edits,
                self.limits.max_patch_edits,
                "patch edits",
            ),
            SequenceKind::AmbiguityEndpoints => count_element(
                &mut self.ambiguity_endpoints,
                self.limits.max_ambiguity_endpoints,
                "ambiguity endpoints",
            ),
            SequenceKind::AmbiguityPairs => count_element(
                &mut self.ambiguity_pairs,
                self.limits.max_ambiguity_pairs,
                "ambiguity possible pairs",
            ),
            SequenceKind::RelationEvidence => count_element(
                relation_evidence,
                MAX_RELATION_EVIDENCE,
                "relation evidence",
            ),
        }
    }
}

fn count_element<E>(observed: &mut usize, limit: usize, label: &str) -> Result<(), E>
where
    E: de::Error,
{
    let next = observed
        .checked_add(1)
        .ok_or_else(|| E::custom(format_args!("{label} exceeds usize capacity")))?;
    if next > limit {
        return Err(E::custom(format_args!(
            "{label} limit exceeded: observed {next}, limit {limit}"
        )));
    }
    *observed = next;
    Ok(())
}

#[derive(Clone, Copy)]
enum ObjectKind {
    Report,
    Relation,
    Ambiguity,
    Constraint,
    Patch,
}

impl ObjectKind {
    fn description(self) -> &'static str {
        match self {
            Self::Report => "a StrataDiff report object",
            Self::Relation => "a relation object",
            Self::Ambiguity => "an ambiguity group object",
            Self::Constraint => "an ambiguity constraint object",
            Self::Patch => "a patch object",
        }
    }
}

#[derive(Clone, Copy)]
enum SequenceKind {
    Relations,
    Ambiguities,
    Changes,
    PatchEdits,
    AmbiguityEndpoints,
    AmbiguityPairs,
    RelationEvidence,
}

impl SequenceKind {
    fn description(self) -> &'static str {
        match self {
            Self::Relations => "the relations array",
            Self::Ambiguities => "the ambiguity groups array",
            Self::Changes => "the changes array",
            Self::PatchEdits => "the patch edits array",
            Self::AmbiguityEndpoints => "an ambiguity endpoints array",
            Self::AmbiguityPairs => "an ambiguity possible-pairs array",
            Self::RelationEvidence => "a relation evidence array",
        }
    }
}

struct ObjectSeed<'a, 'limits> {
    scanner: &'a mut Scanner<'limits>,
    kind: ObjectKind,
}

impl<'de> DeserializeSeed<'de> for ObjectSeed<'_, '_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ObjectVisitor {
            scanner: self.scanner,
            kind: self.kind,
        })
    }
}

struct ObjectVisitor<'a, 'limits> {
    scanner: &'a mut Scanner<'limits>,
    kind: ObjectKind,
}

impl<'de> Visitor<'de> for ObjectVisitor<'_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.description())
    }

    fn visit_map<A>(mut self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while let Some(field) = map.next_key::<Field>()? {
            match (self.kind, field) {
                (ObjectKind::Report, Field::Relations) => {
                    self.scan_sequence(&mut map, SequenceKind::Relations)?;
                }
                (ObjectKind::Report, Field::Ambiguities) => {
                    self.scan_sequence(&mut map, SequenceKind::Ambiguities)?;
                }
                (ObjectKind::Report, Field::Changes) => {
                    self.scan_sequence(&mut map, SequenceKind::Changes)?;
                }
                (ObjectKind::Report, Field::Patch) => {
                    self.scan_object(&mut map, ObjectKind::Patch)?;
                }
                (ObjectKind::Relation, Field::Evidence) => {
                    self.scan_sequence(&mut map, SequenceKind::RelationEvidence)?;
                }
                (ObjectKind::Ambiguity, Field::Before | Field::After) => {
                    self.scan_sequence(&mut map, SequenceKind::AmbiguityEndpoints)?;
                }
                (ObjectKind::Ambiguity, Field::Constraint) => {
                    self.scan_object(&mut map, ObjectKind::Constraint)?;
                }
                (ObjectKind::Constraint, Field::PossiblePairs) => {
                    self.scan_sequence(&mut map, SequenceKind::AmbiguityPairs)?;
                }
                (ObjectKind::Patch, Field::Edits) => {
                    self.scan_sequence(&mut map, SequenceKind::PatchEdits)?;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

impl ObjectVisitor<'_, '_> {
    fn scan_sequence<'de, A>(&mut self, map: &mut A, kind: SequenceKind) -> Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        map.next_value_seed(SequenceSeed {
            scanner: &mut *self.scanner,
            kind,
        })
    }

    fn scan_object<'de, A>(&mut self, map: &mut A, kind: ObjectKind) -> Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        map.next_value_seed(ObjectSeed {
            scanner: &mut *self.scanner,
            kind,
        })
    }
}

struct SequenceSeed<'a, 'limits> {
    scanner: &'a mut Scanner<'limits>,
    kind: SequenceKind,
}

impl<'de> DeserializeSeed<'de> for SequenceSeed<'_, '_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(SequenceVisitor {
            scanner: self.scanner,
            kind: self.kind,
        })
    }
}

struct SequenceVisitor<'a, 'limits> {
    scanner: &'a mut Scanner<'limits>,
    kind: SequenceKind,
}

impl<'de> Visitor<'de> for SequenceVisitor<'_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.description())
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut relation_evidence = 0;
        while seq
            .next_element_seed(ElementSeed {
                scanner: &mut *self.scanner,
                kind: self.kind,
                relation_evidence: &mut relation_evidence,
            })?
            .is_some()
        {}
        Ok(())
    }
}

struct ElementSeed<'a, 'limits> {
    scanner: &'a mut Scanner<'limits>,
    kind: SequenceKind,
    relation_evidence: &'a mut usize,
}

impl<'de> DeserializeSeed<'de> for ElementSeed<'_, '_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        self.scanner
            .count::<D::Error>(self.kind, self.relation_evidence)?;
        match self.kind {
            SequenceKind::Relations => ObjectSeed {
                scanner: self.scanner,
                kind: ObjectKind::Relation,
            }
            .deserialize(deserializer),
            SequenceKind::Ambiguities => ObjectSeed {
                scanner: self.scanner,
                kind: ObjectKind::Ambiguity,
            }
            .deserialize(deserializer),
            _ => <IgnoredAny as Deserialize>::deserialize(deserializer).map(drop),
        }
    }
}

#[derive(Clone, Copy)]
enum Field {
    Relations,
    Ambiguities,
    Changes,
    Patch,
    Evidence,
    Before,
    After,
    Constraint,
    PossiblePairs,
    Edits,
    Other,
}

impl<'de> Deserialize<'de> for Field {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FieldVisitor;

        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = Field;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object field name")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(match value {
                    "relations" => Field::Relations,
                    "ambiguities" => Field::Ambiguities,
                    "changes" => Field::Changes,
                    "patch" => Field::Patch,
                    "evidence" => Field::Evidence,
                    "before" => Field::Before,
                    "after" => Field::After,
                    "constraint" => Field::Constraint,
                    "possible_pairs" => Field::PossiblePairs,
                    "edits" => Field::Edits,
                    _ => Field::Other,
                })
            }
        }

        deserializer.deserialize_identifier(FieldVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_RELATION_EVIDENCE, preflight_json_collections};
    use crate::limits::VerificationLimits;
    use stratadiff_core::DiffReport;

    fn assert_limit(json: &str, limits: &VerificationLimits, expected: &str) {
        let error = preflight_json_collections(json.as_bytes(), limits).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn incomplete_report_collection_boundaries_are_enforced() {
        let limits = VerificationLimits {
            max_relations: 1,
            max_ambiguity_groups: 1,
            max_ambiguity_endpoints: 2,
            max_ambiguity_pairs: 1,
            max_changes: 1,
            max_patch_edits: 1,
            ..VerificationLimits::default()
        };

        for boundary in [
            r#"{"relations":[{}]}"#,
            r#"{"ambiguities":[{}]}"#,
            r#"{"changes":[{}]}"#,
            r#"{"patch":{"edits":[{}]}}"#,
            r#"{"ambiguities":[{"before":[{}],"after":[{}]}]}"#,
            r#"{"ambiguities":[{"constraint":{"possible_pairs":[{}]}}]}"#,
            r#"{"unknown":{"relations":[{},{}]}}"#,
        ] {
            assert!(serde_json::from_str::<DiffReport>(boundary).is_err());
            preflight_json_collections(boundary.as_bytes(), &limits).unwrap();
        }

        for (over_limit, expected) in [
            (
                r#"{"relations":[{},{}]}"#,
                "relations limit exceeded: observed 2, limit 1",
            ),
            (
                r#"{"ambiguities":[{},{}]}"#,
                "ambiguity groups limit exceeded: observed 2, limit 1",
            ),
            (
                r#"{"changes":[{},{}]}"#,
                "changes limit exceeded: observed 2, limit 1",
            ),
            (
                r#"{"patch":{"edits":[{},{}]}}"#,
                "patch edits limit exceeded: observed 2, limit 1",
            ),
            (
                r#"{"ambiguities":[{"before":[{},{}],"after":[{}]}]}"#,
                "ambiguity endpoints limit exceeded: observed 3, limit 2",
            ),
            (
                r#"{"ambiguities":[{"constraint":{"possible_pairs":[{},{}]}}]}"#,
                "ambiguity possible pairs limit exceeded: observed 2, limit 1",
            ),
        ] {
            assert_limit(over_limit, &limits, expected);
        }
    }

    #[test]
    fn relation_evidence_has_a_fixed_boundary() {
        let evidence = (0..MAX_RELATION_EVIDENCE)
            .map(|_| r#""e""#)
            .collect::<Vec<_>>()
            .join(",");
        let boundary = format!(r#"{{"relations":[{{"evidence":[{evidence}]}}]}}"#);
        assert!(serde_json::from_str::<DiffReport>(&boundary).is_err());
        preflight_json_collections(boundary.as_bytes(), &VerificationLimits::default()).unwrap();

        let over_limit = boundary.replacen("]}]", r#", "extra"]}]"#, 1);
        assert_limit(
            &over_limit,
            &VerificationLimits::default(),
            "relation evidence limit exceeded: observed 5, limit 4",
        );
    }
}
