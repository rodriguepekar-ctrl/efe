//! Configuration : `~/.config/efe-admin/config.toml` + env overrides.
//!
//! Lookup order (highest precedence first) :
//!   1. CLI flag (e.g. `--cluster acme`)
//!   2. Env var (e.g. `EFE_ADMIN_CLUSTER`)
//!   3. Config file `[clusters.<alias>]` table
//!   4. Config file `[default]` table
//!   5. Hard-coded defaults

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cli::GlobalArgs;
use crate::error::AdminError;
use crate::output::OutputFormat;

const DEFAULT_CLUSTER_URL: &str = "http://localhost:8081";

#[derive(Debug, Clone)]
pub struct AdminConfig {
    pub cluster_alias: Option<String>,
    pub cluster_url: String,
    pub auth_method: AuthMethod,
    pub token_file: Option<PathBuf>,
    pub idp_url: Option<String>,
    pub client_id: Option<String>,
    pub output_format: OutputFormat,
    pub confirm_mode: ConfirmMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    None,
    Token,
    Sso,
    KubeSa,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmMode {
    Interactive,
    AlwaysYes,
    AlwaysNo,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct ConfigFile {
    #[serde(default)]
    default: ClusterEntry,
    #[serde(default)]
    clusters: BTreeMap<String, ClusterEntry>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
struct ClusterEntry {
    cluster: Option<String>,
    auth_method: Option<AuthMethod>,
    token_file: Option<String>,
    idp_url: Option<String>,
    client_id: Option<String>,
    output_format: Option<OutputFormat>,
    confirm: Option<ConfirmMode>,
}

impl AdminConfig {
    pub fn load(global: &GlobalArgs) -> Result<Self, AdminError> {
        let path = global
            .config
            .clone()
            .or_else(default_config_path)
            .filter(|p| p.exists());

        let file = match &path {
            Some(p) => {
                let raw = std::fs::read_to_string(p).map_err(|e| {
                    AdminError::Config(format!("read {}: {e}", p.display()))
                })?;
                toml::from_str::<ConfigFile>(&raw)?
            }
            None => ConfigFile::default(),
        };

        // Resolve which cluster entry to merge with defaults.
        let alias = global.cluster.clone();
        let merged = match &alias {
            Some(name) => {
                let entry = file
                    .clusters
                    .get(name)
                    .cloned()
                    .ok_or_else(|| AdminError::Config(format!("unknown cluster alias '{name}'")))?;
                merge_entries(&file.default, &entry)
            }
            None => file.default.clone(),
        };

        let cluster_url = merged
            .cluster
            .as_deref()
            .map(str::to_owned)
            .or_else(|| std::env::var("EFE_ADMIN_CLUSTER_URL").ok())
            .unwrap_or_else(|| DEFAULT_CLUSTER_URL.to_owned());

        let auth_method = merged.auth_method.unwrap_or(AuthMethod::None);

        let token_file = merged
            .token_file
            .as_deref()
            .map(|s| Ok::<_, AdminError>(PathBuf::from(expand_tilde(s)?)))
            .transpose()?;

        let output_format = global
            .output_format
            .or(merged.output_format)
            .unwrap_or(OutputFormat::Table);

        let confirm_mode = if global.yes {
            ConfirmMode::AlwaysYes
        } else {
            merged.confirm.unwrap_or(ConfirmMode::Interactive)
        };

        Ok(Self {
            cluster_alias: alias,
            cluster_url,
            auth_method,
            token_file,
            idp_url: merged.idp_url,
            client_id: merged.client_id,
            output_format,
            confirm_mode,
        })
    }
}

fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("efe-admin").join("config.toml"))
}

fn expand_tilde(s: &str) -> Result<String, AdminError> {
    shellexpand::full(s)
        .map(|c| c.into_owned())
        .map_err(|e| AdminError::Config(format!("tilde expansion: {e}")))
}

fn merge_entries(default: &ClusterEntry, override_: &ClusterEntry) -> ClusterEntry {
    macro_rules! pick {
        ($field:ident) => {
            override_.$field.clone().or_else(|| default.$field.clone())
        };
    }
    ClusterEntry {
        cluster: pick!(cluster),
        auth_method: override_.auth_method.or(default.auth_method),
        token_file: pick!(token_file),
        idp_url: pick!(idp_url),
        client_id: pick!(client_id),
        output_format: override_.output_format.or(default.output_format),
        confirm: override_.confirm.or(default.confirm),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(content: &str) -> (tempfile::NamedTempFile, PathBuf) {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        let p = f.path().to_owned();
        (f, p)
    }

    #[test]
    fn load_minimal_config_picks_defaults() {
        let cfg = AdminConfig::load(&GlobalArgs::default()).unwrap();
        // either the user's actual default config OR the built-in default.
        assert!(!cfg.cluster_url.is_empty());
    }

    #[test]
    fn cli_yes_overrides_confirm_mode() {
        let mut g = GlobalArgs::default();
        g.yes = true;
        let cfg = AdminConfig::load(&g).unwrap();
        assert_eq!(cfg.confirm_mode, ConfirmMode::AlwaysYes);
    }

    #[test]
    fn unknown_alias_errors() {
        let toml = r#"
            [default]
            cluster = "https://default.example"
            [clusters.acme]
            cluster = "https://acme.example"
        "#;
        let (_keep, path) = temp_config(toml);
        let mut g = GlobalArgs::default();
        g.config = Some(path);
        g.cluster = Some("globex".into());
        let err = AdminConfig::load(&g).unwrap_err();
        assert!(matches!(err, AdminError::Config(_)));
    }

    #[test]
    fn alias_overrides_default() {
        let toml = r#"
            [default]
            cluster = "https://default.example"
            output_format = "table"

            [clusters.acme]
            cluster = "https://acme.example"
            output_format = "json"
        "#;
        let (_keep, path) = temp_config(toml);
        let mut g = GlobalArgs::default();
        g.config = Some(path);
        g.cluster = Some("acme".into());
        let cfg = AdminConfig::load(&g).unwrap();
        assert_eq!(cfg.cluster_url, "https://acme.example");
        assert_eq!(cfg.output_format, OutputFormat::Json);
    }

    #[test]
    fn flag_format_beats_config() {
        let toml = r#"
            [default]
            cluster = "https://x"
            output_format = "table"
        "#;
        let (_keep, path) = temp_config(toml);
        let mut g = GlobalArgs::default();
        g.config = Some(path);
        g.output_format = Some(OutputFormat::Yaml);
        let cfg = AdminConfig::load(&g).unwrap();
        assert_eq!(cfg.output_format, OutputFormat::Yaml);
    }

}
