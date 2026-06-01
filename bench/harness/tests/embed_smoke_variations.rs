//! PR #24 D-AE — exhaustive `--embed-smoke` variation coverage.
//!
//! BoolishValueParser officially accepts a constellation of truthy /
//! falsy strings.  A regression in clap or our config (e.g. switching
//! to a stricter `BoolValueParser`) would silently break operator
//! muscle memory — these tests catch that.

use clap::Parser;
use efe_bench_harness::cli::{Cli, Command};

fn parse_embed_smoke_arg(values: &[&str]) -> Result<bool, String> {
    let mut argv = vec![
        "efe-bench-harness",
        "publish",
        "--output",
        "/tmp/x",
        "--source",
        "/tmp/y",
        "--chart-version",
        "0.6.0",
    ];
    argv.extend_from_slice(values);
    let cli = Cli::try_parse_from(argv).map_err(|e| e.to_string())?;
    match cli.command {
        Command::Publish(args) => Ok(args.embed_smoke),
        _ => Err("expected Publish command".into()),
    }
}

#[test]
fn d_ae_embed_smoke_truthy_variations() {
    // BoolishValueParser : true/yes/on/1 should all → true.
    for v in ["true", "yes", "on", "1"] {
        let r = parse_embed_smoke_arg(&["--embed-smoke", v])
            .unwrap_or_else(|e| panic!("D-AE : truthy '{v}' rejected : {e}"));
        assert!(r, "D-AE : '{v}' must parse as true");
    }
}

#[test]
fn d_ae_embed_smoke_falsy_variations() {
    // BoolishValueParser : false/no/off/0 should all → false.
    for v in ["false", "no", "off", "0"] {
        let r = parse_embed_smoke_arg(&["--embed-smoke", v])
            .unwrap_or_else(|e| panic!("D-AE : falsy '{v}' rejected : {e}"));
        assert!(!r, "D-AE : '{v}' must parse as false");
    }
}

#[test]
fn d_ae_embed_smoke_bare_flag_defaults_to_true() {
    let r = parse_embed_smoke_arg(&["--embed-smoke"]).unwrap();
    assert!(
        r,
        "D-AE : bare --embed-smoke must use default_missing_value=true"
    );
}

#[test]
fn d_ae_embed_smoke_absent_defaults_to_true() {
    let r = parse_embed_smoke_arg(&[]).unwrap();
    assert!(r, "D-AE : absent --embed-smoke must use default_value_t=true");
}

#[test]
fn d_ae_embed_smoke_rejects_garbage_value() {
    // BoolishValueParser rejects anything outside its boolish set.
    // Protects against typos like `--embed-smoke off-ish`.
    let r = parse_embed_smoke_arg(&["--embed-smoke", "maybe-later"]);
    assert!(
        r.is_err(),
        "D-AE : non-boolish value 'maybe-later' must be rejected by clap, got Ok"
    );
}
