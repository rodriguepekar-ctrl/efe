//! Integration : full mcp-server round-trip through `commands::tenant::create`.
//!
//! Uses `mockito` to stand in for the mcp-server HTTP endpoint.

use efe_admin::auth::TokenProvider;
use efe_admin::cli::GlobalArgs;
use efe_admin::config::{AdminConfig, AuthMethod, ConfirmMode};
use efe_admin::context::AdminContext;
use efe_admin::mcp::McpClient;
use efe_admin::output::{OutputFormat, Renderer};

fn synthetic_config(url: String) -> AdminConfig {
    AdminConfig {
        cluster_alias: Some("test".into()),
        cluster_url: url,
        auth_method: AuthMethod::None,
        token_file: None,
        idp_url: None,
        client_id: None,
        output_format: OutputFormat::Json,
        confirm_mode: ConfirmMode::AlwaysYes,
    }
}

#[tokio::test]
async fn mcp_call_tool_returns_typed_value() {
    let mut s = mockito::Server::new_async().await;
    let m = s
        .mock("POST", "/mcp/tools/call")
        .with_status(200)
        .with_body(
            r#"{"result": {"tenant_id": "acme", "plan": "enterprise",
                          "max_rps": 1000, "data_residency": "EU_ONLY",
                          "created_at": "2026-05-08T00:00:00Z",
                          "initial_admin_token": "efe_aaaaaaaaaaaaaaaaaaaa"}}"#,
        )
        .create_async()
        .await;

    let client = McpClient::new(s.url(), TokenProvider::none()).unwrap();
    let res = client
        .call_tool(
            "efe_create_tenant",
            serde_json::json!({"name": "acme", "confirm": true}),
        )
        .await
        .expect("call ok");
    assert_eq!(res["tenant_id"], "acme");
    assert_eq!(res["max_rps"], 1000);
    m.assert_async().await;
}

#[tokio::test]
async fn mcp_call_tool_propagates_backend_conflict() {
    let mut s = mockito::Server::new_async().await;
    s.mock("POST", "/mcp/tools/call")
        .with_status(200)
        .with_body(
            r#"{"error": {"code": "conflict", "message": "tenant 'acme' already exists"}}"#,
        )
        .create_async()
        .await;
    let client = McpClient::new(s.url(), TokenProvider::none()).unwrap();
    let err = client
        .call_tool("efe_create_tenant", serde_json::json!({"name": "acme", "confirm": true}))
        .await
        .unwrap_err();
    match err {
        efe_admin::AdminError::Backend { code, .. } => assert_eq!(code, "conflict"),
        other => panic!("expected Backend, got {other:?}"),
    }
}

#[tokio::test]
async fn confirm_is_always_set_when_calling_write_tool() {
    // We assert the request body contains `"confirm": true` as a hard
    // invariant : every write tool call from the CLI MUST include it.
    let mut s = mockito::Server::new_async().await;
    let m = s
        .mock("POST", "/mcp/tools/call")
        .match_body(mockito::Matcher::PartialJsonString(
            r#"{"arguments": {"confirm": true}}"#.into(),
        ))
        .with_status(200)
        .with_body(r#"{"result": {"tenant_id": "acme",
                                   "ai_act_art14_acknowledged": true,
                                   "gdpr_art22_acknowledged": false,
                                   "signed_by": "x@y.com",
                                   "attested_at": "2026-05-08T00:00:00Z"}}"#)
        .create_async()
        .await;

    let cfg = synthetic_config(s.url());
    let mut g = GlobalArgs::default();
    g.yes = true;
    g.output_format = Some(OutputFormat::Json);
    let ctx = AdminContext::new(cfg, g).await.unwrap();

    let args = efe_admin::cli::ComplianceAttestArgs {
        tenant: "acme".into(),
        ai_act_art14: true,
        gdpr_art22: false,
        signed_by: "x@y.com".into(),
        dpo_contact: None,
    };
    efe_admin::commands::compliance::dispatch(
        efe_admin::cli::ComplianceArgs {
            command: efe_admin::cli::ComplianceCommand::Attest(args),
        },
        &ctx,
    )
    .await
    .expect("attest ok");
    m.assert_async().await;
}

#[test]
fn renderer_json_round_trips_a_simple_object() {
    let r = Renderer::new(OutputFormat::Json);
    #[derive(serde::Serialize)]
    struct R<'a> {
        name: &'a str,
        plan: &'a str,
    }
    let s = r.render(&R { name: "acme", plan: "enterprise" }).unwrap();
    assert!(s.contains("\"name\""));
    assert!(s.contains("\"acme\""));
    assert!(s.contains("\"plan\""));
}

#[test]
fn renderer_yaml_round_trips() {
    let r = Renderer::new(OutputFormat::Yaml);
    let s = r.render(&serde_json::json!({"a": 1, "b": "two"})).unwrap();
    assert!(s.contains("a: 1"));
    assert!(s.contains("b: two"));
}
