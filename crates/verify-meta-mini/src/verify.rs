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
    /// **v2** : `‖z − z̃‖` scellé (l'enveloppe de sûreté auditée). `None` dans les bundles v1.
    pub safety_delta: Option<f64>,
    /// **v3** : hash de contenu (hex) de la contribution de chaque participant. `None` en v1/v2.
    pub vote_hashes: Option<Vec<String>>,
    /// **v4** : hash (hex) de la LIAISON du run — qui a contribué, sous quelle config, sous quelle
    /// règle. La liaison elle-même est dans le manifeste ; ici on ne scelle que son condensé.
    /// `None` en v1/v2/v3.
    pub binding_hash: Option<String>,
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
            safety_delta: o.get_f64_opt("safety_delta")?,
            vote_hashes: o.get_str_arr_opt("vote_hashes")?,
            binding_hash: o.get_str_opt("binding_hash")?,
            seal: from_hex32(o.get_str("seal")?).ok_or_else(|| "sceau : 64 hex attendus".to_string())?,
        })
    }
}

/// Hash de contenu canonique — **byte-identique** à `efe_meta_verify::canonical_hash` :
/// `SHA-256( id ‖ 0x1f ‖ step_LE ‖ résidu_LE ‖ dim_LE ‖ z̃_LE )`.
#[must_use]
pub fn canonical(
    meta_id: &str,
    step: u64,
    residual: f64,
    consensus: &[f64],
    safety_delta: Option<f64>,
    vote_hashes: Option<&[String]>,
    binding_hash: Option<&str>,
) -> [u8; 32] {
    let mut b = Vec::with_capacity(meta_id.len() + 41 + consensus.len() * 8);
    b.extend_from_slice(meta_id.as_bytes());
    b.push(0x1f);
    b.extend_from_slice(&step.to_le_bytes());
    b.extend_from_slice(&residual.to_le_bytes());
    b.extend_from_slice(&(consensus.len() as u64).to_le_bytes());
    for &v in consensus {
        b.extend_from_slice(&v.to_le_bytes());
    }
    // v2 : le safety_delta scellé est ajouté APRÈS le vecteur, uniquement s'il est présent (None ⇒ v1).
    if let Some(d) = safety_delta {
        b.extend_from_slice(&d.to_le_bytes());
    }
    // v3 : les hashes de contribution des participants, en ordre — count + 32 o bruts par hash décodé.
    if let Some(hashes) = vote_hashes {
        if !hashes.is_empty() {
            b.extend_from_slice(&(hashes.len() as u64).to_le_bytes());
            for h in hashes {
                b.extend_from_slice(&from_hex32(h).unwrap_or([0u8; 32]));
            }
        }
    }
    // v4 : le hash de la LIAISON, ajouté après les vote_hashes. Absent ⇒ octets v3 inchangés.
    if let Some(h) = binding_hash {
        b.extend_from_slice(&from_hex32(h).unwrap_or([0u8; 32]));
    }
    sha256(&b)
}

/// **v4** — la LIAISON du run, telle que le manifeste la porte : qui a contribué, sous quelle
/// configuration déclarée, sous quelle règle d'agrégation.
///
/// Représentation **plate** (deux colonnes alignées) — c'est ce qui permet à ce vérifieur de la
/// lire avec son parseur fait main, sans apprendre les objets imbriqués. Le choix a été fait du
/// côté producteur précisément pour que ce crate-ci reste petit et relisible d'un bout à l'autre.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    /// Identifiants **en ordre** — l'ordre est le lien avec `vote_hashes`.
    pub participant_ids: Vec<String>,
    /// `config_hash` déclarés, alignés ; chaîne vide ⟺ participant mock.
    pub declared_config_hashes: Vec<String>,
    /// Identité de la règle d'agrégation.
    pub operator: String,
    /// Poids de l'opérateur.
    pub alpha: f64,
    /// Poids du couplage.
    pub gamma: f64,
}

impl Binding {
    /// Lit la liaison d'un objet manifeste. `None` si le manifeste n'en porte pas (v1-v3).
    ///
    /// # Errors
    /// Champs présents mais mal typés, ou colonnes désalignées.
    pub fn from_manifest(o: &crate::json::Obj) -> Result<Option<Binding>, String> {
        let Some(participant_ids) = o.get_str_arr_opt("binding_participant_ids")? else {
            return Ok(None);
        };
        let declared_config_hashes = o
            .get_str_arr_opt("binding_declared_config_hashes")?
            .ok_or_else(|| "liaison : binding_declared_config_hashes manquant".to_string())?;
        if participant_ids.len() != declared_config_hashes.len() {
            return Err(format!(
                "liaison mal formée : {} identifiants pour {} config_hash — colonnes désalignées",
                participant_ids.len(),
                declared_config_hashes.len()
            ));
        }
        let operator = o.get_str("binding_operator").map_err(|e| format!("liaison : {e}"))?.to_string();
        let alpha = o.get_f64("binding_alpha").map_err(|e| format!("liaison : {e}"))?;
        let gamma = o.get_f64("binding_gamma").map_err(|e| format!("liaison : {e}"))?;
        Ok(Some(Binding { participant_ids, declared_config_hashes, operator, alpha, gamma }))
    }

    /// L'encodage canonique **binaire**, préfixé en longueur — miroir exact du producteur.
    #[must_use]
    pub fn canonical(&self) -> Vec<u8> {
        let n = self.participant_ids.len().min(self.declared_config_hashes.len());
        let mut b = Vec::with_capacity(64 + n * 64);
        b.extend_from_slice(&(n as u64).to_le_bytes());
        for i in 0..n {
            let id = &self.participant_ids[i];
            b.extend_from_slice(&(id.len() as u64).to_le_bytes());
            b.extend_from_slice(id.as_bytes());
            let h = &self.declared_config_hashes[i];
            if h.is_empty() {
                b.push(0);
            } else {
                b.push(1);
                b.extend_from_slice(&from_hex32(h).unwrap_or([0u8; 32]));
            }
        }
        b.extend_from_slice(&(self.operator.len() as u64).to_le_bytes());
        b.extend_from_slice(self.operator.as_bytes());
        b.extend_from_slice(&self.alpha.to_le_bytes());
        b.extend_from_slice(&self.gamma.to_le_bytes());
        b
    }

    /// Le hash que chaque pas doit sceller.
    #[must_use]
    pub fn hash_hex(&self) -> String {
        to_hex(&sha256(&self.canonical()))
    }
}

/// **v4** — recoupe la liaison du manifeste avec celle SCELLÉE dans les pas.
///
/// Sans ce contrôle, le champ ne servirait à rien : un manifeste dont on aurait réécrit les
/// identités passerait la re-preuve de la chaîne (qui ne regarde que le condensé scellé) et
/// l'auditeur lirait des noms falsifiés sous un root parfaitement valide.
///
/// # Errors
/// Présence asymétrique (scellé sans manifeste ou l'inverse), ou hash discordant.
pub fn check_binding(steps: &[Step], binding: Option<&Binding>) -> Result<(), String> {
    let expected = binding.map(Binding::hash_hex);
    for (i, s) in steps.iter().enumerate() {
        match (&s.binding_hash, &expected) {
            (None, None) => {}
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) => {
                return Err(format!(
                    "pas {i} : liaison du manifeste ≠ liaison scellée (scellé {a}, manifeste {b}) \
                     — identité ou règle falsifiée"
                ))
            }
            (Some(_), None) => return Err(format!("pas {i} : liaison SCELLÉE mais absente du manifeste")),
            (None, Some(_)) => return Err(format!("pas {i} : liaison au manifeste mais NON scellée à ce pas")),
        }
    }
    Ok(())
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
        let ph = canonical(&s.meta_id, s.step, s.residual, &s.consensus, s.safety_delta, s.vote_hashes.as_deref(), s.binding_hash.as_deref());
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
