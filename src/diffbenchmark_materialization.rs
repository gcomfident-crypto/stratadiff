use serde::{Deserialize, Serialize};

pub const DIFFBENCHMARK_REVISION: &str = "870592abd559d0bd822a27eb5c8ea45aee47015b";
pub const DIFFBENCHMARK_LITERATURE_CASES: usize = 285;
pub const MATERIALIZATION_MANIFEST_SCHEMA: &str = "stratadiff-diffbenchmark-materialization-v3";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterializationManifest {
    pub schema: String,
    pub dataset_revision: String,
    pub case_count: usize,
    pub cases: Vec<MaterializedCase>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterializedCase {
    pub oracle_path: String,
    pub oracle_blake3: String,
    pub oracle_repository_url: String,
    pub fetched_repository_url: String,
    pub commit: String,
    pub parent: String,
    pub before: MaterializedSource,
    pub after: MaterializedSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterializedSource {
    pub repository_path: String,
    pub materialized_path: String,
    pub content_blake3: String,
}
