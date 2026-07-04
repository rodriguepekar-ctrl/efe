// SPDX-License-Identifier: UNLICENSED
//! # `efe-meta-verify` — le re-prouveur **autonome** du consensus de méta-essaim.
//!
//! Le pendant « verify-mini » de l'étage méta-essaim. À partir de **deux fichiers**
//! — `meta_steps.jsonl` (les pas de consensus scellés, contenu public) et
//! `meta_manifest.json` (la revendication : id, nombre de pas, `root`) — et d'**une
//! clé tenant** (`--key-hex`, **jamais** dans les fichiers : souveraineté), un tiers
//! **re-prouve hors-ligne** qu'une suite de pas de consensus est authentique et
//! non-altérée — **sans nous**.
//!
//! Le sceau réutilise [`efe_audit_core::chain_hmac`] (+ HKDF par tenant), la **même**
//! construction que `efe-nano` et `efe-swarm-provenance` (vecteurs golden gelés,
//! byte-identiques) ⇒ la chaîne méta est **mutuellement vérifiable** avec les deux
//! autres étages. Le hash de contenu est **SHA-256** — la même primitive que le moat P1
//! (`provenance-dag` / `verify-mini`), donc **entièrement ré-implémentable à la main**
//! (c'est ce que fait le vérifieur zéro-dépendance `verify-meta-mini`, P1.2). **Aucune
//! dépendance au moteur de méta-essaim** : la re-preuve n'exige que la crypto partagée,
//! un hash, et un parseur.
//!
//! Contrat de sortie (identique à verify-mini) : intègre · altéré/réordonné/mauvaise
//! clé (faute **localisée**) · usage/E-S.

use efe_audit_core::chain_hmac;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Un pas de consensus **scellé** : le contenu PUBLIC + son sceau. La clé n'y est jamais.
/// C'est une ligne du `meta_steps.jsonl` (auto-portant : `meta_id` répété par ligne).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedMetaStep {
    /// Id du méta-essaim.
    pub meta_id: String,
    /// Indice du pas (0-based).
    pub step: u64,
    /// Résidu joint à ce pas.
    pub residual: f64,
    /// Méta-consensus **après projection de sûreté** (le vecteur z̃), en `f64` — le
    /// widening **exact** du `f32` du moteur, choisi parce que serde_json round-trip le
    /// `f64` **bit-à-bit** (ryu), pas le `f32` : indispensable pour que la re-preuve
    /// recompute le MÊME hash de contenu que le sceau.
    pub consensus: Vec<f64>,
    /// Sceau HMAC chaîné de ce pas (hex). Rempli par [`seal_in_place`].
    #[serde(default)]
    pub seal: String,
}

/// Le **manifeste** — la revendication que le tiers recoupe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaManifest {
    /// Id du méta-essaim.
    pub meta_id: String,
    /// Nombre de pas scellés.
    pub n_steps: usize,
    /// `meta_provenance_root` attendu (hex du dernier sceau).
    pub root: String,
}

/// Verdict de re-preuve.
#[derive(Debug, Clone)]
pub struct Verdict {
    /// Intègre ?
    pub ok: bool,
    /// Root re-calculé (hex).
    pub root: String,
    /// Indice du **premier** pas en faute (localisation), si échec par pas.
    pub broken_at: Option<usize>,
    /// Motif lisible.
    pub reason: Option<String>,
}

/// Hash de contenu canonique d'un pas : `SHA-256(id ‖ 0x1f ‖ step ‖ résidu ‖ dim ‖ z̃)`.
/// Encodage **déterministe** (little-endian, ordre de champ gelé). SHA-256 = même
/// primitive que le moat P1 ⇒ ré-implémentable à la main (vérifieur zéro-dép P1.2).
#[must_use]
pub fn canonical_hash(meta_id: &str, step: u64, residual: f64, consensus: &[f64]) -> [u8; 32] {
    let mut b = Vec::with_capacity(meta_id.len() + 25 + consensus.len() * 8);
    b.extend_from_slice(meta_id.as_bytes());
    b.push(0x1f); // unit separator : id ‖ corps
    b.extend_from_slice(&step.to_le_bytes());
    b.extend_from_slice(&residual.to_le_bytes());
    b.extend_from_slice(&(consensus.len() as u64).to_le_bytes());
    for &v in consensus {
        b.extend_from_slice(&v.to_le_bytes());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(&b));
    out
}

/// **Scelle** une suite de pas EN PLACE (remplit `.seal`) et rend le `root` (hex du
/// dernier sceau). `seal_i = chain_hmac(seal_{i-1}, canonical_hash_i, key)`, `seal_0`
/// sur parent `None` — exactement la construction d'`efe-swarm-provenance` / `efe-nano`.
pub fn seal_in_place(steps: &mut [SealedMetaStep], key: &[u8]) -> String {
    let mut parent: Option<[u8; 32]> = None;
    let mut last = [0u8; 32];
    for s in steps.iter_mut() {
        let ph = canonical_hash(&s.meta_id, s.step, s.residual, &s.consensus);
        let seal = chain_hmac(parent.as_ref(), &ph, key);
        s.seal = to_hex(&seal);
        parent = Some(seal);
        last = seal;
    }
    to_hex(&last)
}

/// **Re-prouve** : recalcule la chaîne depuis le contenu + la clé, compare **chaque**
/// sceau recomputé au sceau embarqué (localisation), puis le root au manifeste. Ne fait
/// **jamais** confiance au sceau embarqué : il le **recompute**.
#[must_use]
pub fn verify(steps: &[SealedMetaStep], key: &[u8], expected_root: &str) -> Verdict {
    if steps.is_empty() {
        return Verdict { ok: false, root: String::new(), broken_at: None, reason: Some("aucun pas à attester".into()) };
    }
    let mut parent: Option<[u8; 32]> = None;
    for (i, s) in steps.iter().enumerate() {
        let ph = canonical_hash(&s.meta_id, s.step, s.residual, &s.consensus);
        let recomputed = chain_hmac(parent.as_ref(), &ph, key);
        let claimed = match from_hex32(&s.seal) {
            Some(c) => c,
            None => return Verdict { ok: false, root: String::new(), broken_at: Some(i), reason: Some(format!("pas {i} : sceau illisible")) },
        };
        if recomputed != claimed {
            return Verdict {
                ok: false,
                root: to_hex(&recomputed),
                broken_at: Some(i),
                reason: Some(format!("pas {i} : sceau invalide — contenu altéré, réordonné, ou mauvaise clé tenant")),
            };
        }
        parent = Some(recomputed);
    }
    let root = to_hex(&parent.expect("≥ 1 pas ⇒ parent présent"));
    if root != expected_root {
        return Verdict { ok: false, root, broken_at: None, reason: Some("root du manifeste incohérent avec la chaîne re-calculée".into()) };
    }
    Verdict { ok: true, root, broken_at: None, reason: None }
}

/// Hex minuscule d'un buffer.
#[must_use]
pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Décode 64 hex → 32 octets. `None` si longueur ≠ 32 octets ou caractère non-hex.
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

/// Décode une chaîne hex → octets. `None` si longueur impaire ou caractère non-hex.
#[must_use]
pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<SealedMetaStep> {
        (0..5u64)
            .map(|i| SealedMetaStep {
                meta_id: "demo".into(),
                step: i,
                residual: 1.0 / (i as f64 + 1.0),
                consensus: vec![i as f64, 0.5, -1.0],
                seal: String::new(),
            })
            .collect()
    }

    #[test]
    fn seal_then_verify_ok() {
        let key = [7u8; 32];
        let mut s = sample();
        let root = seal_in_place(&mut s, &key);
        let v = verify(&s, &key, &root);
        assert!(v.ok, "authentique re-prouvé");
        assert_eq!(v.root, root);
        assert!(v.broken_at.is_none());
    }

    #[test]
    fn tamper_content_localizes_the_break() {
        let key = [7u8; 32];
        let mut s = sample();
        let root = seal_in_place(&mut s, &key);
        s[3].consensus[0] += 1.0; // altère le CONTENU du pas 3 (sceau embarqué inchangé)
        let v = verify(&s, &key, &root);
        assert!(!v.ok);
        assert_eq!(v.broken_at, Some(3), "faute localisée au pas altéré");
    }

    #[test]
    fn reorder_is_detected() {
        let key = [7u8; 32];
        let mut s = sample();
        let _ = seal_in_place(&mut s, &key);
        let root = s.last().map(|x| x.seal.clone()).unwrap_or_default();
        s.swap(1, 2); // réordonner deux pas
        let v = verify(&s, &key, &root);
        assert!(!v.ok, "un réordre casse la chaîne");
    }

    #[test]
    fn wrong_key_breaks_at_zero() {
        let mut s = sample();
        let root = seal_in_place(&mut s, &[7u8; 32]);
        let v = verify(&s, &[9u8; 32], &root);
        assert!(!v.ok);
        assert_eq!(v.broken_at, Some(0), "mauvaise clé → rupture au pas 0");
    }

    #[test]
    fn empty_is_rejected() {
        assert!(!verify(&[], &[7u8; 32], "").ok);
    }

    #[test]
    fn hex_roundtrip() {
        let b = [0u8, 1, 15, 16, 255, 42];
        assert_eq!(from_hex(&to_hex(&b)).unwrap(), b);
        assert!(from_hex("zz").is_none());
        assert!(from_hex32("00").is_none());
    }

    // DÉCISIF : le geste réel du bundle — sceller, sérialiser en JSONL, re-parser, re-vérifier.
    // Valeurs = widening f32→f64 (comme le méta-consensus réel) pour attraper toute perte.
    #[test]
    fn jsonl_roundtrip_reverifies() {
        let key = [7u8; 32];
        let mut s: Vec<SealedMetaStep> = (0..4u64)
            .map(|i| SealedMetaStep {
                meta_id: "kovant-procurement-v1".into(),
                step: i,
                residual: 7.4960402424643995 / (i as f64 + 1.0),
                consensus: vec![f64::from(-0.190_753_86_f32), f64::from(0.328_241_91_f32), f64::from(0.554_084_1_f32)],
                seal: String::new(),
            })
            .collect();
        let root = seal_in_place(&mut s, &key);
        let reparsed: Vec<SealedMetaStep> = s
            .iter()
            .map(|x| serde_json::from_str::<SealedMetaStep>(&serde_json::to_string(x).unwrap()).unwrap())
            .collect();
        let v = verify(&reparsed, &key, &root);
        assert!(v.ok, "round-trip JSONL doit re-vérifier — broken_at={:?}, root_att={root}, root_calc={}", v.broken_at, v.root);
    }
}
