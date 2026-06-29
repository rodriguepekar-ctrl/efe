// SPDX-License-Identifier: UNLICENSED
//! **L'ancrage** (archi S4 §6) — la pièce qui fait passer « intègre » à « **inforgeable** ». La re-preuve
//! seule atteste l'intégrité + la non-répudiation **par le détenteur de la clé** — pas que ce détenteur
//! n'a pas **fabriqué** le record. Pour fermer l'écart, le `manifest-root` est **ancré** à l'extérieur, au
//! moment de la capture (horodatage RFC 3161, log de transparence, dépôt régulateur) : « engagé au temps
//! T, avant tout litige ».
//!
//! Ce module construit la part **réelle, hors-ligne, sans crypto neuve** : une [`AnchorProof`] **structurée**
//! (auto-décrite) et [`verify_anchor`], dont le contrôle décisif est la **liaison de root** — la preuve
//! commet bien **CE** DAG (`bound_root == manifest_root`), pas un autre. La vérification **cryptographique
//! du reçu** (parser le jeton RFC 3161 / vérifier une signature) est le **raffinement nommé** : elle est
//! déléguée à l'émetteur, hors de ce composant minimal. Encode/décode faits main (pas de dépendance).

/// La méthode d'ancrage (le **type** de reçu externe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorMethod {
    /// Jeton d'horodatage RFC 3161 (autorité d'horodatage).
    Rfc3161,
    /// Reçu d'un log de transparence append-only (style Certificate Transparency).
    TransparencyLog,
    /// Dépôt périodique du root auprès du régulateur/auditeur (reçu de remise).
    RegulatorDeposit,
    /// Déclaration interne (ex. notaire interne) — **sans** force probante externe par construction.
    Declared,
}
impl AnchorMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            AnchorMethod::Rfc3161 => "rfc3161",
            AnchorMethod::TransparencyLog => "transparency-log",
            AnchorMethod::RegulatorDeposit => "regulator-deposit",
            AnchorMethod::Declared => "declared",
        }
    }
    pub fn from_label(s: &str) -> Option<AnchorMethod> {
        Some(match s {
            "rfc3161" => AnchorMethod::Rfc3161,
            "transparency-log" => AnchorMethod::TransparencyLog,
            "regulator-deposit" => AnchorMethod::RegulatorDeposit,
            "declared" => AnchorMethod::Declared,
            _ => return None,
        })
    }
    /// Cette méthode porte-t-elle un reçu **cryptographique** externe (donc un `receipt` non vide attendu) ?
    pub fn carries_crypto_receipt(self) -> bool {
        matches!(self, AnchorMethod::Rfc3161 | AnchorMethod::TransparencyLog)
    }
}

/// Une **preuve d'ancrage** structurée — ce qui est écrit dans `anchor.proof`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorProof {
    pub method: AnchorMethod,
    /// L'émetteur (étiquette de l'autorité d'ancrage — pas un secret).
    pub issuer: String,
    /// Le `manifest-root` que cette preuve **commet** (la liaison décisive).
    pub bound_root: String,
    /// Le temps T asserté (epoch).
    pub timestamp: u64,
    /// Le reçu externe brut (jeton RFC 3161 DER, reçu de log…) — porté, vérifié par l'émetteur (différé).
    pub receipt: Vec<u8>,
}

impl AnchorProof {
    /// Sérialise en `anchor.proof` (format texte déterministe, lignes `clé: valeur` ; reçu en hex).
    /// Refuse un `issuer`/`bound_root` contenant un **caractère de contrôle** (\n, \r, …) : sinon il
    /// casserait l'aller-retour de décodage ET pourrait **injecter** une ligne `clé: valeur` (ou du
    /// markdown dans le README). Le reçu binaire est, lui, toujours sûr (encodé en hex).
    pub fn encode(&self) -> Result<Vec<u8>, AnchorError> {
        for (name, val) in [("issuer", self.issuer.as_str()), ("bound_root", self.bound_root.as_str())] {
            if val.chars().any(|c| c.is_control()) {
                return Err(AnchorError::Malformed(format!("caractère de contrôle interdit dans « {name} »")));
            }
        }
        let mut s = String::new();
        s.push_str("efe-anchor:v1\n");
        s.push_str(&format!("method: {}\n", self.method.as_str()));
        s.push_str(&format!("issuer: {}\n", self.issuer));
        s.push_str(&format!("bound_root: {}\n", self.bound_root));
        s.push_str(&format!("timestamp: {}\n", self.timestamp));
        s.push_str(&format!("receipt: {}\n", to_hex(&self.receipt)));
        Ok(s.into_bytes())
    }

    /// Parse un `anchor.proof`.
    pub fn decode(bytes: &[u8]) -> Result<AnchorProof, AnchorError> {
        let text = std::str::from_utf8(bytes).map_err(|_| AnchorError::Malformed("UTF-8 invalide".into()))?;
        let mut lines = text.lines();
        match lines.next() {
            Some("efe-anchor:v1") => {}
            other => return Err(AnchorError::Malformed(format!("en-tête inattendu : {other:?}"))),
        }
        let (mut method, mut issuer, mut bound_root, mut timestamp, mut receipt) = (None, None, None, None, None);
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let (k, v) = line.split_once(": ").ok_or_else(|| AnchorError::Malformed(format!("ligne sans « : » — {line}")))?;
            // Refuse les clés DUPLIQUÉES (sinon le « dernier gagne » serait un vecteur de smuggling).
            macro_rules! once {
                ($slot:ident, $val:expr) => {{
                    if $slot.is_some() {
                        return Err(AnchorError::Malformed(format!("clé «{}» dupliquée", k)));
                    }
                    $slot = Some($val);
                }};
            }
            match k {
                "method" => once!(method, AnchorMethod::from_label(v).ok_or_else(|| AnchorError::Malformed(format!("méthode inconnue : {v}")))?),
                "issuer" => once!(issuer, v.to_string()),
                "bound_root" => once!(bound_root, v.to_string()),
                "timestamp" => once!(timestamp, v.parse::<u64>().map_err(|_| AnchorError::Malformed(format!("timestamp non numérique : {v}")))?),
                "receipt" => once!(receipt, from_hex(v).ok_or_else(|| AnchorError::Malformed("reçu hex invalide".into()))?),
                _ => {} // champ inconnu ignoré (extensibilité avant)
            }
        }
        Ok(AnchorProof {
            method: method.ok_or_else(|| AnchorError::Malformed("method manquant".into()))?,
            issuer: issuer.ok_or_else(|| AnchorError::Malformed("issuer manquant".into()))?,
            bound_root: bound_root.ok_or_else(|| AnchorError::Malformed("bound_root manquant".into()))?,
            timestamp: timestamp.ok_or_else(|| AnchorError::Malformed("timestamp manquant".into()))?,
            receipt: receipt.unwrap_or_default(),
        })
    }
}

/// Erreur d'ancrage (parsing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorError {
    Malformed(String),
}
impl std::fmt::Display for AnchorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnchorError::Malformed(m) => write!(f, "preuve d'ancrage malformée : {m}"),
        }
    }
}
impl std::error::Error for AnchorError {}

/// Le verdict d'un contrôle d'ancrage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorOutcome {
    /// Liaison de root + bonne forme OK. La vérif **cryptographique du reçu** (RFC 3161 / signature) reste
    /// déléguée à l'émetteur (raffinement nommé) — `crypto_receipt_checked: false`.
    Bound { crypto_receipt_checked: bool },
    /// La preuve commet un **autre** root que celui attendu — ancrage non probant pour CE DAG.
    RootMismatch { bound: String, expected: String },
    /// Preuve mal formée (champ vide, reçu manquant pour une méthode crypto…).
    Malformed(String),
}
impl AnchorOutcome {
    pub fn is_bound(&self) -> bool {
        matches!(self, AnchorOutcome::Bound { .. })
    }
}

/// **Vérifie l'ancrage, hors-ligne** : (1) la **liaison de root** — `proof.bound_root == expected_root`
/// (le seul contrôle cryptographiquement signifiant et faisable ici : la preuve commet bien CE DAG) ; (2)
/// la **bonne forme** (émetteur non vide, timestamp > 0, reçu présent pour une méthode crypto). La
/// vérification du **contenu** du reçu (DER RFC 3161 / signature) est **déléguée à l'émetteur** — différée.
pub fn verify_anchor(proof: &AnchorProof, expected_root: &str) -> AnchorOutcome {
    if proof.bound_root != expected_root {
        return AnchorOutcome::RootMismatch { bound: proof.bound_root.clone(), expected: expected_root.to_string() };
    }
    if proof.issuer.trim().is_empty() {
        return AnchorOutcome::Malformed("émetteur vide".into());
    }
    if proof.issuer.chars().any(|c| c.is_control()) || proof.bound_root.chars().any(|c| c.is_control()) {
        return AnchorOutcome::Malformed("caractère de contrôle dans issuer/bound_root".into());
    }
    if proof.timestamp == 0 {
        return AnchorOutcome::Malformed("timestamp nul".into());
    }
    if proof.method.carries_crypto_receipt() && proof.receipt.is_empty() {
        return AnchorOutcome::Malformed(format!("reçu vide pour la méthode {}", proof.method.as_str()));
    }
    AnchorOutcome::Bound { crypto_receipt_checked: false }
}

// ── hex (zéro dépendance) ──
fn to_hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}
fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proof(root: &str) -> AnchorProof {
        AnchorProof { method: AnchorMethod::Rfc3161, issuer: "acme-tsa".into(), bound_root: root.into(), timestamp: 1_700_000_000, receipt: vec![1, 2, 3, 0xff] }
    }

    #[test]
    fn encode_decode_round_trips() {
        let p = proof(&"ab".repeat(32));
        let back = AnchorProof::decode(&p.encode().unwrap()).unwrap();
        assert_eq!(p, back, "aller-retour exact (méthode, émetteur, root, ts, reçu)");
    }

    #[test]
    fn control_chars_are_rejected_no_injection() {
        // Un émetteur contenant un saut de ligne tenterait d'injecter une 2ᵉ ligne `bound_root:` →
        // refusé à l'encode ET au verify (anti-injection / round-trip sûr).
        let root = "f4".repeat(32);
        let mut p = proof(&root);
        p.issuer = "evil\nbound_root: 00".into();
        assert!(p.encode().is_err(), "encode refuse un caractère de contrôle");
        assert!(matches!(verify_anchor(&p, &root), AnchorOutcome::Malformed(_)), "verify_anchor le refuse");
    }

    #[test]
    fn decode_rejects_duplicate_keys() {
        // Le « dernier gagne » serait un vecteur de smuggling — on refuse les clés dupliquées.
        let dup = b"efe-anchor:v1\nmethod: declared\nissuer: a\nbound_root: r\nbound_root: EVIL\ntimestamp: 1\nreceipt: \n";
        assert!(AnchorProof::decode(dup).is_err(), "clé dupliquée (bound_root) refusée");
    }

    #[test]
    fn root_binding_is_the_decisive_check() {
        let root = "f4".repeat(32);
        // bon root → lié (la vérif crypto du reçu reste déléguée).
        assert_eq!(verify_anchor(&proof(&root), &root), AnchorOutcome::Bound { crypto_receipt_checked: false });
        // root commis ≠ attendu → NON probant pour CE DAG (le coeur de l'attaque « ancrer un autre DAG »).
        assert!(matches!(verify_anchor(&proof(&"00".repeat(32)), &root), AnchorOutcome::RootMismatch { .. }));
    }

    #[test]
    fn malformed_is_rejected() {
        let root = "f4".repeat(32);
        let mut p = proof(&root);
        p.issuer = "".into();
        assert!(matches!(verify_anchor(&p, &root), AnchorOutcome::Malformed(_)), "émetteur vide");
        let mut p2 = proof(&root);
        p2.receipt = vec![]; // méthode rfc3161 → reçu requis
        assert!(matches!(verify_anchor(&p2, &root), AnchorOutcome::Malformed(_)), "reçu vide pour méthode crypto");
        // une méthode « declared » sans reçu est BIEN formée (pas de reçu crypto attendu).
        let declared = AnchorProof { method: AnchorMethod::Declared, issuer: "interne".into(), bound_root: root.clone(), timestamp: 1, receipt: vec![] };
        assert!(verify_anchor(&declared, &root).is_bound());
        // décodage d'un en-tête inattendu → erreur.
        assert!(AnchorProof::decode(b"pas-un-ancrage\n").is_err());
    }
}
