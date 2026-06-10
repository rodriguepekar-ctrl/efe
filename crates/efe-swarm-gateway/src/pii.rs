// SPDX-License-Identifier: UNLICENSED
//! **PII opt-in par tenant (G9)** — caviarde les sorties + previews (feature `pii`).
//!
//! Souveraineté : la **détection est LOCALE** (regex, aucun appel réseau → compatible
//! `egress=deny`). On ne touche PAS au détecteur Presidio d'`efe-pii` (réseau) ; on réutilise
//! seulement son **redactor pur** (`efe_pii::redactor::redact`) + la taxonomie `PolicyKind`
//! (`Off`/`Detect`/`Redact`/`Strict`, par-tenant, défaut `Off`).
//!
//! On caviarde la **présentation** (sortie renvoyée + previews live + copie au repos) ; les
//! `AgentCallRecord`/hashs restent **intacts** → `verify_swarm` re-prouve l'original.

use std::collections::HashMap;
use std::sync::OnceLock;

use efe_pii::policy::DetectedSpan;
use efe_pii::redactor::redact;
pub use efe_pii::policy::PolicyKind;
use regex::Regex;
use serde_json::Value;

/// Détecteurs PII **locaux** (compilés une fois). Conservateurs : sur-caviarder est sûr.
fn detectors() -> &'static Vec<(Regex, &'static str)> {
    static D: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    D.get_or_init(|| {
        vec![
            (Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap(), "EMAIL_ADDRESS"),
            (Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(), "US_SSN"),
            (Regex::new(r"\b(?:\d[ \-]?){13,19}\b").unwrap(), "CREDIT_CARD"),
            (Regex::new(r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b").unwrap(), "IBAN"),
            (Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap(), "IP_ADDRESS"),
            (Regex::new(r"\+?\d[\d \-.()]{8,}\d").unwrap(), "PHONE_NUMBER"),
        ]
    })
}

/// Détecte les spans PII (offsets octets) dans `text`. **Aucun réseau.**
pub fn detect_spans(text: &str) -> Vec<DetectedSpan> {
    let mut spans = Vec::new();
    for (re, entity) in detectors() {
        for m in re.find_iter(text) {
            spans.push(DetectedSpan { start: m.start(), end: m.end(), entity: (*entity).to_string(), score: 0.9 });
        }
    }
    spans
}

/// Caviarde un texte selon la politique (`Redact`/`Strict` caviardent ; `Off`/`Detect`
/// laissent inchangé). `redact` gère les chevauchements + la sûreté UTF-8.
pub fn redact_text(text: &str, policy: PolicyKind) -> String {
    match policy {
        PolicyKind::Off | PolicyKind::Detect => text.to_string(),
        PolicyKind::Redact | PolicyKind::Strict => redact(text, &detect_spans(text)),
    }
}

/// Caviarde **récursivement** les feuilles chaîne d'un JSON (sortie d'un run).
pub fn redact_json(value: &Value, policy: PolicyKind) -> Value {
    if !matches!(policy, PolicyKind::Redact | PolicyKind::Strict) {
        return value.clone();
    }
    fn walk(v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(redact(s, &detect_spans(s))),
            Value::Array(a) => Value::Array(a.iter().map(walk).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), walk(v))).collect()),
            other => other.clone(),
        }
    }
    walk(value)
}

/// Politique PII **par tenant** (défaut + surcharges). Chargée depuis l'env côté binaire.
#[derive(Debug, Clone)]
pub struct PiiConfig {
    default: PolicyKind,
    per_tenant: HashMap<String, PolicyKind>,
}

impl Default for PiiConfig {
    fn default() -> Self {
        Self { default: PolicyKind::Off, per_tenant: HashMap::new() }
    }
}

impl PiiConfig {
    pub fn new(default: PolicyKind) -> Self {
        Self { default, per_tenant: HashMap::new() }
    }
    /// Surcharge la politique d'un tenant.
    pub fn with_tenant(mut self, tenant: impl Into<String>, policy: PolicyKind) -> Self {
        self.per_tenant.insert(tenant.into(), policy);
        self
    }
    /// Politique effective d'un tenant (surcharge sinon défaut).
    pub fn policy_for(&self, tenant: &str) -> PolicyKind {
        self.per_tenant.get(tenant).copied().unwrap_or(self.default)
    }
}

/// Redactor des **previews du dashboard** (impl du trait léger d'`efe-swarm-dashboard`).
pub struct GatewayRedactor {
    policy: PolicyKind,
}
impl GatewayRedactor {
    pub fn new(policy: PolicyKind) -> Self {
        Self { policy }
    }
}
impl efe_swarm_dashboard::PreviewRedactor for GatewayRedactor {
    fn redact(&self, text: &str) -> String {
        redact_text(text, self.policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn local_detector_finds_common_pii() {
        let spans = detect_spans("contact john@example.com or 555-12-3456");
        let entities: Vec<&str> = spans.iter().map(|s| s.entity.as_str()).collect();
        assert!(entities.contains(&"EMAIL_ADDRESS"));
        assert!(entities.contains(&"US_SSN"));
    }

    #[test]
    fn redact_text_respects_policy() {
        let t = "email john@example.com";
        assert_eq!(redact_text(t, PolicyKind::Off), t, "Off = no-op");
        assert_eq!(redact_text(t, PolicyKind::Detect), t, "Detect = audit only");
        assert!(redact_text(t, PolicyKind::Redact).contains("[REDACTED:EMAIL_ADDRESS]"));
        assert!(!redact_text(t, PolicyKind::Redact).contains("john@example.com"));
    }

    #[test]
    fn redact_json_walks_string_leaves() {
        let v = json!({ "value": "reach me at jane@corp.io", "n": 42, "arr": ["call 555-12-3456"] });
        let red = redact_json(&v, PolicyKind::Redact);
        assert!(!red.to_string().contains("jane@corp.io"));
        assert!(!red.to_string().contains("555-12-3456"));
        assert_eq!(red.get("n").and_then(|x| x.as_i64()), Some(42), "non-strings intacts");
        // Off : aucune modification.
        assert_eq!(redact_json(&v, PolicyKind::Off), v);
    }

    #[test]
    fn per_tenant_policy_resolution() {
        let cfg = PiiConfig::new(PolicyKind::Off).with_tenant("clinic", PolicyKind::Redact);
        assert_eq!(cfg.policy_for("clinic"), PolicyKind::Redact);
        assert_eq!(cfg.policy_for("other"), PolicyKind::Off);
    }

    #[test]
    fn gateway_redactor_implements_preview_trait() {
        use efe_swarm_dashboard::PreviewRedactor;
        let r = GatewayRedactor::new(PolicyKind::Redact);
        assert!(r.redact("ip 10.0.0.1").contains("[REDACTED:IP_ADDRESS]"));
    }
}
