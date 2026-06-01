//! Smoke-test integration : parse CLI args without a running backend.

use clap::Parser;
use efe_admin::cli::{Cli, Subcommand, TenantCommand};

#[test]
fn version_subcommand_parses() {
    let cli = Cli::try_parse_from(["efe-admin", "version"]).expect("parse");
    assert!(matches!(cli.command, Subcommand::Version));
}

#[test]
fn tenant_create_minimal() {
    let cli = Cli::try_parse_from(["efe-admin", "tenant", "create", "acme"]).expect("parse");
    let Subcommand::Tenant(t) = cli.command else { panic!("expected tenant") };
    let TenantCommand::Create(args) = t.command else { panic!("expected create") };
    assert_eq!(args.name, "acme");
}

#[test]
fn tenant_create_with_all_flags() {
    let cli = Cli::try_parse_from([
        "efe-admin", "tenant", "create", "acme",
        "--plan", "enterprise",
        "--max-rps", "1000",
        "--max-tokens-per-day", "1000000",
        "--data-residency", "EU_ONLY",
        "--contact-email", "ops@acme.com",
        "--yes",
    ]).expect("parse");
    let Subcommand::Tenant(t) = cli.command else { panic!() };
    let TenantCommand::Create(args) = t.command else { panic!() };
    assert_eq!(args.max_rps, 1000);
    assert_eq!(args.max_tokens_per_day, 1_000_000);
    assert_eq!(args.contact_email.as_deref(), Some("ops@acme.com"));
}

#[test]
fn data_residency_aliases() {
    // alias eu (lowercase) → EU_ONLY
    let cli = Cli::try_parse_from([
        "efe-admin", "tenant", "create", "acme", "--data-residency", "eu",
    ]).expect("parse");
    let Subcommand::Tenant(t) = cli.command else { panic!() };
    let TenantCommand::Create(args) = t.command else { panic!() };
    assert_eq!(args.data_residency.to_string(), "EU_ONLY");
}

#[test]
fn compliance_attest_requires_signer() {
    // Missing --signed-by must fail.
    let res = Cli::try_parse_from([
        "efe-admin", "compliance", "attest", "acme", "--ai-act-art14",
    ]);
    assert!(res.is_err());
}

#[test]
fn keys_create_default_classify_30d() {
    let cli = Cli::try_parse_from(["efe-admin", "keys", "create", "acme"]).expect("parse");
    let Subcommand::Keys(k) = cli.command else { panic!() };
    let efe_admin::cli::KeysCommand::Create(args) = k.command else { panic!() };
    assert_eq!(args.scope.to_string(), "classify");
    assert_eq!(args.expires_in, "30d");
}

#[test]
fn audit_query_limit_default() {
    let cli = Cli::try_parse_from(["efe-admin", "audit", "query", "acme"]).expect("parse");
    let Subcommand::Audit(a) = cli.command else { panic!() };
    let efe_admin::cli::AuditCommand::Query(args) = a.command else { panic!() };
    assert_eq!(args.limit, 50);
}

#[test]
fn global_yes_propagates() {
    let cli = Cli::try_parse_from([
        "efe-admin", "--yes", "tenant", "delete", "acme",
    ]).expect("parse");
    assert!(cli.global.yes);
}

#[test]
fn version_flag_works() {
    let res = Cli::try_parse_from(["efe-admin", "--version"]);
    // clap exits with DisplayVersion ; that's an Err but printable ok.
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
}

#[test]
fn help_works() {
    let res = Cli::try_parse_from(["efe-admin", "--help"]);
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
}
