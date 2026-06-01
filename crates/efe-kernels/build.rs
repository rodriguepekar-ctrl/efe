//! Build script for `efe-kernels` — Sprint D28.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 10.
//!
//! When the `cuda` Cargo feature is active, this script :
//!
//!   1. Detects `nvcc` in `$PATH` (or `EFE_CUDA_HOME/bin/nvcc`).
//!   2. Verifies CUDA Toolkit ≥ 12.4.
//!   3. Compiles every `.cu` file under `cuda/` into a static
//!      library `libefe_kernels_cuda.a` linked into the final crate.
//!   4. Emits `cargo:rustc-link-lib=` directives for cuBLAS, cuBLASLt,
//!      cuRAND and the CUDA runtime.
//!
//! When the `cuda` feature is **inactive** (default build), this
//! script is essentially a no-op : it does not require the CUDA
//! toolkit, lets the workspace compile on Mac/Windows, and prints
//! a `cargo:warning` so users know they are building a CPU-only
//! `efe-kernels`.
//!
//! # Environment variables consumed
//!
//! * `EFE_CUDA_ARCH` — comma-separated SM list (default `75,80,86,89,90`).
//! * `EFE_CUDA_HOME` — explicit CUDA install root.
//! * `EFE_CUDA_DEBUG` — when set, compile with `-G -lineinfo`.

#![allow(clippy::print_stderr)]

use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=EFE_CUDA_ARCH");
    println!("cargo:rerun-if-env-changed=EFE_CUDA_HOME");
    println!("cargo:rerun-if-env-changed=EFE_CUDA_DEBUG");

    if env::var_os("CARGO_FEATURE_CUDA").is_none() {
        println!(
            "cargo:warning=efe-kernels: `cuda` feature disabled — \
             building CPU-only stubs (CudaBackend::new will return \
             DeviceUnavailable)."
        );
        return;
    }

    #[cfg(feature = "cuda")]
    cuda_build::compile_all();

    #[cfg(not(feature = "cuda"))]
    {
        // Should be unreachable thanks to the env-var guard above
        // (Cargo always sets CARGO_FEATURE_CUDA when --features cuda
        // is on), but defensive guard if a Cargo internal regresses.
        eprintln!(
            "efe-kernels: feature flag `cuda` not seen by build script — \
             skipping CUDA compilation."
        );
    }
}

// ────────────────────────────────────────────────────────────────────
// `cuda` feature-gated compile logic.
//
// All references to the optional `cc` crate are confined to this
// module so that builds without the feature do not require the
// crate to be present in the dependency graph.
// ────────────────────────────────────────────────────────────────────

#[cfg(feature = "cuda")]
mod cuda_build {
    use std::env;
    use std::path::PathBuf;
    use std::process::Command;

    pub fn compile_all() {
        let cuda_home = env::var("EFE_CUDA_HOME").ok().map(PathBuf::from);
        let nvcc = locate_nvcc(cuda_home.as_deref()).unwrap_or_else(|| {
            panic!(
                "efe-kernels: `cuda` feature enabled but `nvcc` not in \
                 PATH. Either install CUDA Toolkit ≥ 12.4, set \
                 `EFE_CUDA_HOME=/opt/cuda-12.4`, or remove `--features cuda`."
            )
        });

        let (major, minor) = nvcc_version(&nvcc);
        if major < 12 || (major == 12 && minor < 4) {
            panic!(
                "efe-kernels: requires CUDA ≥ 12.4 ; found {major}.{minor} \
                 at {}",
                nvcc.display()
            );
        }
        println!(
            "cargo:warning=efe-kernels: detected CUDA {major}.{minor} at {}",
            nvcc.display()
        );

        let archs = env::var("EFE_CUDA_ARCH")
            .unwrap_or_else(|_| "75,80,86,89,90".to_string());
        let archs: Vec<&str> =
            archs.split(',').filter(|s| !s.is_empty()).collect();
        if archs.is_empty() {
            panic!("efe-kernels: EFE_CUDA_ARCH must list ≥ 1 SM (e.g. 80)");
        }

        let cu_files = collect_cu_files();
        if cu_files.is_empty() {
            panic!("efe-kernels: no `.cu` files found under cuda/ (expected ≥ 1)");
        }
        println!(
            "cargo:warning=efe-kernels: compiling {} CUDA source files for sm_{{{}}}",
            cu_files.len(),
            archs.join(",")
        );

        let pkg_major = env::var("CARGO_PKG_VERSION_MAJOR").unwrap_or_default();
        let pkg_minor = env::var("CARGO_PKG_VERSION_MINOR").unwrap_or_default();
        let pkg_patch = env::var("CARGO_PKG_VERSION_PATCH").unwrap_or_default();

        let mut build = cc::Build::new();
        build
            .compiler(&nvcc)
            .cuda(true)
            .flag("-O3")
            .flag("-Xcompiler=-fPIC")
            .flag(&format!("-DEFE_VERSION_MAJOR={pkg_major}"))
            .flag(&format!("-DEFE_VERSION_MINOR={pkg_minor}"))
            .flag(&format!("-DEFE_VERSION_PATCH={pkg_patch}"))
            .include("cuda/include");

        if env::var_os("EFE_CUDA_DEBUG").is_some() {
            build.flag("-G").flag("-lineinfo");
        }

        for arch in &archs {
            build.flag(&format!("-gencode=arch=compute_{arch},code=sm_{arch}"));
        }

        for f in &cu_files {
            build.file(f);
        }

        build.compile("efe_kernels_cuda");

        println!("cargo:rustc-link-lib=cudart");
        println!("cargo:rustc-link-lib=cublas");
        println!("cargo:rustc-link-lib=cublasLt");
        println!("cargo:rustc-link-lib=curand");

        if env::var_os("CARGO_FEATURE_NCCL").is_some() {
            println!("cargo:rustc-link-lib=nccl");
        }
    }

    fn collect_cu_files() -> Vec<PathBuf> {
        let mut acc = Vec::new();
        walk_dir(&PathBuf::from("cuda"), &mut acc);
        acc.sort();
        acc
    }

    fn walk_dir(dir: &PathBuf, acc: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return; };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk_dir(&p, acc);
            } else if p.extension().and_then(|e| e.to_str()) == Some("cu") {
                println!("cargo:rerun-if-changed={}", p.display());
                acc.push(p);
            } else if p.extension().and_then(|e| e.to_str()) == Some("cuh") {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
    }

    fn locate_nvcc(cuda_home: Option<&std::path::Path>) -> Option<PathBuf> {
        if let Some(root) = cuda_home {
            let candidate = root.join("bin").join(if cfg!(windows) {
                "nvcc.exe"
            } else {
                "nvcc"
            });
            if candidate.exists() {
                return Some(candidate);
            }
        }
        let exe = if cfg!(windows) { "nvcc.exe" } else { "nvcc" };
        if let Ok(path) = env::var("PATH") {
            let sep = if cfg!(windows) { ';' } else { ':' };
            for entry in path.split(sep) {
                let candidate = PathBuf::from(entry).join(exe);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    fn nvcc_version(nvcc: &PathBuf) -> (u32, u32) {
        let output = Command::new(nvcc)
            .arg("--version")
            .output()
            .unwrap_or_else(|e| {
                panic!("efe-kernels: failed to spawn nvcc: {e}")
            });
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(rest) = line.split("release ").nth(1) {
                let major_minor: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                let parts: Vec<&str> = major_minor.split('.').collect();
                if parts.len() >= 2 {
                    let major = parts[0].parse::<u32>().unwrap_or(0);
                    let minor = parts[1].parse::<u32>().unwrap_or(0);
                    return (major, minor);
                }
            }
        }
        panic!(
            "efe-kernels: cannot parse nvcc version output:\n{}",
            stdout
        );
    }
}
