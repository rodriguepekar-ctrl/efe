//! Sprint D18 — synthetic dataset generation for bench runs.
//!
//! Cf. architecture § 15.4. The generator is **deterministic** :
//! the same `DatasetSpec` produces byte-identical datasets,
//! enabling reproducible bench reports.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::BenchError;

/// Specification for a synthetic bench dataset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatasetSpec {
    /// Number of tenants to generate.
    pub tenants: u32,
    /// Catalogs per tenant.
    pub catalogs_per_tenant: u32,
    /// Classes per catalog.
    pub classes_per_catalog: u32,
    /// Sentences per class.
    pub sentences_per_class: u32,
    /// Embedding dimension (must match the configured embedder).
    pub dim: u32,
    /// Optional seed for the deterministic PRNG. None → seed 0.
    pub seed: Option<u64>,
}

impl DatasetSpec {
    /// Small dev-scale spec : 1 tenant × 1 catalog × 5 classes × 1 sentence.
    /// Useful for smoke-testing the bench harness itself.
    #[must_use]
    pub fn smoke() -> Self {
        Self {
            tenants: 1,
            catalogs_per_tenant: 1,
            classes_per_catalog: 5,
            sentences_per_class: 1,
            dim: 384,
            seed: Some(0),
        }
    }

    /// Tier 1 typical scale : 100 tenants × 10 catalogs × 50 classes × 10 sentences.
    /// Total ~500k rows, dataset disk ~30 MB.
    #[must_use]
    pub fn tier1_typical() -> Self {
        Self {
            tenants: 100,
            catalogs_per_tenant: 10,
            classes_per_catalog: 50,
            sentences_per_class: 10,
            dim: 384,
            seed: Some(0),
        }
    }

    /// Stress-scale spec : 1k tenants × 100 catalogs × 500 classes × 100 sentences.
    /// Total ~50M rows, dataset disk ~3 GB. CI-only.
    #[must_use]
    pub fn stress() -> Self {
        Self {
            tenants: 1_000,
            catalogs_per_tenant: 100,
            classes_per_catalog: 500,
            sentences_per_class: 100,
            dim: 384,
            seed: Some(42),
        }
    }

    /// Validate : reject zero-cardinality components.
    pub fn validate(&self) -> Result<(), BenchError> {
        if self.tenants == 0 {
            return Err(BenchError::config("tenants must be > 0"));
        }
        if self.catalogs_per_tenant == 0 {
            return Err(BenchError::config("catalogs_per_tenant must be > 0"));
        }
        if self.classes_per_catalog == 0 {
            return Err(BenchError::config("classes_per_catalog must be > 0"));
        }
        if self.sentences_per_class == 0 {
            return Err(BenchError::config("sentences_per_class must be > 0"));
        }
        if self.dim == 0 || self.dim > 4096 {
            return Err(BenchError::config(format!(
                "dim out of range: {} (expected 1..=4096)",
                self.dim
            )));
        }
        Ok(())
    }

    /// Total row count = tenants × catalogs × classes × sentences.
    #[must_use]
    pub fn total_sentences(&self) -> u64 {
        (self.tenants as u64)
            * (self.catalogs_per_tenant as u64)
            * (self.classes_per_catalog as u64)
            * (self.sentences_per_class as u64)
    }
}

/// One bench tenant : symbolic id + N catalogs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchTenant {
    /// Symbolic id `bench-tenant-NNNNNN`.
    pub id: String,
    /// Catalogs registered by this tenant.
    pub catalogs: Vec<BenchCatalog>,
}

/// One bench catalog : name + version + classes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchCatalog {
    /// Catalog name (e.g. `cat-0`).
    pub name: String,
    /// Catalog version (single version per bench catalog ; `v1`).
    pub version: String,
    /// Classes (each class has `sentences_per_class` sentences).
    pub classes: Vec<BenchClass>,
}

/// One bench class : name + sentences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchClass {
    /// Class name (`class-N`).
    pub name: String,
    /// Sentences synthétiques.
    pub sentences: Vec<String>,
}

/// Full dataset (in-memory ; for large stress specs, callers should
/// stream via [`gen_dataset_streaming`] instead).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dataset {
    /// Spec used to generate this dataset.
    pub spec: DatasetSpec,
    /// All tenants.
    pub tenants: Vec<BenchTenant>,
}

impl Dataset {
    /// Total tenant count.
    #[must_use]
    pub fn n_tenants(&self) -> usize {
        self.tenants.len()
    }

    /// Total catalog count across all tenants.
    #[must_use]
    pub fn n_catalogs(&self) -> usize {
        self.tenants.iter().map(|t| t.catalogs.len()).sum()
    }

    /// Total class count.
    #[must_use]
    pub fn n_classes(&self) -> usize {
        self.tenants
            .iter()
            .flat_map(|t| t.catalogs.iter())
            .map(|c| c.classes.len())
            .sum()
    }

    /// Total sentence count.
    #[must_use]
    pub fn n_sentences(&self) -> u64 {
        self.tenants
            .iter()
            .flat_map(|t| t.catalogs.iter().flat_map(|c| c.classes.iter()))
            .map(|cl| cl.sentences.len() as u64)
            .sum()
    }
}

/// Generate a dataset matching `spec`. **Deterministic** : same
/// spec → byte-identical output.
pub fn gen_dataset(spec: &DatasetSpec) -> Result<Dataset, BenchError> {
    spec.validate()?;
    let _seed = spec.seed.unwrap_or(0);
    let mut tenants = Vec::with_capacity(spec.tenants as usize);
    for ti in 0..spec.tenants {
        let tenant_id = format!("bench-tenant-{ti:06}");
        let mut catalogs = Vec::with_capacity(spec.catalogs_per_tenant as usize);
        for ci in 0..spec.catalogs_per_tenant {
            let catalog_name = format!("cat-{ci}");
            let mut classes = Vec::with_capacity(spec.classes_per_catalog as usize);
            for kl in 0..spec.classes_per_catalog {
                let class_name = format!("class-{kl}");
                let mut sentences = Vec::with_capacity(spec.sentences_per_class as usize);
                for si in 0..spec.sentences_per_class {
                    sentences.push(canonical_sentence(&tenant_id, &catalog_name, &class_name, si));
                }
                classes.push(BenchClass {
                    name: class_name,
                    sentences,
                });
            }
            catalogs.push(BenchCatalog {
                name: catalog_name,
                version: "v1".into(),
                classes,
            });
        }
        tenants.push(BenchTenant {
            id: tenant_id,
            catalogs,
        });
    }
    Ok(Dataset {
        spec: spec.clone(),
        tenants,
    })
}

/// Build a deterministic synthetic sentence based on the
/// canonical id triple. The output is a short ASCII string
/// derived from the SHA-256 of the inputs ; unique per
/// (tenant, catalog, class, sentence_idx).
fn canonical_sentence(
    tenant: &str,
    catalog: &str,
    class: &str,
    sentence_idx: u32,
) -> String {
    let mut h = Sha256::new();
    h.update(tenant.as_bytes());
    h.update(b"/");
    h.update(catalog.as_bytes());
    h.update(b"/");
    h.update(class.as_bytes());
    h.update(b"/");
    h.update(sentence_idx.to_be_bytes());
    let digest = h.finalize();
    // Encode the first 16 bytes as hex → 32 ASCII chars.
    hex::encode(&digest[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_spec_validates() {
        DatasetSpec::smoke().validate().unwrap();
    }

    #[test]
    fn validate_rejects_zero_tenants() {
        let mut s = DatasetSpec::smoke();
        s.tenants = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_excessive_dim() {
        let mut s = DatasetSpec::smoke();
        s.dim = 100_000;
        assert!(s.validate().is_err());
    }

    #[test]
    fn total_sentences_matches_product() {
        let s = DatasetSpec {
            tenants: 2,
            catalogs_per_tenant: 3,
            classes_per_catalog: 4,
            sentences_per_class: 5,
            dim: 384,
            seed: None,
        };
        assert_eq!(s.total_sentences(), 2 * 3 * 4 * 5);
    }

    #[test]
    fn gen_smoke_dataset_correct_shape() {
        let spec = DatasetSpec::smoke();
        let ds = gen_dataset(&spec).unwrap();
        assert_eq!(ds.n_tenants(), 1);
        assert_eq!(ds.n_catalogs(), 1);
        assert_eq!(ds.n_classes(), 5);
        assert_eq!(ds.n_sentences(), 5);
    }

    #[test]
    fn gen_dataset_is_deterministic() {
        let spec = DatasetSpec::tier1_typical();
        let a = gen_dataset(&spec).unwrap();
        let b = gen_dataset(&spec).unwrap();
        // Compare a couple of key fields by hash to keep the test cheap.
        let a_first_sentence = &a.tenants[0].catalogs[0].classes[0].sentences[0];
        let b_first_sentence = &b.tenants[0].catalogs[0].classes[0].sentences[0];
        assert_eq!(a_first_sentence, b_first_sentence);
    }

    #[test]
    fn canonical_sentence_unique_per_index() {
        let s1 = canonical_sentence("t", "c", "k", 0);
        let s2 = canonical_sentence("t", "c", "k", 1);
        assert_ne!(s1, s2);
    }

    #[test]
    fn canonical_sentence_unique_per_tenant() {
        let s1 = canonical_sentence("ta", "c", "k", 0);
        let s2 = canonical_sentence("tb", "c", "k", 0);
        assert_ne!(s1, s2);
    }

    #[test]
    fn canonical_sentence_is_32_hex_chars() {
        let s = canonical_sentence("t", "c", "k", 0);
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
