//! PR #24 D-AU + D-AV + D-AW — edge case sweep.
//!
//! Stress the escape functions, flag-interaction matrix, and the
//! empty-state shape of `history.json`.

use efe_bench_harness::publish::Publisher;

// ─────────────────────────────────────────────────────────────────────
// D-AU — large input stress : escape functions must handle 64 KB in
// well under a second.  Catches accidental quadratic behaviour from
// repeated .replace() rewrites or .push_str() reallocs.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn d_au_xml_escape_handles_64kb_input_quickly() {
    use efe_bench_harness::publish::badge::xml_escape;
    let big = "S".repeat(64 * 1024);
    let start = std::time::Instant::now();
    let out = xml_escape(&big);
    let elapsed = start.elapsed();
    assert_eq!(out.len(), big.len(), "no metacharacters → identical length");
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "D-AU : xml_escape on 64KB took too long : {elapsed:?}"
    );
}

#[test]
fn d_au_xml_escape_handles_metacharacter_storm() {
    use efe_bench_harness::publish::badge::xml_escape;
    // 16K characters, every one a metacharacter that triggers escape.
    let storm = "<>&\"'".repeat(16 * 1024 / 5);
    let start = std::time::Instant::now();
    let out = xml_escape(&storm);
    let elapsed = start.elapsed();
    // Output is strictly larger than input (each char expands).
    assert!(out.len() > storm.len());
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "D-AU : xml_escape on metacharacter storm took too long : {elapsed:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// D-AV — embed_smoke flag is preserved through dry_run (the dry_run
// path constructs a staging Publisher, so we need to ensure
// embed_smoke is propagated).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn d_av_dry_run_preserves_embed_smoke_setting() {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = Publisher::new(
        tmp.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nx-dryrun-smoke"),
        "0.6.0".into(),
    );
    p.embed_smoke = false; // explicitly disable
    let manifest = p.dry_run().await.unwrap();
    // Dry-run completes ; output dir not polluted (D1 invariant).
    manifest.validate().unwrap();
    // The publisher's output_dir should still be empty even though
    // dry_run ran successfully (D-AV ensures the embed_smoke setting
    // didn't accidentally trigger a side-effect into output_dir).
    let count = std::fs::read_dir(tmp.path())
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(
        count, 0,
        "D-AV : dry-run with --embed-smoke false leaked files"
    );
}

#[tokio::test]
async fn d_av_dry_run_with_embed_smoke_true_still_returns_valid_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = Publisher::new(
        tmp.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nx-dryrun-smoke-on"),
        "0.6.0".into(),
    );
    p.embed_smoke = true;
    let manifest = p.dry_run().await.unwrap();
    manifest.validate().unwrap();
    // Same hermeticity invariant.
    let count = std::fs::read_dir(tmp.path())
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(count, 0);
}

// ─────────────────────────────────────────────────────────────────────
// D-AW — history.json with empty entries MUST still serialize with the
// schema_version field present.  An empty entries[] is acceptable
// (first publish) ; missing schema_version is NOT.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn d_aw_history_json_empty_has_schema_version_field() {
    use efe_bench_harness::publish::api::history::HistoryJson;
    let h = HistoryJson::new(vec![]);
    let json = serde_json::to_string_pretty(&h).unwrap();
    assert!(
        json.contains("\"schema_version\": 1"),
        "D-AW : empty history must still carry schema_version field :\n{json}"
    );
    assert!(
        json.contains("\"entries\": []"),
        "D-AW : empty entries must serialize as empty array, not omitted :\n{json}"
    );
}

#[test]
fn d_aw_history_json_empty_roundtrips_via_disk_byte_stable() {
    // Re-serializing the deserialized form must produce the same bytes.
    use efe_bench_harness::publish::api::history::HistoryJson;
    let h = HistoryJson::new(vec![]);
    let body = serde_json::to_vec_pretty(&h).unwrap();
    let parsed: HistoryJson = serde_json::from_slice(&body).unwrap();
    let body2 = serde_json::to_vec_pretty(&parsed).unwrap();
    assert_eq!(body, body2, "D-AW : empty history serialisation not idempotent");
}
