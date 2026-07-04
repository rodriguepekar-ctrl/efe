// SPDX-License-Identifier: UNLICENSED
//! **La re-preuve** — encodage canonique, chaîne, et vérification, tout ré-implémenté à la main.
//! Deux invariants byte-à-byte, prouvés dans `tests/` : `canonical` = `efe_meta_verify::canonical_hash`
//! (SHA-256) et `seal_step` = `efe_audit_core::chain_hmac` (HMAC-SHA256, genèse `b"genesis"`).

use crate::hmac::{ct_eq, hmac_sha256};
use crate::json::parse_object;
use crate::sha256::sha256;

/// Un pas de consensus scellé, tel que re-lu du bundle.
pub struct Step {
    /// Id du méta-essaim.
    pub meta_id: String,
    /// Indice du pas.
    pub step: u64,
    /// Résidu joint.
    pub residual: f64,
    /// Méta-consensus après projection (z̃), en f64.
    pub consensus: Vec<f64>,
    /// Sceau embarqué (décodé).
    pub seal: [u8; 32],
}

impl Step {
    /// Parse une ligne JSONL de `meta_steps.jsonl` en un pas (parseur fait main, pas de serde).
    pub fn from_json_line(line: &str) -> Result<Step, String> {
        let o = parse_object(line)?;
        Ok(Step {
            meta_id: o.get_str("meta_id")?.to_string(),
            step: o.get_u64("step")?,
            residual: o.get_f64("residual")?,
            consensus: o.get_f64_arr("consensus")?,
            seal: from_hex32(o.get_str("seal")?).ok_or_else(|| "sceau : 64 hex attendus".to_string())?,
        })
    }
}

/// Hash de contenu canonique — **byte-identique** à `efe_meta_verify::canonical_hash` :
/// `SHA-256( id ‖ 0x1f ‖ step_LE ‖ résidu_LE ‖ dim_LE ‖ z̃_LE )`.
#[must_use]
pub fn canonical(meta_id: &str, step: u64, residual: f64, consensus: &[f64]) -> [u8; 32] {
    let mut b = Vec::with_capacity(meta_id.len() + 25 + consensus.len() * 8);
    b.extend_from_slice(meta_id.as_bytes());
    b.push(0x1f);
    b.extend_from_slice(&step.to_le_bytes());
    b.extend_from_slice(&residual.to_le_bytes());
    b.extend_from_slice(&(consensus.len() as u64).to_le_bytes());
    for &v in consensus {
        b.extend_from_slice(&v.to_le_bytes());
    }
    sha256(&b)
}

/// Un maillon de la chaîne — **byte-identique** à `efe_audit_core::chain_hmac` :
/// `HMAC-SHA256( clé , parent ‖ payload_hash )`, `parent = b"genesis"` à la genèse.
#[must_use]
pub fn seal_step(parent: Option<&[u8; 32]>, payload_hash: &[u8; 32], key: &[u8]) -> [u8; 32] {
    let mut msg = Vec::with_capacity(32 + 32);
    match parent {
        Some(p) => msg.extend_from_slice(p),
        None => msg.extend_from_slice(b"genesis"),
    }
    msg.extend_from_slice(payload_hash);
    hmac_sha256(key, &msg)
}

/// Le verdict de re-preuve.
pub enum Outcome {
    /// Intègre — root re-calculé.
    Ok {
        /// Le `meta_provenance_root` re-calculé.
        root: [u8; 32],
    },
    /// Refusé — premier pas en faute (localisation) + motif.
    Broken {
        /// Indice du pas en faute (ou `n` si c'est le root du manifeste).
        at: usize,
        /// Motif lisible.
        reason: String,
    },
}

/// RE-PROUVE : recompute chaque maillon depuis le contenu + la clé, compare (temps constant) au
/// sceau embarqué, puis le root au manifeste. Ne fait **jamais** confiance au sceau : il le recompute.
#[must_use]
pub fn verify(steps: &[Step], key: &[u8], expected_root: &[u8; 32]) -> Outcome {
    if steps.is_empty() {
        return Outcome::Broken { at: 0, reason: "aucun pas à attester".into() };
    }
    let mut parent: Option<[u8; 32]> = None;
    for (i, s) in steps.iter().enumerate() {
        let ph = canonical(&s.meta_id, s.step, s.residual, &s.consensus);
        let recomputed = seal_step(parent.as_ref(), &ph, key);
        if !ct_eq(&recomputed, &s.seal) {
            return Outcome::Broken { at: i, reason: format!("pas {i} : sceau invalide — contenu altéré, réordonné, ou mauvaise clé tenant") };
        }
        parent = Some(recomputed);
    }
    let root = parent.expect("≥ 1 pas ⇒ parent présent");
    if !ct_eq(&root, expected_root) {
        return Outcome::Broken { at: steps.len(), reason: "root du manifeste incohérent avec la chaîne re-calculée".into() };
    }
    Outcome::Ok { root }
}

/// Hex minuscule.
#[must_use]
pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Décode une chaîne hex → octets. `None` si longueur impaire ou caractère non-hex.
#[must_use]
pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Décode 64 hex → 32 octets.
#[must_use]
pub fn from_hex32(s: &str) -> Option<[u8; 32]> {
    let v = from_hex(s)?;
    if v.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    Some(out)
}
