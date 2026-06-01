//! PR #24 § 12 + AC22 + D-I — end-to-end PDF roundtrip.
//!
//! Exercises the **full** publish-quarterly → evidence-verify chain
//! through the public `dispatch_*` engine entrypoints (the same code
//! paths the CLI uses).  Critical because the unit tests in
//! `publish::quarterly::sign` build PDFs in-process and bypass the
//! engine — this file proves the wiring from `clap::Args` to the
//! signed-PDF-on-disk to the verifier is intact.
//!
//! `cosign_disabled = true` keeps the test hermetic : we do not require
//! the cosign binary on the test runner.  The embedded-SHA half of the
//! chain (which does NOT depend on cosign) is fully exercised.

use std::path::PathBuf;

use efe_bench_harness::cli::{EvidenceAction, EvidenceArgs, PublishQuarterlyArgs};
use efe_bench_harness::engine::{dispatch_evidence, dispatch_publish_quarterly};
use efe_bench_harness::error::ExitCode;

fn pq_args(out: PathBuf, quarter: &str) -> PublishQuarterlyArgs {
    PublishQuarterlyArgs {
        period: quarter.to_string(),
        chart_version: Some("0.6.0".into()),
        output: out,
        cosign_identity: None,
        cosign_disabled: true,
        dry_run: false,
    }
}

#[tokio::test]
async fn ac22_publish_quarterly_then_evidence_verify_succeeds() {
    // 1.  Generate a signed (cosign-disabled, embedded-SHA only) PDF.
    let tmp = tempfile::tempdir().unwrap();
    let quarter = "Q2-2026";
    let rc = dispatch_publish_quarterly(pq_args(tmp.path().to_path_buf(), quarter))
        .await
        .expect("publish-quarterly must succeed");
    assert_eq!(rc, ExitCode::Success, "publish-quarterly returned non-zero");

    let q_dir = tmp.path().join("v0.6.0/quarterly");
    let pdf = q_dir.join(format!("{quarter}.pdf"));
    let json = q_dir.join(format!("{quarter}.json"));
    assert!(pdf.exists(), "PDF missing : {}", pdf.display());
    assert!(json.exists(), "JSON sidecar missing : {}", json.display());

    // 2.  Verify the PDF via the engine dispatcher (the same path the
    //     `evidence verify <pdf>` CLI subcommand takes).
    let verify_rc = dispatch_evidence(EvidenceArgs {
        action: EvidenceAction::Verify { pdf: pdf.clone() },
    })
    .await
    .expect("evidence verify must succeed on a fresh PDF (AC22)");
    assert_eq!(verify_rc, ExitCode::Success, "evidence verify returned non-zero");
}

#[tokio::test]
async fn ac22_tampering_with_json_sidecar_breaks_verify() {
    // Negative case : tamper the JSON sidecar AFTER publish, re-verify
    // must fail with an embedded-SHA mismatch.  This is the integrity
    // guarantee customer auditors rely on (PR #24 § 12.1).
    let tmp = tempfile::tempdir().unwrap();
    let quarter = "Q3-2026";
    dispatch_publish_quarterly(pq_args(tmp.path().to_path_buf(), quarter))
        .await
        .unwrap();
    let q_dir = tmp.path().join("v0.6.0/quarterly");
    let pdf = q_dir.join(format!("{quarter}.pdf"));
    let json = q_dir.join(format!("{quarter}.json"));

    // Tamper.
    let mut bytes = std::fs::read(&json).unwrap();
    bytes.extend_from_slice(b"\n// tampered post-publish -- auditor must catch this\n");
    std::fs::write(&json, &bytes).unwrap();

    // Verify : must fail.
    let err = dispatch_evidence(EvidenceArgs {
        action: EvidenceAction::Verify { pdf },
    })
    .await
    .expect_err("evidence verify must fail when the JSON sidecar is tampered");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("SHA mismatch") || msg.contains("Internal"),
        "expected SHA mismatch diagnostic, got : {msg}"
    );
}

#[tokio::test]
async fn ac22_dry_run_writes_nothing_no_pdf_to_verify() {
    // Dry-run must NOT emit any artefact.  Confirms the dry_run branch
    // is honoured at the engine level (defense-in-depth for T121).
    let tmp = tempfile::tempdir().unwrap();
    let mut args = pq_args(tmp.path().to_path_buf(), "Q1-2026");
    args.dry_run = true;
    let rc = dispatch_publish_quarterly(args).await.unwrap();
    assert_eq!(rc, ExitCode::Success);

    let q_dir = tmp.path().join("v0.6.0/quarterly");
    if q_dir.exists() {
        let count = std::fs::read_dir(&q_dir).map(|d| d.count()).unwrap_or(0);
        assert_eq!(count, 0, "dry-run leaked artefacts in {}", q_dir.display());
    }
}

// ─────────────────────────────────────────────────────────────────────
// D-M — verifier MUST fail hard when a `.bundle` is present alongside
// the PDF but cosign cannot be invoked.  Previously the harness would
// emit a `warn!` and exit Ok — deceiving auditors into believing the
// signature had been checked.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn d_m_verify_fails_hard_when_bundle_present_but_cosign_uninvokable() {
    // Generate a fresh signed-but-cosign-disabled PDF.
    let tmp = tempfile::tempdir().unwrap();
    let quarter = "Q4-2025";
    dispatch_publish_quarterly(pq_args(tmp.path().to_path_buf(), quarter))
        .await
        .unwrap();
    let q_dir = tmp.path().join("v0.6.0/quarterly");
    let pdf = q_dir.join(format!("{quarter}.pdf"));
    let bundle = pdf.with_extension("pdf.bundle");

    // Manually plant a junk .bundle so the verify path follows the
    // "bundle present" branch even when cosign is disabled at publish.
    std::fs::write(&bundle, b"this is not a real cosign bundle").unwrap();

    // Force cosign-uninvokable by clearing PATH inside this process'
    // child invocation.  Done by overriding the PATH env var.
    // The verifier's Command::new("cosign") will return Err on spawn
    // (binary not found) — the D-M guard turns that into a hard error.
    //
    // We cannot easily unset PATH for the in-process call, so instead we
    // assert the GENERAL invariant : *some* failure must surface when a
    // bundle is present.  If cosign IS installed on the test machine,
    // the junk bundle will be rejected → still an error.  Either way,
    // verify() MUST return Err — that is the contract.
    let err = dispatch_evidence(EvidenceArgs {
        action: EvidenceAction::Verify { pdf },
    })
    .await
    .expect_err(
        "D-M : verify must fail when a .bundle is present but cannot be cryptographically verified",
    );
    let msg = format!("{err:?}");
    assert!(
        msg.contains("cosign") || msg.contains("Internal"),
        "D-M : diagnostic must mention cosign / Internal — got : {msg}"
    );
}
