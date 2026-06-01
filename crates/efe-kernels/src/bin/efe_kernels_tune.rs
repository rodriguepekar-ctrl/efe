// SPDX-License-Identifier: UNLICENSED
//! `efe-kernels-tune` CLI — Sprint **Φ-D D32 (Bundle completion)**.
//!
//! Cf. `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 15
//! artefact #21 + Annexe D commande "auto-tune GPU".
//!
//! Usage :
//!
//! ```bash
//! # Auto-detect GPU compute cap + emit env exports
//! $ efe-kernels-tune
//!
//! # Force une arch specifique (= utile pour cross-compile)
//! $ efe-kernels-tune --gpu-type t4
//! $ efe-kernels-tune --gpu-type a100
//! $ efe-kernels-tune --gpu-type h100
//!
//! # Eval immédiat (= bash-friendly)
//! $ eval $(efe-kernels-tune --gpu-type t4)
//! ```
//!
//! Output format : shell `export` statements (= consumable par
//! `eval $(...)` ou `source <(...)`).
//!
//! ## Audit fix Φ-D.A4
//!
//! Sans ce binaire, les runbooks (`cuda_kernel_regression.md`) et le
//! workflow CI (`bench-cuda-kernels.yml`) référencent une commande
//! `efe-server tune` qui n'existait pas → tout fail au premier run.
//! Ce binaire est l'implémentation concrète qui consume le module
//! [`efe_kernels::tune`].

use std::process;

use efe_kernels::tune::{ComputeCap, TuneReport};

/// Exit code 0 = success, 1 = invalid args, 2 = no GPU detected.
fn main() {
    let args: Vec<String> = std::env::args().collect();

    let cap = match parse_args(&args) {
        Ok(c) => c,
        Err(ParseError::Help) => {
            print_help();
            process::exit(0);
        }
        Err(ParseError::InvalidGpuType(s)) => {
            eprintln!("Error: unknown --gpu-type `{s}`");
            eprintln!("Valid: t4 | l4 | a10 | a100 | h100 | autodetect");
            process::exit(1);
        }
    };

    let report = TuneReport::for_compute_cap(cap);

    // Banner sur stderr (= ne casse pas le `eval $(...)`)
    eprintln!(
        "# efe-kernels-tune : compute_cap = {} ({}), {} kernels tuned",
        cap,
        cap.as_sm_label(),
        report.tiles.len()
    );

    // Env exports sur stdout
    for line in report.as_env_exports() {
        println!("{line}");
    }
}

/// Erreurs de parsing CLI. `AutodetectFailed` était inclus dans
/// la livraison initiale mais jamais retourné (= la branche
/// autodetect retombe toujours sur T4 par fallback). Retiré post
/// audit Φ-D level 2 (B4).
/// Erreurs de parsing CLI. `AutodetectFailed` était inclus dans
/// la livraison initiale mais jamais retourné (= la branche
/// autodetect retombe toujours sur T4 par fallback). Retiré post
/// audit Φ-D level 2 (B4).
#[derive(Debug)]
#[allow(dead_code)]    // `Debug` derive uses fields but Rust 1.79+ flags them anyway
enum ParseError {
    Help,
    InvalidGpuType(String),
}

fn parse_args(args: &[String]) -> Result<ComputeCap, ParseError> {
    let mut i = 1;
    let mut gpu_type = "autodetect".to_string();
    while i < args.len() {
        let a = &args[i];
        if a == "-h" || a == "--help" {
            return Err(ParseError::Help);
        }
        if a == "--gpu-type" {
            i += 1;
            if i >= args.len() {
                return Err(ParseError::InvalidGpuType("(missing)".into()));
            }
            gpu_type = args[i].to_lowercase();
        }
        i += 1;
    }
    match gpu_type.as_str() {
        "t4" => Ok(ComputeCap::T4),
        "l4" => Ok(ComputeCap::L4),
        "a10" => Ok(ComputeCap::A10),
        "a100" => Ok(ComputeCap::A100),
        "h100" => Ok(ComputeCap::H100),
        "autodetect" => {
            // Sans la feature `cuda`, on ne peut pas autodetect.
            // → fall back baseline T4 + warning sur stderr.
            #[cfg(feature = "cuda")]
            {
                // Futur D32 : interroger NVML via cudarc.
                // Pour l'instant, fall back baseline T4 même avec cuda.
                eprintln!("# warning: NVML auto-detect not yet implemented (D32 final) ; using T4 baseline");
                Ok(ComputeCap::T4)
            }
            #[cfg(not(feature = "cuda"))]
            {
                eprintln!("# warning: cuda feature off ; using T4 baseline");
                Ok(ComputeCap::T4)
            }
        }
        other => Err(ParseError::InvalidGpuType(other.to_string())),
    }
}

fn print_help() {
    println!(
        "efe-kernels-tune — Auto-tune GPU kernel tile sizes per architecture\n\
         \n\
         USAGE:\n\
            efe-kernels-tune [--gpu-type <type>]\n\
         \n\
         OPTIONS:\n\
            --gpu-type <type>   t4 | l4 | a10 | a100 | h100 | autodetect (default)\n\
            -h, --help          Show this help\n\
         \n\
         EXAMPLE:\n\
            eval $(efe-kernels-tune --gpu-type t4)\n\
         \n\
         REFERENCE:\n\
            docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md § 15 artefact #21"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_t4_explicit() {
        let args = vec!["bin".into(), "--gpu-type".into(), "t4".into()];
        let cap = parse_args(&args).unwrap();
        assert_eq!(cap, ComputeCap::T4);
    }

    #[test]
    fn parse_args_a100_explicit() {
        let args = vec!["bin".into(), "--gpu-type".into(), "a100".into()];
        let cap = parse_args(&args).unwrap();
        assert_eq!(cap, ComputeCap::A100);
    }

    #[test]
    fn parse_args_case_insensitive() {
        let args = vec!["bin".into(), "--gpu-type".into(), "H100".into()];
        let cap = parse_args(&args).unwrap();
        assert_eq!(cap, ComputeCap::H100);
    }

    #[test]
    fn parse_args_help_returns_error() {
        let args = vec!["bin".into(), "--help".into()];
        assert!(matches!(parse_args(&args), Err(ParseError::Help)));
    }

    #[test]
    fn parse_args_invalid_gpu_type() {
        let args = vec!["bin".into(), "--gpu-type".into(), "tpu".into()];
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, ParseError::InvalidGpuType(_)));
    }

    #[test]
    fn parse_args_missing_value() {
        let args = vec!["bin".into(), "--gpu-type".into()];
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, ParseError::InvalidGpuType(_)));
    }

    #[test]
    fn parse_args_autodetect_falls_back_t4() {
        // Sans args, fall back baseline T4 + warning.
        let args = vec!["bin".into()];
        let cap = parse_args(&args).unwrap();
        assert_eq!(cap, ComputeCap::T4);
    }
}
