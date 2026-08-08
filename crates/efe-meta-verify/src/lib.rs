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
use std::path::Path;

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
    /// **v2** : `‖z − z̃‖` — de combien l'**enveloppe de sûreté** a ramené le consensus dans
    /// l'ensemble admissible à ce pas (`0.0` = enveloppe inactive). **Absent** (`None`) dans les
    /// bundles v1 : l'encodage canonique v1 est alors **inchangé** (roots v1 stables). Sceller ce
    /// champ rend l'activation du garde-fou **re-prouvable** : « au pas k, l'enveloppe a agi de δ ».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_delta: Option<f64>,
    /// **v3** : hash de contenu (SHA-256, hex) de la **contribution** de chaque participant à ce pas
    /// (le vote lifté), **en ordre**. Borné à 32 o/participant quel que soit la dimension. Rend
    /// « l'essaim *i* a contribué l'entrée *h* au pas *k* » re-prouvable, et permet le **replay**
    /// (re-exécuter → re-hacher → concordance). Absent (`None`) en v1/v2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vote_hashes: Option<Vec<String>>,
    /// Sceau HMAC chaîné de ce pas (hex). Rempli par [`seal_in_place`].
    #[serde(default)]
    pub seal: String,
}

/// Le **manifeste** — la revendication que le tiers recoupe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaManifest {
    /// Id du méta-essaim.
    pub meta_id: String,
    /// Version du schéma d'encodage canonique : **1** (v1) ou **2** (v1 + `safety_delta`). Défaut 1
    /// (rétro-compatible : un manifeste v1 sans ce champ se lit en `schema_version = 1`).
    #[serde(default = "schema_v1")]
    pub schema_version: u32,
    /// Nombre de pas scellés.
    pub n_steps: usize,
    /// `meta_provenance_root` attendu (hex du dernier sceau).
    pub root: String,
}

/// Défaut serde du schéma (bundles v1 sans le champ).
fn schema_v1() -> u32 {
    1
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
pub fn canonical_hash(meta_id: &str, step: u64, residual: f64, consensus: &[f64], safety_delta: Option<f64>, vote_hashes: Option<&[String]>) -> [u8; 32] {
    let mut b = Vec::with_capacity(meta_id.len() + 41 + consensus.len() * 8);
    b.extend_from_slice(meta_id.as_bytes());
    b.push(0x1f); // unit separator : id ‖ corps
    b.extend_from_slice(&step.to_le_bytes());
    b.extend_from_slice(&residual.to_le_bytes());
    b.extend_from_slice(&(consensus.len() as u64).to_le_bytes());
    for &v in consensus {
        b.extend_from_slice(&v.to_le_bytes());
    }
    // v2 : le `safety_delta` est ajouté APRÈS le vecteur, uniquement s'il est présent. `None` ⇒ octets
    // strictement v1 (roots v1 stables). Le champ étant scellé, le retirer/altérer casse la re-preuve.
    if let Some(d) = safety_delta {
        b.extend_from_slice(&d.to_le_bytes());
    }
    // v3 : les hashes de contribution des participants, EN ORDRE, ajoutés après le safety_delta. Chaque
    // hash (64 hex) est décodé et scellé en 32 o bruts. Absent ⇒ octets v2 (⊂ v1) inchangés.
    if let Some(hashes) = vote_hashes {
        if !hashes.is_empty() {
            b.extend_from_slice(&(hashes.len() as u64).to_le_bytes());
            for h in hashes {
                b.extend_from_slice(&from_hex32(h).unwrap_or([0u8; 32]));
            }
        }
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(&b));
    out
}

/// Hash de contenu d'un vecteur (SHA-256 des octets `f64` little-endian → hex). Sert à sceller la
/// **contribution** d'un participant au consensus (v3), **bornée à 32 o** quelle que soit la dimension.
#[must_use]
pub fn content_hash_hex(values: &[f64]) -> String {
    let mut b = Vec::with_capacity(values.len() * 8);
    for &v in values {
        b.extend_from_slice(&v.to_le_bytes());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(&b));
    to_hex(&out)
}

/// **Scelle** une suite de pas EN PLACE (remplit `.seal`) et rend le `root` (hex du
/// dernier sceau). `seal_i = chain_hmac(seal_{i-1}, canonical_hash_i, key)`, `seal_0`
/// sur parent `None` — exactement la construction d'`efe-swarm-provenance` / `efe-nano`.
pub fn seal_in_place(steps: &mut [SealedMetaStep], key: &[u8]) -> String {
    let mut parent: Option<[u8; 32]> = None;
    let mut last = [0u8; 32];
    for s in steps.iter_mut() {
        let ph = canonical_hash(&s.meta_id, s.step, s.residual, &s.consensus, s.safety_delta, s.vote_hashes.as_deref());
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
        let ph = canonical_hash(&s.meta_id, s.step, s.residual, &s.consensus, s.safety_delta, s.vote_hashes.as_deref());
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

// ─────────────────────────────────────────────────────────────────────────────
// API INCRÉMENTALE — pour le câblage moteur (sceller un pas à la fois, sans jamais
// détenir la trace entière). Byte-identique à la version batch (`seal_in_place`).
// ─────────────────────────────────────────────────────────────────────────────

/// Chaîne de provenance **incrémentale** : scelle un pas de consensus à la fois et retient le seul
/// `parent` (HMAC courant). Conçue pour être pilotée par un observateur du moteur de méta-essaim —
/// la trace complète n'a **jamais** à être matérialisée. La **clé** vit ici le temps du run et est
/// **droppée** par [`MetaChainState::finish`] (souveraineté : elle n'entre jamais dans l'artefact).
pub struct MetaChainState {
    meta_id: String,
    key: Vec<u8>,
    parent: Option<[u8; 32]>,
    steps: Vec<SealedMetaStep>,
}

impl MetaChainState {
    /// Nouvelle chaîne vide pour `meta_id`, scellée sous `key` (32 o, dérivée par tenant).
    pub fn new(meta_id: impl Into<String>, key: Vec<u8>) -> Self {
        Self { meta_id: meta_id.into(), key, parent: None, steps: Vec::new() }
    }

    /// Scelle **un** pas et l'ajoute à la chaîne : `seal = chain_hmac(parent, SHA256(canonique), clé)`.
    /// `safety_delta = Some(‖z − z̃‖)` scelle l'enveloppe de sûreté (v2) ; `vote_hashes = Some(...)` scelle
    /// les contributions des participants (v3). `None`/`None` = v1.
    pub fn push(&mut self, step: u64, residual: f64, consensus: Vec<f64>, safety_delta: Option<f64>, vote_hashes: Option<Vec<String>>) {
        let ph = canonical_hash(&self.meta_id, step, residual, &consensus, safety_delta, vote_hashes.as_deref());
        let seal = chain_hmac(self.parent.as_ref(), &ph, &self.key);
        self.steps.push(SealedMetaStep { meta_id: self.meta_id.clone(), step, residual, consensus, safety_delta, vote_hashes, seal: to_hex(&seal) });
        self.parent = Some(seal);
    }

    /// Nombre de pas scellés.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Vrai si aucun pas n'a été scellé.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Consomme la chaîne → l'artefact de provenance. **La clé est droppée ici** (jamais dans la sortie).
    #[must_use]
    pub fn finish(self) -> MetaProvenance {
        let root = self.parent.map_or_else(String::new, |p| to_hex(&p));
        MetaProvenance { meta_id: self.meta_id, root, steps: self.steps }
    }
}

/// L'artefact de provenance d'un run de méta-essaim — **sans clé**. Re-prouvable hors-ligne par
/// `verify_meta` (P1.1) et `verify_meta_mini` (P1.2, zéro-dépendance).
#[derive(Debug, Clone)]
pub struct MetaProvenance {
    /// Id du méta-essaim.
    pub meta_id: String,
    /// `meta_provenance_root` (hex du dernier sceau).
    pub root: String,
    /// Les pas de consensus scellés.
    pub steps: Vec<SealedMetaStep>,
}

impl MetaProvenance {
    /// Le manifeste (la revendication que le tiers recoupe). `schema_version` = **3** dès qu'un pas
    /// porte des `vote_hashes` (contributions participants), **2** dès qu'un `safety_delta` (enveloppe
    /// de sûreté) est présent, sinon **1**.
    #[must_use]
    pub fn manifest(&self) -> MetaManifest {
        let schema_version = if self.steps.iter().any(|s| s.vote_hashes.is_some()) {
            3
        } else if self.steps.iter().any(|s| s.safety_delta.is_some()) {
            2
        } else {
            1
        };
        MetaManifest { meta_id: self.meta_id.clone(), schema_version, n_steps: self.steps.len(), root: self.root.clone() }
    }

    /// Le contenu de `meta_steps.jsonl` (une [`SealedMetaStep`] par ligne).
    #[must_use]
    pub fn steps_jsonl(&self) -> String {
        let mut s = String::new();
        for step in &self.steps {
            s.push_str(&serde_json::to_string(step).expect("SealedMetaStep sérialisable"));
            s.push('\n');
        }
        s
    }

    /// Le contenu de `meta_manifest.json` — **la définition unique** de sa forme octet.
    ///
    /// Extrait de [`Self::write_bundle`] (Couche 3 M2) parce que le bundle ne voyage plus seulement
    /// par le disque : le contrôleur méta-essaim le rend aussi sur le fil gRPC. Deux sérialisations
    /// séparées auraient dérivé — un espacement, un `\n` final — et la dérive n'aurait été
    /// découverte que chez l'auditeur, dont le vérifieur aurait refusé un bundle pourtant intègre.
    /// Un seul producteur d'octets, donc, pour les deux chemins.
    #[must_use]
    pub fn manifest_json(&self) -> String {
        serde_json::to_string_pretty(&self.manifest()).expect("manifeste sérialisable") + "\n"
    }

    /// Écrit le bundle (`meta_steps.jsonl` + `meta_manifest.json`) sous `dir`. **La clé n'y est jamais.**
    pub fn write_bundle(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("meta_steps.jsonl"), self.steps_jsonl())?;
        std::fs::write(dir.join("meta_manifest.json"), self.manifest_json())?;
        Ok(())
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;

    #[test]
    fn incremental_equals_batch_and_reverifies() {
        let key = [0x3du8; 32];
        let content: Vec<(u64, f64, Vec<f64>)> = (0..5u64).map(|i| (i, 1.0 / (i as f64 + 1.0), vec![i as f64, 0.5, -1.0])).collect();

        // Version BATCH (seal_in_place) = la référence.
        let mut batch: Vec<SealedMetaStep> = content
            .iter()
            .map(|(s, r, c)| SealedMetaStep { meta_id: "m".into(), step: *s, residual: *r, consensus: c.clone(), safety_delta: None, vote_hashes: None, seal: String::new() })
            .collect();
        let batch_root = seal_in_place(&mut batch, &key);

        // Version INCRÉMENTALE (MetaChainState) = le câblage moteur.
        let mut chain = MetaChainState::new("m", key.to_vec());
        for (s, r, c) in &content {
            chain.push(*s, *r, c.clone(), None, None);
        }
        let prov = chain.finish();

        assert_eq!(prov.root, batch_root, "incrémental == batch (root)");
        assert_eq!(prov.steps.len(), batch.len());
        for (a, b) in prov.steps.iter().zip(&batch) {
            assert_eq!(a.seal, b.seal, "sceau identique au pas {}", a.step);
        }
        assert!(verify(&prov.steps, &key, &prov.root).ok, "l'artefact incrémental se re-prouve");
        assert_eq!(prov.manifest().n_steps, 5);
    }

    #[test]
    fn v2_safety_delta_is_sealed_reprovable_and_schema_tagged() {
        let key = [0x5au8; 32];
        let mut chain = MetaChainState::new("m", key.to_vec());
        chain.push(0, 1.0, vec![0.1, 0.2], Some(0.0), None); // enveloppe inactive
        chain.push(1, 0.5, vec![0.3, 0.4], Some(0.47), None); // enveloppe ACTIVE (δ=0.47) — l'événement clé
        let prov = chain.finish();

        assert_eq!(prov.manifest().schema_version, 2, "un delta présent (sans vote_hashes) ⇒ schéma v2");
        assert!(verify(&prov.steps, &key, &prov.root).ok, "v2 se re-prouve");
        assert_eq!(prov.steps[1].safety_delta, Some(0.47));

        // Retirer sournoisement le delta scellé (déclasser v2→v1) CASSE la re-preuve.
        let mut stripped = prov.steps.clone();
        stripped[1].safety_delta = None;
        assert!(!verify(&stripped, &key, &prov.root).ok, "retirer un safety_delta scellé casse le sceau");

        // v1 (None) ≠ v2 (Some) sur le même contenu (le delta compte) ; et None reste byte-stable.
        assert_ne!(canonical_hash("m", 1, 0.5, &[0.3, 0.4], None, None), canonical_hash("m", 1, 0.5, &[0.3, 0.4], Some(0.47), None));
        assert_eq!(canonical_hash("m", 1, 0.5, &[0.3, 0.4], None, None), canonical_hash("m", 1, 0.5, &[0.3, 0.4], None, None));
    }

    #[test]
    fn v3_vote_hashes_are_sealed_reprovable_and_schema_tagged() {
        let key = [0x6bu8; 32];
        // Deux participants → deux hashes de contribution par pas.
        let vh0 = vec![content_hash_hex(&[1.0, 2.0]), content_hash_hex(&[3.0, 4.0])];
        let vh1 = vec![content_hash_hex(&[5.0, 6.0]), content_hash_hex(&[7.0, 8.0])];
        let mut chain = MetaChainState::new("m", key.to_vec());
        chain.push(0, 1.0, vec![0.1, 0.2], Some(0.0), Some(vh0));
        chain.push(1, 0.5, vec![0.3, 0.4], Some(0.47), Some(vh1.clone()));
        let prov = chain.finish();

        assert_eq!(prov.manifest().schema_version, 3, "des vote_hashes présents ⇒ schéma v3");
        assert!(verify(&prov.steps, &key, &prov.root).ok, "v3 se re-prouve");
        assert_eq!(prov.steps[1].vote_hashes.as_ref().unwrap()[0], vh1[0], "le hash de contribution voyage");

        // Altérer un hash de contribution CASSE la re-preuve (les votes sont liés).
        let mut tampered = prov.steps.clone();
        tampered[1].vote_hashes.as_mut().unwrap()[0] = content_hash_hex(&[9.9, 9.9]);
        assert!(!verify(&tampered, &key, &prov.root).ok, "altérer un vote_hash scellé casse le sceau");
    }
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
                safety_delta: None,
                vote_hashes: None,
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
                safety_delta: None,
                vote_hashes: None,
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
