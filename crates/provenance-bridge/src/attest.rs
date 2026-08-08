// SPDX-License-Identifier: UNLICENSED
//! **L'attestation de racine** — P.3 : la racine du bundle devient vérifiable **sans détenir
//! aucun secret**.
//!
//! # Le saut que ce module fait faire, et celui qu'il ne fait pas
//!
//! Jusqu'ici, tout le fil était **symétrique** : la pile A et la pile B scellent avec un HMAC par
//! tenant, et re-prouver exige de détenir la clé. P.2 a changé le *vérifieur* (un binaire à
//! dépendances vides, dont l'auditeur lit toute la source) ; il n'a pas changé le *besoin de clé*.
//!
//! P.3 signe la racine en **ed25519**. Un tiers qui ne détient que la **clé publique** peut alors
//! conclure une chose précise, et une seule : *cette racine-là a bien été émise par le détenteur de
//! la clé privée, et n'a pas été fabriquée après coup*. Il ne peut toujours **pas** re-prouver le
//! **contenu** du bundle sans la clé tenant — ce niveau-là reste détenteur-de-clé.
//!
//! # ⚠️ La signature ne prouve rien si la clé publique vient du même canal
//!
//! N'importe qui peut engendrer une paire de clés et signer n'importe quelle racine. La signature
//! n'a de valeur que si l'auditeur a **épinglé la clé publique hors bande** — par contrat, par
//! documentation publiée, par un canal distinct de celui qui lui remet le bundle. C'est pourquoi ce
//! module émet un **`key_id`** (une empreinte de la clé publique) et **jamais la clé publique
//! elle-même** : forcer l'auditeur à aller la chercher ailleurs est précisément le point. Servir la
//! clé à côté du bundle rendrait la vérification circulaire — et donc vide.
//!
//! C'est la même discipline que `cosign.rs` avec ses nœuds **pinnés** (`PinnedNode`) : le vérifieur
//! ne tient que la publique épinglée, jamais celle que l'attestant lui tend.
//!
//! # Ce que P.3 ne donne toujours pas
//!
//! **La détection de troncature sans clé.** Signer la racine authentifie *une* racine ; encore
//! faut-il que l'auditeur sache **laquelle** attendre. Tant que P.4 ne publie pas l'ancre hors du
//! bundle, un producteur malveillant peut tronquer le run *puis* signer honnêtement la racine du
//! bundle tronqué. La signature serait valide, et le mensonge intact. P.3 authentifie l'émetteur ;
//! P.4 fixe l'attendu.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use efe_swarm_provenance::blake3_bytes;
use uuid::Uuid;

use crate::seal::Bundle;

/// Le domaine de signature, **versionné et distinct** de tous les autres du dépôt.
///
/// La discipline vient de `cosign.rs` : *« le domaine versionné empêche qu'une co-signature d'un
/// usage soit re-interprétée comme l'autre »*. Une signature d'attestation de racine ne doit
/// jamais pouvoir être re-présentée comme une co-signature de nœud, ni l'inverse. Le `\x00`
/// terminal sépare le domaine du premier champ.
const ATTEST_DOMAIN_V1: &[u8] = b"EFE-FILP-ROOT-ATTEST-v1\x00";

/// La version du **format** d'attestation. Committée dans le tuple signé : changer le format sans
/// changer ce numéro produirait deux tuples différents pour un même schéma déclaré.
pub const ATTEST_SCHEMA_V1: u16 = 1;

/// Une attestation de racine — le seul artefact du fil qu'un tiers vérifie **sans secret**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootAttestation {
    /// Version du format (voir [`ATTEST_SCHEMA_V1`]).
    pub schema: u16,
    /// Le run attesté. Empêche de déplacer une attestation vers un autre run.
    pub run_id: Uuid,
    /// Le tenant. Empêche le transport inter-tenant.
    pub tenant_id: String,
    /// L'engagement sur le bundle — **ce qui est réellement attesté**.
    pub manifest_root: String,
    /// Le compte de nœuds : second garde contre la troncature, signé lui aussi.
    pub node_count: u32,
    /// **L'empreinte** de la clé publique — jamais la clé. L'auditeur s'en sert pour savoir
    /// *laquelle* de ses clés épinglées appliquer.
    pub key_id: String,
    /// Millisecondes depuis l'époque. Signé : empêche de présenter une vieille attestation comme
    /// fraîche.
    pub signed_at: u64,
    /// La signature ed25519, en hexadécimal (128 caractères = 64 octets).
    pub sig: String,
}

/// Pourquoi une attestation n'a pas été acceptée — ou n'a pas pu être produite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttestError {
    /// Format d'attestation inconnu de ce vérifieur.
    SchemaUnsupported(u16),
    /// La clé fournie n'est pas celle que l'attestation revendique.
    ///
    /// Distinguée de [`Self::SignatureInvalid`] à dessein : un auditeur qui applique la mauvaise
    /// clé doit l'apprendre, et non conclure à une falsification.
    KeyIdMismatch {
        /// Ce que l'attestation revendique.
        claimed: String,
        /// L'empreinte de la clé effectivement fournie.
        provided: String,
    },
    /// La signature n'est pas 64 octets hexadécimaux.
    MalformedSignature,
    /// La clé n'est pas 32 octets hexadécimaux, ou n'est pas un point ed25519 valide.
    MalformedKey,
    /// La signature ne vérifie pas — falsification, ou champ modifié après signature.
    SignatureInvalid,
    /// L'attestation est authentique, mais elle ne porte pas sur **ce** bundle.
    ///
    /// Le cas qui compte : une attestation valide, mais d'un autre run, d'un autre tenant, ou
    /// d'une autre racine. Sans ce contrôle, un producteur pourrait tendre un bundle accompagné
    /// d'une attestation authentique… d'autre chose.
    BundleMismatch {
        /// Le champ en désaccord.
        field: &'static str,
    },
}

impl std::fmt::Display for AttestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttestError::SchemaUnsupported(v) => write!(f, "schéma d'attestation non supporté : {v}"),
            AttestError::KeyIdMismatch { claimed, provided } => write!(
                f,
                "mauvaise clé : l'attestation revendique « {claimed} », la clé fournie est « {provided} »"
            ),
            AttestError::MalformedSignature => write!(f, "signature malformée (attendu 128 hex)"),
            AttestError::MalformedKey => write!(f, "clé publique malformée (attendu 64 hex, point ed25519 valide)"),
            AttestError::SignatureInvalid => write!(f, "signature ed25519 INVALIDE — forge, ou champ altéré après signature"),
            AttestError::BundleMismatch { field } => {
                write!(f, "attestation authentique mais portant sur autre chose : « {field} » diffère")
            }
        }
    }
}

impl std::error::Error for AttestError {}

// ─────────────────────────────────────────────────────────────────────────────
//  Hexadécimal — encodage/décodage, écrits ici parce qu'aucun n'est exporté
// ─────────────────────────────────────────────────────────────────────────────

/// Encode en hexadécimal minuscule.
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Décode de l'hexadécimal **strict** : longueur paire, minuscules ou majuscules, rien d'autre.
///
/// Volontairement intolérant. Ce décodeur lit de l'entrée **non fiable** (une attestation qu'on
/// nous tend) ; accepter des séparateurs, un préfixe `0x` ou des espaces multiplierait les
/// représentations d'une même valeur, et une signature n'a qu'une seule écriture légitime.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < b.len() {
        let hi = (b[i] as char).to_digit(16)?;
        let lo = (b[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    Some(out)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Le tuple signé
// ─────────────────────────────────────────────────────────────────────────────

/// Le tuple attesté :
/// `blake3( DOMAINE ‖ schema ‖ run_id(16) ‖ |tenant| ‖ tenant ‖ |root| ‖ root ‖ node_count ‖ signed_at ‖ |key_id| ‖ key_id )`.
///
/// **Séparation de domaine** (préfixe constant versionné) + **préfixes de longueur** sur tout champ
/// de taille variable ⇒ aucune ambiguïté de concaténation : deux jeux de champs distincts ne
/// peuvent pas produire le même tuple. Le `run_id` entre par ses **16 octets bruts** plutôt que par
/// sa forme textuelle, qui admet plusieurs écritures.
///
/// Déterministe : mêmes champs ⇒ même tuple, donc l'attestant et le vérifieur calculent la même
/// chose sans se concerter.
#[must_use]
pub fn attest_tuple(
    schema: u16,
    run_id: Uuid,
    tenant: &str,
    manifest_root: &str,
    node_count: u32,
    signed_at: u64,
    key_id: &str,
) -> [u8; 32] {
    let mut buf = Vec::with_capacity(
        ATTEST_DOMAIN_V1.len() + 2 + 16 + 8 + tenant.len() + 8 + manifest_root.len() + 4 + 8 + 8 + key_id.len(),
    );
    buf.extend_from_slice(ATTEST_DOMAIN_V1);
    buf.extend_from_slice(&schema.to_le_bytes());
    buf.extend_from_slice(run_id.as_bytes());
    buf.extend_from_slice(&(tenant.len() as u64).to_le_bytes());
    buf.extend_from_slice(tenant.as_bytes());
    buf.extend_from_slice(&(manifest_root.len() as u64).to_le_bytes());
    buf.extend_from_slice(manifest_root.as_bytes());
    buf.extend_from_slice(&node_count.to_le_bytes());
    buf.extend_from_slice(&signed_at.to_le_bytes());
    buf.extend_from_slice(&(key_id.len() as u64).to_le_bytes());
    buf.extend_from_slice(key_id.as_bytes());
    blake3_bytes(&buf).0
}

/// L'**empreinte** d'une clé publique : `blake3(pubkey)[..8]` en hexadécimal (16 caractères).
///
/// Ce n'est **pas un secret** — c'est un nom. Il permet à un auditeur qui détient plusieurs clés
/// épinglées de savoir laquelle appliquer, sans que l'attestation ait à lui tendre une clé (ce qui
/// rendrait la vérification circulaire).
#[must_use]
pub fn key_id_of(vk: &VerifyingKey) -> String {
    hex_encode(&blake3_bytes(vk.as_bytes()).0[..8])
}

// ─────────────────────────────────────────────────────────────────────────────
//  Le signataire
// ─────────────────────────────────────────────────────────────────────────────

/// Le détenteur de la clé privée d'attestation.
///
/// `Debug` est **rédigé** : la clé ne peut pas fuir dans un log par inadvertance. Même discipline
/// que `TenantKey` côté pile B.
#[derive(Clone)]
pub struct BundleSigner {
    sk: SigningKey,
    key_id: String,
}

impl std::fmt::Debug for BundleSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // L'empreinte publique suffit au diagnostic ; la clé, jamais.
        write!(f, "BundleSigner(key_id={}, sk=[rédigée])", self.key_id)
    }
}

impl BundleSigner {
    /// Construit depuis une graine de 32 octets.
    #[must_use]
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let sk = SigningKey::from_bytes(&seed);
        let key_id = key_id_of(&sk.verifying_key());
        Self { sk, key_id }
    }

    /// Construit depuis une graine en hexadécimal (64 caractères).
    ///
    /// Rend [`AttestError::MalformedKey`] sur toute entrée qui n'est pas exactement cela — on ne
    /// complète pas, on ne tronque pas : une graine approximative donnerait une clé silencieusement
    /// différente de celle que l'opérateur croit avoir déployée.
    pub fn from_seed_hex(hex: &str) -> Result<Self, AttestError> {
        let bytes = hex_decode(hex).ok_or(AttestError::MalformedKey)?;
        let seed: [u8; 32] = bytes.as_slice().try_into().map_err(|_| AttestError::MalformedKey)?;
        Ok(Self::from_seed(seed))
    }

    /// L'empreinte publique de cette clé — à publier, et à épingler côté auditeur.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// La clé **publique** en hexadécimal — destinée à une publication **hors bande**
    /// (documentation, contrat, canal distinct). Ne jamais la servir à côté du bundle : la
    /// vérification deviendrait circulaire.
    #[must_use]
    pub fn public_key_hex(&self) -> String {
        hex_encode(self.sk.verifying_key().as_bytes())
    }

    /// Atteste la racine d'un bundle.
    ///
    /// `signed_at` est **fourni par l'appelant** et non lu d'une horloge ici : le pont reste pur,
    /// et un test peut produire une attestation reproductible à l'octet.
    #[must_use]
    pub fn attest(
        &self,
        run_id: Uuid,
        tenant: &str,
        bundle: &Bundle,
        signed_at: u64,
    ) -> RootAttestation {
        let tuple = attest_tuple(
            ATTEST_SCHEMA_V1,
            run_id,
            tenant,
            &bundle.manifest_root,
            bundle.node_count,
            signed_at,
            &self.key_id,
        );
        let sig: Signature = self.sk.sign(&tuple);
        RootAttestation {
            schema: ATTEST_SCHEMA_V1,
            run_id,
            tenant_id: tenant.to_string(),
            manifest_root: bundle.manifest_root.clone(),
            node_count: bundle.node_count,
            key_id: self.key_id.clone(),
            signed_at,
            sig: hex_encode(&sig.to_bytes()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  La vérification — côté auditeur, sans aucun secret
// ─────────────────────────────────────────────────────────────────────────────

/// Vérifie qu'une attestation est **authentique** : signée par la clé donnée, champs intacts.
///
/// ⚠️ Cela ne dit **pas** qu'elle porte sur le bundle que vous tenez — voir
/// [`verify_attestation_binds`], qui est ce qu'un auditeur veut réellement. Séparer les deux est
/// délibéré : confondre « la signature est bonne » et « elle parle de ceci » est exactement
/// l'erreur qui rend une attestation inutile.
pub fn verify_attestation(att: &RootAttestation, vk: &VerifyingKey) -> Result<(), AttestError> {
    if att.schema != ATTEST_SCHEMA_V1 {
        return Err(AttestError::SchemaUnsupported(att.schema));
    }
    // On dit à l'auditeur qu'il applique la mauvaise clé, plutôt que de lui laisser croire à une
    // falsification.
    let provided = key_id_of(vk);
    if provided != att.key_id {
        return Err(AttestError::KeyIdMismatch { claimed: att.key_id.clone(), provided });
    }
    let sig_bytes = hex_decode(&att.sig).ok_or(AttestError::MalformedSignature)?;
    let sig_arr: [u8; 64] =
        sig_bytes.as_slice().try_into().map_err(|_| AttestError::MalformedSignature)?;
    let sig = Signature::from_bytes(&sig_arr);

    let tuple = attest_tuple(
        att.schema,
        att.run_id,
        &att.tenant_id,
        &att.manifest_root,
        att.node_count,
        att.signed_at,
        &att.key_id,
    );
    // `verify_strict` et non `verify` : il rejette les clés d'ordre faible et les malléabilités
    // que la vérification permissive laisse passer. Même choix que `cosign.rs`.
    vk.verify_strict(&tuple, &sig).map_err(|_| AttestError::SignatureInvalid)
}

/// Ce qu'un auditeur veut vraiment : l'attestation est authentique **et** elle porte sur **ce**
/// bundle, pour **ce** run et **ce** tenant.
///
/// Sans le second contrôle, un producteur pourrait tendre un bundle accompagné d'une attestation
/// parfaitement valide… portant sur un autre run.
pub fn verify_attestation_binds(
    att: &RootAttestation,
    vk: &VerifyingKey,
    run_id: Uuid,
    tenant: &str,
    bundle: &Bundle,
) -> Result<(), AttestError> {
    verify_attestation(att, vk)?;
    if att.run_id != run_id {
        return Err(AttestError::BundleMismatch { field: "run_id" });
    }
    if att.tenant_id != tenant {
        return Err(AttestError::BundleMismatch { field: "tenant_id" });
    }
    if att.manifest_root != bundle.manifest_root {
        return Err(AttestError::BundleMismatch { field: "manifest_root" });
    }
    if att.node_count != bundle.node_count {
        return Err(AttestError::BundleMismatch { field: "node_count" });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seal::Bundle;

    fn bundle() -> Bundle {
        Bundle {
            jsonl: "{\"kind\":\"agent_act\"}\n".into(),
            manifest_root: "aa".repeat(32),
            node_count: 7,
        }
    }
    fn signer() -> BundleSigner {
        BundleSigner::from_seed([9u8; 32])
    }
    fn run() -> Uuid {
        Uuid::from_u128(0xD0D0)
    }
    const TENANT: &str = "acme-corp";

    // ── Hexadécimal ───────────────────────────────────────────────────────────────────────

    #[test]
    fn hex_roundtrips_and_rejects_malformed_input() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0x1a]), "00ff1a");
        assert_eq!(hex_decode("00ff1a").unwrap(), vec![0x00, 0xff, 0x1a]);
        assert_eq!(hex_decode("00FF1A").unwrap(), vec![0x00, 0xff, 0x1a], "majuscules acceptées");
        assert!(hex_decode("abc").is_none(), "longueur impaire refusée");
        assert!(hex_decode("zz").is_none(), "non-hexadécimal refusé");
        assert!(hex_decode("0xff").is_none(), "préfixe refusé — une signature n'a qu'une écriture");
        assert!(hex_decode("ff ff").is_none(), "espaces refusés");
    }

    // ── Le contrat nominal ────────────────────────────────────────────────────────────────

    #[test]
    fn an_attestation_verifies_with_the_public_key_alone() {
        // LE point de P.3 : aucun secret côté vérifieur.
        let s = signer();
        let b = bundle();
        let att = s.attest(run(), TENANT, &b, 1_700_000_000_000);
        let vk = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        assert!(verify_attestation(&att, &vk).is_ok(), "vérifiée avec la seule clé publique");
        assert!(verify_attestation_binds(&att, &vk, run(), TENANT, &b).is_ok(), "…et elle lie CE bundle");
    }

    #[test]
    fn attesting_is_deterministic() {
        // Mêmes champs ⇒ même signature : l'attestant et le vérifieur calculent le même tuple.
        let s = signer();
        let b = bundle();
        assert_eq!(s.attest(run(), TENANT, &b, 42), s.attest(run(), TENANT, &b, 42));
    }

    #[test]
    fn the_key_id_names_the_key_without_revealing_it() {
        let s = signer();
        assert_eq!(s.key_id().len(), 16, "empreinte courte, non secrète");
        assert_ne!(s.key_id(), s.public_key_hex(), "l'empreinte n'est PAS la clé");
        assert_eq!(s.public_key_hex().len(), 64);
        // Deux clés distinctes ⟹ deux empreintes distinctes.
        assert_ne!(s.key_id(), BundleSigner::from_seed([1u8; 32]).key_id());
    }

    // ── Les refus : chaque champ signé est réellement lié ──────────────────────────────────

    #[test]
    fn tampering_with_any_signed_field_invalidates_the_signature() {
        let s = signer();
        let b = bundle();
        let vk = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        let good = s.attest(run(), TENANT, &b, 1_000);

        // La racine — le champ que tout le fil protège.
        let mut a = good.clone();
        a.manifest_root = "bb".repeat(32);
        assert_eq!(verify_attestation(&a, &vk), Err(AttestError::SignatureInvalid), "racine modifiée");

        // Le compte de nœuds — le second garde contre la troncature.
        let mut a = good.clone();
        a.node_count = 6;
        assert_eq!(verify_attestation(&a, &vk), Err(AttestError::SignatureInvalid), "compte gonflé/réduit");

        // Le run — empêche de déplacer une attestation.
        let mut a = good.clone();
        a.run_id = Uuid::from_u128(0xBEEF);
        assert_eq!(verify_attestation(&a, &vk), Err(AttestError::SignatureInvalid), "run substitué");

        // Le tenant — empêche le transport inter-tenant.
        let mut a = good.clone();
        a.tenant_id = "globex".into();
        assert_eq!(verify_attestation(&a, &vk), Err(AttestError::SignatureInvalid), "tenant substitué");

        // L'horodatage — empêche de rejouer une vieille attestation comme fraîche.
        let mut a = good.clone();
        a.signed_at = 2_000;
        assert_eq!(verify_attestation(&a, &vk), Err(AttestError::SignatureInvalid), "date rejouée");
    }

    #[test]
    fn a_wrong_key_is_reported_as_a_wrong_key_not_as_a_forgery() {
        // Distinction qui compte pour l'auditeur : « tu appliques la mauvaise clé » ≠ « on te ment ».
        let att = signer().attest(run(), TENANT, &bundle(), 1);
        let other = SigningKey::from_bytes(&[1u8; 32]).verifying_key();
        match verify_attestation(&att, &other) {
            Err(AttestError::KeyIdMismatch { claimed, provided }) => {
                assert_eq!(claimed, signer().key_id());
                assert_ne!(provided, claimed);
            }
            other => panic!("attendu KeyIdMismatch, obtenu {other:?}"),
        }
    }

    #[test]
    fn an_authentic_attestation_for_another_bundle_is_refused() {
        // Le piège : la signature est PARFAITEMENT valide, mais elle parle d'autre chose.
        let s = signer();
        let vk = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        let att = s.attest(run(), TENANT, &bundle(), 1);
        assert!(verify_attestation(&att, &vk).is_ok(), "authentique…");

        let other = Bundle { jsonl: String::new(), manifest_root: "cc".repeat(32), node_count: 7 };
        assert_eq!(
            verify_attestation_binds(&att, &vk, run(), TENANT, &other),
            Err(AttestError::BundleMismatch { field: "manifest_root" }),
            "…mais pas pour CE bundle"
        );
        assert_eq!(
            verify_attestation_binds(&att, &vk, Uuid::from_u128(1), TENANT, &bundle()),
            Err(AttestError::BundleMismatch { field: "run_id" })
        );
        assert_eq!(
            verify_attestation_binds(&att, &vk, run(), "globex", &bundle()),
            Err(AttestError::BundleMismatch { field: "tenant_id" })
        );
    }

    #[test]
    fn malformed_signatures_and_schemas_are_refused() {
        let s = signer();
        let vk = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        let good = s.attest(run(), TENANT, &bundle(), 1);

        let mut a = good.clone();
        a.sig = "zz".repeat(64);
        assert_eq!(verify_attestation(&a, &vk), Err(AttestError::MalformedSignature));

        let mut a = good.clone();
        a.sig = "ab".into(); // bonne forme hexadécimale, mauvaise longueur
        assert_eq!(verify_attestation(&a, &vk), Err(AttestError::MalformedSignature));

        let mut a = good.clone();
        a.schema = 99;
        assert_eq!(verify_attestation(&a, &vk), Err(AttestError::SchemaUnsupported(99)));
    }

    #[test]
    fn a_malformed_seed_is_refused_rather_than_padded() {
        // Une graine approximative donnerait une clé silencieusement différente de celle que
        // l'opérateur croit avoir déployée.
        assert_eq!(BundleSigner::from_seed_hex("abcd").unwrap_err(), AttestError::MalformedKey);
        assert_eq!(BundleSigner::from_seed_hex("zz".repeat(32).as_str()).unwrap_err(), AttestError::MalformedKey);
        assert!(BundleSigner::from_seed_hex(&"0a".repeat(32)).is_ok());
    }

    #[test]
    fn the_signer_never_leaks_its_key_in_debug() {
        let s = signer();
        let rendered = format!("{s:?}");
        assert!(rendered.contains("rédigée"), "le Debug est rédigé");
        assert!(!rendered.contains(&hex_encode(&[9u8; 32])), "la graine n'apparaît jamais");
    }

    // ── La limite honnête de P.3 ──────────────────────────────────────────────────────────

    #[test]
    fn p3_does_not_detect_truncation_only_p4_will() {
        // Un producteur malveillant tronque le run, puis signe HONNÊTEMENT la racine du bundle
        // tronqué. La signature est valide, et le mensonge intact. P.3 authentifie l'ÉMETTEUR ;
        // il ne fixe pas l'ATTENDU — c'est P.4 (l'ancre publiée hors bundle).
        let s = signer();
        let vk = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        let truncated = Bundle { jsonl: String::new(), manifest_root: "dd".repeat(32), node_count: 3 };
        let att = s.attest(run(), TENANT, &truncated, 1);
        assert!(
            verify_attestation_binds(&att, &vk, run(), TENANT, &truncated).is_ok(),
            "la signature d'un bundle tronqué est VALIDE — la limite de P.3, dite"
        );
    }
}
