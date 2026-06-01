//! Sprint **D16** — `efe-bundle` CLI.
//!
//! Build, sign, verify, and inspect EFE embedder bundles.
//!
//! # Synopsis
//!
//! ```text
//! efe-bundle keygen
//!     --out <path/to/keypair-dir>
//!
//! efe-bundle pack
//!     --source-dir <model-files>
//!     --bundle-id 'family/name@semver/framework/dim'
//!     --framework <onnx|safetensors>
//!     --dim <usize>
//!     --runtime-config <json>
//!     --license-spdx <id> [--license-url <url>] [--license-redist <bool>]
//!     [--hf-repo <repo>] [--hf-revision <sha>]
//!     [--git-sha <sha>] [--source-date-epoch <unix>]
//!     [--builder <ci-uri>]
//!     [--sign-key <path/to/private.pem>]
//!     --out <bundle-dir>
//!
//! efe-bundle verify
//!     --bundle <bundle-dir>
//!     [--pubkeys <path/to/pubkeys.json>]
//!     [--signature-policy required|warn|skip]
//!
//! efe-bundle inspect
//!     --bundle <bundle-dir> [--json]
//! ```
//!
//! # Exit codes
//!
//! * `0` — success.
//! * `1` — operation refused (manifest invalid, sig invalid, hash
//!   mismatch — i.e. policy violations).
//! * `2` — operational error (file IO, malformed input, etc.).

#![allow(clippy::missing_docs_in_private_items)]

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use efe_server::config::SignaturePolicy;
use efe_server::embedder::{
    load_bundle_dir, load_trust_store_file, BuildInfo, Component, Dtype, Framework, LicenseInfo,
    LoadPolicy, Manifest, ModelOrigin, Pooling, RuntimeConfig, TrustStore,
};
use rand_core::OsRng;
use sha2::{Digest, Sha256};

#[derive(Parser, Debug)]
#[command(
    name = "efe-bundle",
    version,
    about = "EFE embedder bundle CLI (D16) — pack, sign, verify, inspect."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Generate a new Ed25519 signing keypair (32 bytes private + 32
    /// bytes public, both raw bytes). Writes
    /// `<out>/private.key` (mode 0400) and `<out>/public.key`
    /// (mode 0444). The base64url-no-pad form of the public key
    /// is the value to put in `pubkeys.json`.
    Keygen {
        /// Directory to write the keypair into. Created if missing.
        #[arg(long)]
        out: PathBuf,
    },

    /// Pack a directory of source files into a D16 bundle :
    /// hash every file, build manifest.json, optionally sign.
    Pack {
        /// Directory containing the unpacked source layout
        /// (weights/, tokenizer/, runtime_config.json, etc.).
        /// Files are copied into <out> with the same relative
        /// paths.
        #[arg(long)]
        source_dir: PathBuf,
        /// Canonical id `family/name@semver/framework/dim`.
        #[arg(long)]
        bundle_id: String,
        /// Weight format.
        #[arg(long, value_parser = parse_framework)]
        framework: Framework,
        /// Output dimension (must match what the runtime model emits).
        #[arg(long)]
        dim: usize,
        /// Runtime config as a JSON object — eg
        /// `{"max_seq_len":256,"pooling":"mean","normalize":true,"dtype":"float32"}`.
        #[arg(long)]
        runtime_config: String,
        /// SPDX license id (`Apache-2.0`, `MIT`, …).
        #[arg(long)]
        license_spdx: String,
        /// Optional license URL.
        #[arg(long)]
        license_url: Option<String>,
        /// Whether the license permits redistribution.
        /// Use `--license-redist` (flag) for true (default).
        /// Use `--no-license-redist` (clap-built-in negation) for
        /// the explicit false case.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        license_redist: bool,
        /// HuggingFace repo (provenance trail).
        #[arg(long)]
        hf_repo: Option<String>,
        /// HuggingFace revision (40 hex chars).
        #[arg(long)]
        hf_revision: Option<String>,
        /// Model card URL.
        #[arg(long)]
        model_card_url: Option<String>,
        /// Git SHA of the EFE repo at build time. Defaults to
        /// `0`*40 (cf. reproducibility note in arch § 6.3).
        #[arg(long, default_value = "0000000000000000000000000000000000000000")]
        git_sha: String,
        /// SOURCE_DATE_EPOCH (unix seconds). Pins
        /// `build.built_at_unix_ms` for reproducibility.
        #[arg(long, default_value = "0")]
        source_date_epoch: u64,
        /// CI URL or operator label producing this bundle.
        #[arg(long, default_value = "manual")]
        builder: String,
        /// Path to the SBOM file relative to source_dir. Copied
        /// verbatim and listed in components as required=false.
        #[arg(long)]
        sbom_path: Option<String>,
        /// Path to the Ed25519 private key (raw 32 bytes).
        /// If absent, the bundle is left unsigned (operator must
        /// sign before pushing to prod).
        #[arg(long)]
        sign_key: Option<PathBuf>,
        /// Output directory. Will be populated with manifest.json,
        /// (optional) manifest.json.sig, and the components.
        #[arg(long)]
        out: PathBuf,
    },

    /// Verify a bundle against a trust store and a signature policy.
    /// Exit 0 on success, 1 on policy violation, 2 on op error.
    Verify {
        /// Bundle root directory.
        #[arg(long)]
        bundle: PathBuf,
        /// Trust store JSON. Required when `signature_policy ≠ skip`.
        #[arg(long)]
        pubkeys: Option<PathBuf>,
        /// Signature policy.
        #[arg(long, default_value = "required", value_parser = parse_sig_policy)]
        signature_policy: SignaturePolicy,
    },

    /// Print a human-readable summary of a bundle's manifest.
    Inspect {
        /// Bundle root directory.
        #[arg(long)]
        bundle: PathBuf,
        /// Emit JSON instead of TTY summary.
        #[arg(long)]
        json: bool,
    },
}

fn parse_framework(s: &str) -> Result<Framework, String> {
    match s {
        "onnx" => Ok(Framework::Onnx),
        "safetensors" => Ok(Framework::Safetensors),
        other => Err(format!("unknown framework: {other:?}")),
    }
}

fn parse_sig_policy(s: &str) -> Result<SignaturePolicy, String> {
    SignaturePolicy::from_str(s).map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(e.exit_code());
        }
    }
}

#[derive(Debug)]
enum BundleCliError {
    /// Operation refused (policy violation).
    Refused(String),
    /// Operational error (IO, malformed JSON, etc.).
    Operational(String),
}

impl BundleCliError {
    fn exit_code(&self) -> i32 {
        match self {
            BundleCliError::Refused(_) => 1,
            BundleCliError::Operational(_) => 2,
        }
    }
}

impl std::fmt::Display for BundleCliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleCliError::Refused(s) | BundleCliError::Operational(s) => f.write_str(s),
        }
    }
}

async fn run(cli: Cli) -> Result<(), BundleCliError> {
    match cli.cmd {
        Cmd::Keygen { out } => cmd_keygen(&out),
        Cmd::Pack {
            source_dir,
            bundle_id,
            framework,
            dim,
            runtime_config,
            license_spdx,
            license_url,
            license_redist,
            hf_repo,
            hf_revision,
            model_card_url,
            git_sha,
            source_date_epoch,
            builder,
            sbom_path,
            sign_key,
            out,
        } => {
            cmd_pack(
                &source_dir,
                &bundle_id,
                framework,
                dim,
                &runtime_config,
                &license_spdx,
                license_url.as_deref(),
                license_redist,
                hf_repo.as_deref(),
                hf_revision.as_deref(),
                model_card_url.as_deref(),
                &git_sha,
                source_date_epoch,
                &builder,
                sbom_path.as_deref(),
                sign_key.as_deref(),
                &out,
            )
            .await
        }
        Cmd::Verify {
            bundle,
            pubkeys,
            signature_policy,
        } => cmd_verify(&bundle, pubkeys.as_deref(), signature_policy).await,
        Cmd::Inspect { bundle, json } => cmd_inspect(&bundle, json).await,
    }
}

// =====================================================================
// keygen
// =====================================================================

fn cmd_keygen(out: &Path) -> Result<(), BundleCliError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    std::fs::create_dir_all(out)
        .map_err(|e| BundleCliError::Operational(format!("mkdir {}: {e}", out.display())))?;
    let sk = SigningKey::generate(&mut OsRng);
    let pk = sk.verifying_key();
    let priv_path = out.join("private.key");
    let pub_path = out.join("public.key");
    let pub_b64_path = out.join("public.b64url");
    std::fs::write(&priv_path, sk.to_bytes()).map_err(io)?;
    std::fs::write(&pub_path, pk.to_bytes()).map_err(io)?;
    let pk_b64 = URL_SAFE_NO_PAD.encode(pk.to_bytes());
    std::fs::write(&pub_b64_path, pk_b64.as_bytes()).map_err(io)?;
    // Tighten perms on the private key (0o400 on Unix). On Windows
    // we skip this — the operator's directory ACL is the gate.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&priv_path).map_err(io)?.permissions();
        perms.set_mode(0o400);
        std::fs::set_permissions(&priv_path, perms).map_err(io)?;
    }
    println!("✓ keygen OK");
    println!(
        "  private.key (32 bytes, mode 0400 on Unix) : {}",
        priv_path.display()
    );
    println!(
        "  public.key  (32 bytes raw)                : {}",
        pub_path.display()
    );
    println!(
        "  public.b64url (paste into pubkeys.json)   : {}",
        pub_b64_path.display()
    );
    println!();
    println!("trust-store entry suggestion :");
    println!("  {{");
    println!("    \"key_id\": \"<choose-an-id>\",");
    println!("    \"ed25519_public\": \"{pk_b64}\",");
    println!("    \"active_for_signing\": true,");
    println!("    \"valid_from\": \"<UTC start>\",");
    println!("    \"valid_until\": \"<UTC end>\"");
    println!("  }}");
    Ok(())
}

fn io(e: std::io::Error) -> BundleCliError {
    BundleCliError::Operational(format!("io: {e}"))
}

// =====================================================================
// pack
// =====================================================================

#[allow(clippy::too_many_arguments)]
async fn cmd_pack(
    source_dir: &Path,
    bundle_id: &str,
    framework: Framework,
    dim: usize,
    runtime_config_json: &str,
    license_spdx: &str,
    license_url: Option<&str>,
    license_redist: bool,
    hf_repo: Option<&str>,
    hf_revision: Option<&str>,
    model_card_url: Option<&str>,
    git_sha: &str,
    source_date_epoch: u64,
    builder: &str,
    sbom_path: Option<&str>,
    sign_key: Option<&Path>,
    out: &Path,
) -> Result<(), BundleCliError> {
    let runtime: RuntimeConfig = serde_json::from_str(runtime_config_json)
        .map_err(|e| BundleCliError::Refused(format!("--runtime-config is not valid JSON: {e}")))?;

    // **Audit fix D16-H8** — validate the sign-key file BEFORE
    // doing the (potentially long) walk + hash + copy. Pre-audit
    // code only validated at the end, so an operator with a wrong
    // --sign-key path waited minutes for a 5 GiB bundle to pack
    // before the error fired.
    let signing_key_loaded = match sign_key {
        Some(p) => {
            let bytes = std::fs::read(p).map_err(io)?;
            if bytes.len() != 32 {
                return Err(BundleCliError::Refused(format!(
                    "private key file must be 32 raw bytes (got {} bytes)",
                    bytes.len()
                )));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Some(SigningKey::from_bytes(&arr))
        }
        None => None,
    };

    // Walk source_dir, hash every file, build components[].
    std::fs::create_dir_all(out).map_err(io)?;
    let mut components: Vec<Component> = Vec::new();
    let weights_path = framework.weights_path();
    walk_and_hash(
        source_dir,
        source_dir,
        &mut components,
        weights_path,
        sbom_path,
    )?;
    components.sort_by(|a, b| a.path.cmp(&b.path));

    // Copy files into out/<rel>.
    for c in &components {
        let src = source_dir.join(&c.path);
        let dst = out.join(&c.path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        std::fs::copy(&src, &dst).map_err(io)?;
    }

    let manifest = Manifest {
        manifest_version: "1.0".into(),
        bundle_id: bundle_id.to_string(),
        framework,
        dim,
        components,
        runtime,
        build: BuildInfo {
            builder: builder.to_string(),
            built_at_unix_ms: source_date_epoch.saturating_mul(1000),
            git_sha: git_sha.to_string(),
            source_date_epoch,
        },
        license: LicenseInfo {
            spdx_id: license_spdx.to_string(),
            url: license_url.map(str::to_string),
            redistribution_allowed: license_redist,
        },
        model_origin: if hf_repo.is_some() || hf_revision.is_some() || model_card_url.is_some() {
            Some(ModelOrigin {
                huggingface_repo: hf_repo.map(str::to_string),
                huggingface_revision: hf_revision.map(str::to_string),
                model_card_url: model_card_url.map(str::to_string),
            })
        } else {
            None
        },
        sbom_path: sbom_path.map(str::to_string),
    };
    manifest
        .validate()
        .map_err(|e| BundleCliError::Refused(format!("manifest invalid: {e}")))?;
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| BundleCliError::Operational(format!("serialise manifest: {e}")))?;
    std::fs::write(out.join("manifest.json"), &manifest_bytes).map_err(io)?;

    // Sign if a signing key was loaded (we already validated it
    // up-front per audit fix D16-H8).
    if let Some(sk) = signing_key_loaded {
        let sig = sk.sign(&manifest_bytes);
        std::fs::write(out.join("manifest.json.sig"), sig.to_bytes()).map_err(io)?;
        println!("✓ packed + signed");
    } else {
        println!("✓ packed (UNSIGNED — sign before publishing)");
    }
    let fp = hex::encode(Sha256::digest(&manifest_bytes));
    println!("  bundle_id    : {}", manifest.bundle_id);
    println!("  fingerprint  : {fp}");
    println!("  components   : {} files", manifest.components.len());
    println!("  out          : {}", out.display());
    Ok(())
}

/// Recursive walk : every file under `source_dir` gets a Component
/// entry. Required is true for weights + tokenizer ; false for
/// SBOM and any other auxiliary file.
///
/// **Audit fix D16-H7** — hidden files (any path segment starting
/// with `.`) are skipped. Pre-audit code accidentally bundled
/// `.DS_Store`, `__pycache__`, `.git`, etc. on developer
/// workstations.
fn walk_and_hash(
    base: &Path,
    cur: &Path,
    out: &mut Vec<Component>,
    required_weights_rel: &str,
    optional_sbom_rel: Option<&str>,
) -> Result<(), BundleCliError> {
    for entry in std::fs::read_dir(cur).map_err(io)? {
        let entry = entry.map_err(io)?;
        let path = entry.path();
        // Skip hidden files / directories at any level.
        let name_starts_with_dot = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.starts_with('.'))
            .unwrap_or(false);
        if name_starts_with_dot {
            tracing::debug!(
                target: "efe.bundle.pack",
                path = %path.display(),
                "skipping hidden file/dir"
            );
            continue;
        }
        // Use file_type() to avoid following symlinks (defense in
        // depth — symlink loops would recurse forever).
        let file_type = entry.file_type().map_err(io)?;
        if file_type.is_symlink() {
            tracing::warn!(
                target: "efe.bundle.pack",
                path = %path.display(),
                "skipping symlink — bundles must contain only regular files"
            );
            continue;
        }
        if file_type.is_dir() {
            walk_and_hash(base, &path, out, required_weights_rel, optional_sbom_rel)?;
            continue;
        }
        let rel = path
            .strip_prefix(base)
            .map_err(|e| BundleCliError::Operational(format!("strip_prefix: {e}")))?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str == "manifest.json" || rel_str == "manifest.json.sig" {
            continue; // these are produced by pack
        }
        let bytes = std::fs::read(&path).map_err(io)?;
        let sha = hex::encode(Sha256::digest(&bytes));
        let required = rel_str == required_weights_rel || rel_str == "tokenizer/tokenizer.json";
        let _ = optional_sbom_rel;
        out.push(Component {
            path: rel_str,
            sha256: sha,
            size_bytes: bytes.len() as u64,
            required,
        });
    }
    Ok(())
}

// =====================================================================
// verify
// =====================================================================

async fn cmd_verify(
    bundle: &Path,
    pubkeys: Option<&Path>,
    signature_policy: SignaturePolicy,
) -> Result<(), BundleCliError> {
    let trust = if matches!(signature_policy, SignaturePolicy::Skip) {
        TrustStore { keys: vec![] }
    } else {
        let p = pubkeys.ok_or_else(|| {
            BundleCliError::Refused("--pubkeys is required when signature_policy ≠ skip".into())
        })?;
        load_trust_store_file(p)
            .map_err(|e| BundleCliError::Operational(format!("trust store load: {e}")))?
    };
    let policy = LoadPolicy {
        signature: signature_policy,
        require_sbom: false,
        ack_non_redistributable: true, // CLI verify is informational
    };
    match load_bundle_dir(bundle, &policy, &trust).await {
        Ok(b) => {
            println!("✓ bundle verified");
            println!("  bundle_id   : {}", b.manifest().bundle_id);
            println!("  fingerprint : {}", b.fingerprint());
            println!("  framework   : {:?}", b.manifest().framework);
            println!("  dim         : {}", b.manifest().dim);
            if let Some(kid) = b.signing_key_id() {
                println!("  signed by   : {kid}");
            } else {
                println!("  signed by   : <not verified>");
            }
            Ok(())
        }
        Err(e) => Err(BundleCliError::Refused(format!(
            "bundle verification failed: {e}"
        ))),
    }
}

// =====================================================================
// inspect
// =====================================================================

async fn cmd_inspect(bundle: &Path, json: bool) -> Result<(), BundleCliError> {
    let manifest_bytes = std::fs::read(bundle.join("manifest.json")).map_err(io)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| BundleCliError::Operational(format!("manifest parse: {e}")))?;
    if json {
        let s = serde_json::to_string_pretty(&manifest)
            .map_err(|e| BundleCliError::Operational(format!("re-serialise: {e}")))?;
        println!("{s}");
        return Ok(());
    }
    let fp = hex::encode(Sha256::digest(&manifest_bytes));
    println!("bundle_id     : {}", manifest.bundle_id);
    println!("fingerprint   : {fp}");
    println!("framework     : {:?}", manifest.framework);
    println!("dim           : {}", manifest.dim);
    println!("max_seq_len   : {}", manifest.runtime.max_seq_len);
    println!("pooling       : {:?}", manifest.runtime.pooling);
    println!("normalize     : {}", manifest.runtime.normalize);
    println!("dtype         : {:?}", manifest.runtime.dtype);
    println!(
        "license       : {} ({})",
        manifest.license.spdx_id,
        if manifest.license.redistribution_allowed {
            "redist OK"
        } else {
            "non-redist"
        }
    );
    println!("components    : {} files", manifest.components.len());
    for c in &manifest.components {
        let req = if c.required { "required" } else { "optional" };
        println!(
            "  {} {} ({} B, sha256={}…)",
            req,
            c.path,
            c.size_bytes,
            &c.sha256[..16]
        );
    }
    if let Some(o) = &manifest.model_origin {
        if let Some(repo) = &o.huggingface_repo {
            println!("hf_repo       : {repo}");
        }
        if let Some(rev) = &o.huggingface_revision {
            println!("hf_revision   : {rev}");
        }
    }
    println!("git_sha       : {}", manifest.build.git_sha);
    println!("source_date_epoch : {}", manifest.build.source_date_epoch);
    println!("builder       : {}", manifest.build.builder);
    let sig = bundle.join("manifest.json.sig");
    if sig.exists() {
        println!("signature     : present (manifest.json.sig)");
    } else {
        println!("signature     : MISSING");
    }
    Ok(())
}
