//! Prompt registry — cached lookup with embedded templates.

use crate::ab_test::AbAssignmentTable;
use crate::template::{PromptTemplate, TemplateError};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

/// Registry of versioned prompt templates.
///
/// Templates are loaded at startup either from :
///   - **Embedded sources** (`include_str!`-style, default for prod) —
///     bundles templates with the binary, gives a signed reproducible
///     artifact.
///   - **Filesystem** (dev / hot-reload) — `from_dir(...)`.
///
/// A/B test assignments are stored in [`AbAssignmentTable`] and applied
/// at lookup time.
pub struct PromptRegistry {
    templates: HashMap<String, Arc<PromptTemplate>>,
    ab: RwLock<AbAssignmentTable>,
}

impl std::fmt::Debug for PromptRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptRegistry")
            .field("templates", &self.templates.keys().collect::<Vec<_>>())
            .field("ab_assignments", &self.ab.read().assignments.len())
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("template `{0}` not found in registry")]
    NotFound(String),

    #[error("template parse failure for `{name}`: {source}")]
    ParseError { name: String, source: TemplateError },

    #[error("io error reading {path}: {source}")]
    IoError {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

impl PromptRegistry {
    /// Construct an empty registry (used for tests + manual seeding).
    pub fn empty() -> Self {
        Self {
            templates: HashMap::new(),
            ab: RwLock::new(AbAssignmentTable::default()),
        }
    }

    /// Load templates from a directory (one `.md` per template).
    pub fn from_dir(dir: &Path) -> Result<Self, RegistryError> {
        let mut templates = HashMap::new();
        let entries = std::fs::read_dir(dir).map_err(|e| RegistryError::IoError {
            path: dir.display().to_string(),
            source: e,
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).map_err(|e| RegistryError::IoError {
                path: path.display().to_string(),
                source: e,
            })?;
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let tmpl = PromptTemplate::parse(&raw).map_err(|e| RegistryError::ParseError {
                name: stem.clone(),
                source: e,
            })?;
            templates.insert(stem, Arc::new(tmpl));
        }
        Ok(Self {
            templates,
            ab: RwLock::new(AbAssignmentTable::default()),
        })
    }

    /// Construct a registry from a static map (used for embedded
    /// templates with `include_str!`).
    pub fn from_static(items: Vec<(&'static str, &'static str)>) -> Result<Self, RegistryError> {
        let mut templates = HashMap::new();
        for (name, raw) in items {
            let tmpl = PromptTemplate::parse(raw).map_err(|e| RegistryError::ParseError {
                name: name.to_string(),
                source: e,
            })?;
            templates.insert(name.to_string(), Arc::new(tmpl));
        }
        Ok(Self {
            templates,
            ab: RwLock::new(AbAssignmentTable::default()),
        })
    }

    /// Override the A/B assignment table (admin endpoint will call this).
    pub fn set_ab_table(&self, table: AbAssignmentTable) {
        *self.ab.write() = table;
    }

    /// Look up a template by id, applying A/B assignment for the tenant.
    pub fn lookup(
        &self,
        base_id: &str,
        tenant_id: &str,
    ) -> Result<Arc<PromptTemplate>, RegistryError> {
        let resolved = self.ab.read().resolve(base_id, tenant_id);
        self.templates
            .get(&resolved)
            .cloned()
            .or_else(|| self.templates.get(base_id).cloned())
            .ok_or_else(|| RegistryError::NotFound(base_id.into()))
    }

    /// Iterate over all template ids loaded.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<_> = self.templates.keys().cloned().collect();
        names.sort();
        names
    }
}

impl Default for PromptRegistry {
    fn default() -> Self {
        Self::empty()
    }
}
