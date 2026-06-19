// SPDX-License-Identifier: UNLICENSED
//! # `cosign` — R3 / RC-2 : la **co-signature T-WITNESS** (ed25519, non-répudiable).
//!
//! RC-1 a rendu le moat **honnête** sur sa faiblesse à distance : un appel hors de la boîte
//! produit un record [`ComputeAttestation::R4RemoteChannel`] — « j'atteste la *transaction* sur un
//! canal pinné, jamais la computation ». R3 est le **geste actif** : un nœud distant **pinné**
//! co-signe sa propre computation avec sa **propre clé ed25519**, dont l'orchestrateur (et tout
//! tiers) ne tient que la **partie publique**. Le record passe de « *un* distant a renvoyé X » à
//! « **le nœud N, dont tu as pinné la clé, jure sous sa signature non-répudiable avoir produit X** ».
//!
//! ## Ce que R3 donne — et ce qu'il ne donne pas
//! - **Donne** : non-répudiation (seul le détenteur de la clé privée a pu signer) + responsabilité
//!   **nommée** (une identité enrôlée, donc révocable, donc imputable) + liaison contenu→nœud
//!   infalsifiable (la signature couvre `prompt_hash ‖ output_hash`).
//! - **Ne donne pas** la **correction** : un nœud pinné *honnête-mais-faux* co-signe fidèlement une
//!   sortie fausse. R3 prouve QUI, pas QUE-c'est-juste. C'est de l'**imputabilité**, pas de la vérité.
//!
//! ## Pourquoi ed25519 (asymétrique) et pas HMAC
//! La chaîne N17 est scellée en HMAC **symétrique** sous la clé maître tenant (qui ne quitte jamais
//! l'orchestrateur). Une co-signature HMAC exigerait que le vérifieur **détienne le secret** du nœud
//! — donc qu'il puisse **forger** la signature : pas de non-répudiation. L'**asymétrie est l'essence**
//! de R3 : le nœud garde la clé **privée** (elle ne quitte jamais le nœud, miroir d'I-RC2), le
//! vérifieur ne tient que la **publique pinnée**. Seul le nœud a pu signer ; quiconque vérifie, sans
//! secret. Deux systèmes orthogonaux : le HMAC tenant scelle le record (co-signature **incluse**)
//! dans le `leaf_hash` ; l'ed25519 du nœud signe le tuple. **Forger un R3 exige les DEUX clés.**
//!
//! ## La règle de fer (no false elevation, I-RC9)
//! Le barreau **scellé** est la *revendication* du producteur ; le barreau **vérifié** est ce que la
//! preuve cryptographique *soutient*. Le vérifieur n'accepte **jamais** R3 sur parole : une
//! co-signature invérifiable (nœud non pinné, révoqué, tuple altéré, signature fausse) **redescend**
//! le record à R4 — la transaction re-prouve toujours — et le verdict le **signale**. Voir
//! [`verify_cosignature`] et `n17_chain::assess_cosignatures`.
//!
//! On réutilise la **même** primitive qu'N18 publisher-identity (`ed25519-dalek`) ; rien de maison.

use std::collections::HashMap;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use efe_nano_sdk::{Blake3Hash, ComputeAttestation, CoSignatureScheme, NodeCoSignature};

use crate::hash::blake3_bytes;

// ─────────────────────────────────────────────────────────────────────────────
//  hex (mini-impl, sans dépendance — aligné la mini-base64 d'N18)
// ─────────────────────────────────────────────────────────────────────────────

/// Encode des octets en hex minuscule.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

/// Décode une chaîne hex (longueur paire) en octets. `None` si malformée.
pub(crate) fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < b.len() {
        let hi = (b[i] as char).to_digit(16)?;
        let lo = (b[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Le tuple co-signé
// ─────────────────────────────────────────────────────────────────────────────

/// **R3 / R3+** — le tuple que le nœud signe : `blake3( DOMAINE ‖ |node_id| ‖ node_id ‖ prompt_hash ‖
/// output_hash ‖ [|model_fp| ‖ model_fp] ‖ |nonce| ‖ nonce )`. La composante **`model_fp`** (R3+)
/// n'est présente que si le nœud atteste son modèle (`model_fingerprint = Some`), et le DOMAINE
/// bascule alors de `EFE-RC2` à `EFE-RC3` — un `None` redonne exactement le tuple R3.
///
/// Les champs sont précisément ceux que le nœud **peut connaître à l'instant du calcul** : son
/// identité, le prompt reçu, la sortie produite, l'empreinte de ses poids (R3+), le nonce-défi reçu
/// de l'orchestrateur. Il ne signe
/// **pas** le `provenance_id` (assigné *après* par l'orchestrateur, inconnu du nœud) : la liaison au
/// run est assurée par le `leaf_hash` (qui contient `provenance_id`/`tenant_id`). **Séparation de
/// domaine** (préfixe constant) + **préfixes de longueur** sur les champs variables ⇒ aucune
/// ambiguïté de concaténation. Déterministe : mêmes entrées ⇒ même tuple (vérifieur == attestant).
pub fn cosign_tuple(
    node_id: &str,
    final_prompt_hash: &Blake3Hash,
    output_hash: &Blake3Hash,
    model_fingerprint: Option<&str>,
    nonce: &str,
) -> Blake3Hash {
    let mut buf = Vec::with_capacity(32 + node_id.len() + 64 + 8 + nonce.len());
    // **R3+** : quand le nœud atteste un modèle (`Some`), le tuple emploie un domaine DISTINCT
    // (`EFE-RC3-COSIGN-v1`) et **inclut** l'empreinte-modèle. Sans attestation (`None`), il est
    // **byte-identique** au tuple R3 (`EFE-RC2-COSIGN-v1`, sans composante modèle) — les
    // co-signatures production-seules restent valides. Le domaine versionné empêche qu'une
    // co-signature d'un usage soit re-interprétée comme l'autre.
    match model_fingerprint {
        Some(_) => buf.extend_from_slice(b"EFE-RC3-COSIGN-v1\x00"),
        None => buf.extend_from_slice(b"EFE-RC2-COSIGN-v1\x00"),
    }
    buf.extend_from_slice(&(node_id.len() as u64).to_le_bytes());
    buf.extend_from_slice(node_id.as_bytes());
    buf.extend_from_slice(&final_prompt_hash.0);
    buf.extend_from_slice(&output_hash.0);
    if let Some(mf) = model_fingerprint {
        buf.extend_from_slice(&(mf.len() as u64).to_le_bytes());
        buf.extend_from_slice(mf.as_bytes());
    }
    buf.extend_from_slice(&(nonce.len() as u64).to_le_bytes());
    buf.extend_from_slice(nonce.as_bytes());
    blake3_bytes(&buf)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Le registre de nœuds (la racine de confiance, sans secret)
// ─────────────────────────────────────────────────────────────────────────────

/// Statut d'une clé de nœud enrôlée. Une clé révoquée fait redescendre ses records R3 à R4 (I-RC13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeStatus {
    /// Clé active : les co-signatures vérifiées hissent le record à R3.
    Active,
    /// Clé révoquée (compromission, déclassement) : la co-signature ne hisse plus — redescente R4.
    Revoked {
        /// Motif (forensique). Aligné l'esprit de `RevocationReason` (N18).
        reason: String,
    },
}

/// Un nœud **pinné** : son identité, sa clé publique ed25519 (hex des 32 octets bruts) et son statut.
///
/// L'orchestrateur ne détient que du **public** — un tiers peut donc tenir le registre + les records
/// + la clé tenant et **tout vérifier hors ligne**, sans aucun secret. L'enrôlement (publication +
/// épinglage de la pubkey) est un **acte d'opérateur, hors-bande** — miroir de l'épinglage d'endpoint
/// de RC-1, mais sur une identité de *signature* et non un canal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedNode {
    /// Clé publique ed25519, hex des 32 octets bruts (`VerifyingKey::to_bytes`).
    pub pubkey_hex: String,
    /// Statut courant de la clé.
    pub status: NodeStatus,
    /// **R3+** : les empreintes-modèles **attendues** pour ce nœud (hex), pinnées hors-bande par
    /// l'opérateur. Le vérifieur ne tient `model_hash_verified` que si l'empreinte co-signée par le
    /// nœud ∈ cet ensemble (I-RC7/I-RC14). Vide ⇒ le nœud n'atteste pas de modèle (R3 production-seule).
    pub expected_models: Vec<String>,
}

/// Le registre des nœuds attestants pinnés : `node_id → PinnedNode`. Input de **vérification**
/// (que du public). Calqué sur l'esprit d'`InMemoryPublisherRegistry` (N18).
#[derive(Debug, Clone, Default)]
pub struct NodeRegistry {
    nodes: HashMap<String, PinnedNode>,
}

impl NodeRegistry {
    /// Un registre **vide** : aucun nœud pinné ⇒ toute revendication R3 est invérifiable et
    /// redescend à R4 (I-RC9). C'est le défaut honnête de `verify_run_offline` (sans registre).
    pub fn empty() -> Self {
        Self { nodes: HashMap::new() }
    }

    /// Enrôle un nœud **actif** avec sa clé publique (hex). Builder cumulable. Aucune empreinte-modèle
    /// attendue par défaut (R3 production-seule) ; pinner via [`expect_model`](NodeRegistry::expect_model).
    pub fn with_node(mut self, node_id: impl Into<String>, pubkey_hex: impl Into<String>) -> Self {
        self.nodes.insert(
            node_id.into(),
            PinnedNode { pubkey_hex: pubkey_hex.into(), status: NodeStatus::Active, expected_models: Vec::new() },
        );
        self
    }

    /// Enrôle (ou écrase) un nœud avec un statut explicite.
    pub fn insert(&mut self, node_id: impl Into<String>, node: PinnedNode) {
        self.nodes.insert(node_id.into(), node);
    }

    /// **Révoque** la clé d'un nœud (I-RC13). Sans effet si le nœud est inconnu.
    pub fn revoke(&mut self, node_id: &str, reason: impl Into<String>) {
        if let Some(n) = self.nodes.get_mut(node_id) {
            n.status = NodeStatus::Revoked { reason: reason.into() };
        }
    }

    /// **R3+** : pinne une **empreinte-modèle attendue** (hex) pour un nœud (acte d'opérateur,
    /// hors-bande). Cumulable — un nœud peut servir plusieurs modèles (pool, rotation). Le vérifieur
    /// ne tient `model_hash_verified` que si l'empreinte co-signée ∈ cet ensemble (I-RC14). Sans
    /// effet si le nœud n'est pas (encore) enrôlé — la pubkey vient d'abord.
    pub fn expect_model(&mut self, node_id: &str, fingerprint: impl Into<String>) {
        if let Some(n) = self.nodes.get_mut(node_id) {
            n.expected_models.push(fingerprint.into());
        }
    }

    /// Résout un `node_id` vers son nœud pinné (clé publique + statut).
    pub fn lookup(&self, node_id: &str) -> Option<&PinnedNode> {
        self.nodes.get(node_id)
    }

    /// Nombre de nœuds enrôlés.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    /// `true` si aucun nœud n'est enrôlé.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  La vérification (le moat) — aucune élévation sans preuve
// ─────────────────────────────────────────────────────────────────────────────

/// La raison pour laquelle une revendication R3 **n'est pas honorée** et redescend à R4.
/// Toujours **rapportée** (jamais silencieuse, I-RC9) via `n17_chain::CosignAnomaly`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CosignDowngrade {
    /// Le `node_id` n'est pas dans le registre (aucune clé pinnée pour lui).
    NodeNotPinned,
    /// La clé du nœud est révoquée (I-RC13).
    NodeRevoked,
    /// Schéma de co-signature non supporté (≠ ed25519 à v1).
    UnsupportedScheme,
    /// Le tuple recalculé par le vérifieur ≠ `signed_tuple_hash` : la co-signature ne lie pas
    /// le contenu de CE record (sortie échangée, prompt altéré, nonce divergent…).
    TupleMismatch,
    /// La clé publique pinnée est malformée (hex / longueur).
    MalformedKey,
    /// La signature est malformée (hex / longueur ≠ 64 octets).
    MalformedSignature,
    /// La signature ed25519 ne vérifie pas contre la clé pinnée sur le tuple (forge / mauvaise clé).
    SignatureInvalid,
    /// **RC-2** : le record revendique R3 mais n'a **pas de sortie** (`output_hash == None`) — on
    /// refuse de lier une co-signature à un contenu absent (pas de substitution ZERO). Voir I-RC9.
    MissingOutput,
    /// **RC-2 / I-RC12 (volet hors-ligne)** : le nonce-défi de cette co-signature est **réutilisé**
    /// par un autre record R3 du même run — signal de rejeu intra-run (le défi doit être unique par
    /// appel). La fraîcheur inter-run (défi jamais vu d'un run passé) reste, elle, une garantie de
    /// l'émission orchestrateur (suivi des nonces consommés), hors de portée du vérifieur hors ligne.
    NonceReused,
    /// **RC-2** : la chaîne N17 du run n'est **pas intègre** — le record n'est pas prouvé appartenir
    /// au run, donc sa revendication R3 n'est pas honorée (pas de témoin sur un run non re-prouvé).
    ChainNotIntact,
}

impl CosignDowngrade {
    /// Libellé stable pour la forensique / les rapports.
    pub fn label(self) -> &'static str {
        match self {
            CosignDowngrade::NodeNotPinned => "node-not-pinned",
            CosignDowngrade::NodeRevoked => "node-revoked",
            CosignDowngrade::UnsupportedScheme => "unsupported-scheme",
            CosignDowngrade::TupleMismatch => "tuple-mismatch",
            CosignDowngrade::MalformedKey => "malformed-key",
            CosignDowngrade::MalformedSignature => "malformed-signature",
            CosignDowngrade::SignatureInvalid => "signature-invalid",
            CosignDowngrade::MissingOutput => "missing-output",
            CosignDowngrade::NonceReused => "nonce-reused",
            CosignDowngrade::ChainNotIntact => "chain-not-intact",
        }
    }
}

/// **Vérifie** une co-signature de nœud contre le registre — la garantie de R3, pure et hors ligne.
///
/// Le vérifieur **recompose** le tuple à partir des *propres champs du record* (`final_prompt_hash`,
/// `output_hash`) + l'identité et le nonce de la co-signature — il ne fait **pas** confiance au
/// `signed_tuple_hash` fourni, il le **recalcule** et le compare — puis vérifie la signature ed25519
/// (`verify_strict`, anti-malléabilité) contre la clé publique **pinnée**. `Ok(())` ⇒ R3 tenu ;
/// `Err(raison)` ⇒ redescente à R4 (jamais d'acceptation sur parole, jamais de redescente silencieuse).
pub fn verify_cosignature(
    final_prompt_hash: &Blake3Hash,
    output_hash: &Blake3Hash,
    cosig: &NodeCoSignature,
    registry: &NodeRegistry,
) -> Result<(), CosignDowngrade> {
    // 1. Le nœud doit être pinné et non révoqué.
    let node = registry.lookup(&cosig.node_id).ok_or(CosignDowngrade::NodeNotPinned)?;
    if let NodeStatus::Revoked { .. } = node.status {
        return Err(CosignDowngrade::NodeRevoked);
    }
    // 2. Schéma supporté (ed25519 à v1).
    if cosig.scheme != CoSignatureScheme::Ed25519 {
        return Err(CosignDowngrade::UnsupportedScheme);
    }
    // 3. Recalcule le tuple depuis les champs du record (et l'empreinte-modèle co-signée, R3+, qui
    //    bascule le domaine en RC3) — et compare au tuple revendiqué. La signature couvre donc
    //    l'empreinte-modèle quand elle est présente : la falsifier invalide la co-signature.
    let tuple = cosign_tuple(
        &cosig.node_id,
        final_prompt_hash,
        output_hash,
        cosig.model_fingerprint.as_deref(),
        &cosig.nonce,
    );
    if tuple.to_hex() != cosig.signed_tuple_hash {
        return Err(CosignDowngrade::TupleMismatch);
    }
    // 4. Parse la clé publique pinnée (32 octets) et la signature (64 octets).
    let vk_bytes = hex_decode(&node.pubkey_hex).ok_or(CosignDowngrade::MalformedKey)?;
    let vk_arr: [u8; 32] = vk_bytes.as_slice().try_into().map_err(|_| CosignDowngrade::MalformedKey)?;
    let vk = VerifyingKey::from_bytes(&vk_arr).map_err(|_| CosignDowngrade::MalformedKey)?;
    let sig_bytes = hex_decode(&cosig.signature_hex).ok_or(CosignDowngrade::MalformedSignature)?;
    let sig_arr: [u8; 64] =
        sig_bytes.as_slice().try_into().map_err(|_| CosignDowngrade::MalformedSignature)?;
    let sig = Signature::from_bytes(&sig_arr);
    // 5. Vérifie la signature ed25519 sur le tuple — sans aucun secret. `verify_strict` impose
    //    l'encodage canonique (défense contre la malléabilité de signature).
    vk.verify_strict(&tuple.0, &sig).map_err(|_| CosignDowngrade::SignatureInvalid)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
//  R3+ — l'attestation-modèle (axe distinct du barreau : I-RC15)
// ─────────────────────────────────────────────────────────────────────────────

/// **R3+** — le verdict d'attestation-modèle d'une co-signature **déjà vérifiée** (l'appelant a
/// confirmé `verify_cosignature` = `Ok`). Décide si `model_hash_verified` peut être **tenu**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelAttestation {
    /// La co-signature ne porte **pas** d'empreinte-modèle (R3 production-seule) ⇒
    /// `model_hash_verified` reste `false` (honnête : le modèle n'est pas attesté).
    NotAttested,
    /// L'empreinte co-signée ∈ les `expected_models` pinnés du nœud ⇒ `model_hash_verified` peut
    /// être **tenu** : le nœud a co-signé avoir chargé un modèle **attendu**.
    Verified,
    /// L'empreinte co-signée n'est **pas** dans les `expected_models` du nœud (ou aucun n'est pinné)
    /// ⇒ **pas** de lift ; l'écart est signalé (le nœud a déclaré un modèle non attendu).
    Mismatch,
}

/// **R3+** — confronte l'empreinte-modèle co-signée aux `expected_models` pinnés du nœud.
///
/// Suppose que la co-signature a **déjà** été vérifiée (`verify_cosignature` = `Ok`) : c'est l'étape
/// *en plus* qui décide `model_hash_verified` — un **axe distinct** du barreau R3 (I-RC15). Pur,
/// hors ligne, sans secret. Ne **tient jamais** l'intégrité-modèle sans une empreinte ∈ pinnée (I-RC7).
pub fn model_attestation(cosig: &NodeCoSignature, registry: &NodeRegistry) -> ModelAttestation {
    let fp = match cosig.model_fingerprint.as_deref() {
        Some(fp) => fp,
        None => return ModelAttestation::NotAttested,
    };
    match registry.lookup(&cosig.node_id) {
        Some(n) if n.expected_models.iter().any(|e| e == fp) => ModelAttestation::Verified,
        _ => ModelAttestation::Mismatch,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  L'attestant (le nœud qui co-signe) + le MockAttestor (clé propre, pour les gates)
// ─────────────────────────────────────────────────────────────────────────────

/// **Le contrat d'attestation** côté nœud : produire une [`NodeCoSignature`] sur la computation.
///
/// C'est le **seam de production** : un vrai démon attestant distant (qui détient la clé privée sur
/// le nœud) implémente ce trait ; l'orchestrateur, sur le chemin distant, lui demande de co-signer
/// `(prompt_hash, output_hash, nonce)` et estampille R3 si la co-signature vérifie (sinon R4). Le
/// transport réel (réseau) est du déploiement ; le contrat, lui, est figé ici.
pub trait Attestor: Send + Sync {
    /// L'identité enrôlée du nœud (la clé du registre).
    fn node_id(&self) -> &str;
    /// La clé publique ed25519 (hex des 32 octets) — à **pinner** dans le registre.
    fn public_key_hex(&self) -> String;
    /// Co-signe la computation : signe `cosign_tuple(node_id, prompt, output, nonce)`.
    fn cosign(&self, final_prompt_hash: &Blake3Hash, output_hash: &Blake3Hash, nonce: &str) -> NodeCoSignature;
}

/// **[SIMULÉ]** Un attestant in-process à **clé propre** ed25519 — pour les gates/tests, sans nœud
/// distant ni infrastructure. Sa cryptographie est **réelle** (vraie `SigningKey`, vraies signatures
/// vérifiées par le vrai `VerifyingKey`) ; seul le *transport* est in-process et la *computation*
/// absente. Construit par `from_seed` ⇒ **déterministe** (même graine ⇒ même clé ⇒ gate reproductible,
/// sans horloge ni aléa — ADR-007 friendly).
#[derive(Clone)]
pub struct MockAttestor {
    node_id: String,
    signing: SigningKey,
    /// **R3+** : l'empreinte-modèle que cet attestant **déclare** faire tourner (et co-signera).
    /// `None` ⇒ co-signature production-seule (R3).
    model_fingerprint: Option<String>,
}

impl std::fmt::Debug for MockAttestor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Ne JAMAIS imprimer la clé privée.
        f.debug_struct("MockAttestor").field("node_id", &self.node_id).finish_non_exhaustive()
    }
}

impl MockAttestor {
    /// Construit un attestant déterministe à partir d'une graine de 32 octets (la clé privée brute).
    /// Production-seule par défaut (n'atteste pas de modèle) ; voir [`with_model`](MockAttestor::with_model).
    pub fn from_seed(node_id: impl Into<String>, seed: [u8; 32]) -> Self {
        Self { node_id: node_id.into(), signing: SigningKey::from_bytes(&seed), model_fingerprint: None }
    }

    /// **R3+** : déclare l'empreinte-modèle (hex) que cet attestant co-signera — le modèle qu'il
    /// « fait tourner ». Les co-signatures produites emploient alors le tuple RC3 et portent l'empreinte.
    pub fn with_model(mut self, fingerprint: impl Into<String>) -> Self {
        self.model_fingerprint = Some(fingerprint.into());
        self
    }
}

impl Attestor for MockAttestor {
    fn node_id(&self) -> &str {
        &self.node_id
    }
    fn public_key_hex(&self) -> String {
        hex_encode(self.signing.verifying_key().to_bytes().as_slice())
    }
    fn cosign(&self, final_prompt_hash: &Blake3Hash, output_hash: &Blake3Hash, nonce: &str) -> NodeCoSignature {
        let mf = self.model_fingerprint.as_deref();
        let tuple = cosign_tuple(&self.node_id, final_prompt_hash, output_hash, mf, nonce);
        let sig: Signature = self.signing.sign(&tuple.0);
        NodeCoSignature {
            node_id: self.node_id.clone(),
            scheme: CoSignatureScheme::Ed25519,
            signature_hex: hex_encode(&sig.to_bytes()),
            signed_tuple_hash: tuple.to_hex(),
            nonce: nonce.to_string(),
            model_fingerprint: self.model_fingerprint.clone(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  L'estampillage conditionnel (côté orchestrateur) — R3 si co-sig valide, sinon R4
// ─────────────────────────────────────────────────────────────────────────────

/// **R3 — l'estampillage conditionnel.** À l'issue d'un appel DISTANT (barreau de base R4), si un
/// attestant est présent et que l'appel a produit une sortie, l'orchestrateur lui fait **co-signer**
/// la computation, puis **vérifie cette co-signature avant d'élever** : il ne hisse R4 → R3 que si
/// la co-signature est **VALIDE** ; sur tout échec, le barreau de base R4 est conservé inchangé.
///
/// **L'estampillage est conditionné à la VALIDITÉ, jamais à la simple PRÉSENCE** (doc archi §6.1
/// étape 4 / §11, I-RC9 « no false elevation »). Avant de hisser, l'orchestrateur contrôle que (1) le
/// `nonce` co-signé est bien **le sien** — le défi per-appel qu'il a émis (I-RC12) ; c'est la
/// **fraîcheur** que le vérifieur hors ligne ne peut PAS recontrôler (il ne connaît pas le défi émis),
/// donc le seul point où elle peut l'être — et que (2) la signature **vérifie** contre la pubkey
/// pinnée du `node_id` via [`verify_cosignature`] (nœud pinné, non révoqué, schéma supporté, tuple
/// lié au contenu, signature ed25519 canonique). C'est une défense **fail-fast en profondeur** :
/// `n17_chain::assess_held_strength` re-vérifie de toute façon chaque R3 hors ligne et redescend les
/// faux à R4 (I-RC9) — mais on n'estampille jamais un R3 qu'on sait déjà invalide au point d'émission.
///
/// Cette fonction n'agit **que** sur R4 ([`ComputeAttestation::R4RemoteChannel`]) : un R2 local ou un
/// legacy ne sont jamais hissés (un nœud distant ne peut pas attester une computation locale). C'est
/// le contrat que le backend appelle au moment de l'appel ; le transport réseau vers un vrai démon
/// attestant est du déploiement.
pub fn cosign_stamp(
    base: &ComputeAttestation,
    attestor: Option<&dyn Attestor>,
    registry: &NodeRegistry,
    final_prompt_hash: &Blake3Hash,
    output_hash: Option<&Blake3Hash>,
    nonce: &str,
) -> ComputeAttestation {
    match (attestor, base, output_hash) {
        (Some(att), ComputeAttestation::R4RemoteChannel { endpoint, .. }, Some(oh)) => {
            let cosignature = att.cosign(final_prompt_hash, oh, nonce);
            // Fail-fast (architecture §6) : on n'estampille R3 que si (1) le nœud a co-signé LE DÉFI
            // ÉMIS (il ne l'a pas ignoré au profit d'un autre nonce) ET (2) la co-signature VÉRIFIE
            // contre la pubkey pinnée. Sinon on garde le R4 de base — jamais d'élévation non
            // soutenue (I-RC9). NB honnête : ceci ne prouve PAS la *fraîcheur* du défi lui-même
            // (qu'il soit unique et non rejoué d'un run passé) — c'est la responsabilité du seam
            // d'émission orchestrateur (génération d'un défi unique + suivi des nonces consommés,
            // I-RC12), hors de cet incrément. Le vérifieur hors ligne y ajoute l'unicité INTRA-RUN
            // (cf. `n17_chain::assess_held_strength`, anomalie `NonceReused`).
            let node_signed_issued_challenge = cosignature.nonce == nonce;
            let cosig_verifies =
                verify_cosignature(final_prompt_hash, oh, &cosignature, registry).is_ok();
            if node_signed_issued_challenge && cosig_verifies {
                ComputeAttestation::R3RemoteCosigned { endpoint: endpoint.clone(), cosignature }
            } else {
                base.clone()
            }
        }
        _ => base.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::EndpointIdentity;

    fn hashes() -> (Blake3Hash, Blake3Hash) {
        (blake3_bytes(b"the prompt"), blake3_bytes(b"the output"))
    }

    fn r4(host: &str) -> ComputeAttestation {
        ComputeAttestation::R4RemoteChannel {
            endpoint: EndpointIdentity::new(host, "sha256/CHANNELPIN"),
            keyed: false,
        }
    }

    // ── hex round-trip ───────────────────────────────────────────────────────
    #[test]
    fn hex_round_trips() {
        let b = [0u8, 1, 15, 16, 255, 128, 64];
        assert_eq!(hex_decode(&hex_encode(&b)).unwrap(), b);
        assert!(hex_decode("xyz").is_none()); // non-hex
        assert!(hex_decode("abc").is_none()); // longueur impaire
    }

    // ── le tuple est déterministe et sensible à CHAQUE champ ───────────────────
    #[test]
    fn tuple_is_deterministic_and_field_sensitive() {
        let (p, o) = hashes();
        let base = cosign_tuple("node-1", &p, &o, None, "nonce-A");
        assert_eq!(base, cosign_tuple("node-1", &p, &o, None, "nonce-A"), "déterministe");
        assert_ne!(base, cosign_tuple("node-2", &p, &o, None, "nonce-A"), "node_id compte");
        assert_ne!(base, cosign_tuple("node-1", &o, &o, None, "nonce-A"), "prompt compte");
        assert_ne!(base, cosign_tuple("node-1", &p, &p, None, "nonce-A"), "output compte");
        assert_ne!(base, cosign_tuple("node-1", &p, &o, None, "nonce-B"), "nonce compte");
    }

    #[test]
    fn tuple_has_no_concatenation_ambiguity() {
        // Sans préfixes de longueur, ("ab","c") et ("a","bc") pourraient collisionner. Ici, non.
        let (_, o) = hashes();
        let p = Blake3Hash::ZERO;
        assert_ne!(
            cosign_tuple("ab", &p, &o, None, "c"),
            cosign_tuple("a", &p, &o, None, "bc"),
            "les préfixes de longueur ferment l'ambiguïté de concaténation"
        );
    }

    // ── R3+ : le tuple modèle est distinct (domaine RC3 + empreinte), sensible à l'empreinte ──
    #[test]
    fn model_tuple_is_distinct_and_fingerprint_sensitive() {
        let (p, o) = hashes();
        let r3 = cosign_tuple("node-1", &p, &o, None, "n");
        let r3p = cosign_tuple("node-1", &p, &o, Some("sha256/MODEL-A"), "n");
        // R3 production-seule ≠ R3+ même contenu (domaine + composante modèle).
        assert_ne!(r3, r3p, "R3 (sans modèle) ≠ R3+ (avec modèle) — domaines distincts");
        // l'empreinte compte.
        assert_ne!(r3p, cosign_tuple("node-1", &p, &o, Some("sha256/MODEL-B"), "n"), "l'empreinte compte");
        // déterministe.
        assert_eq!(r3p, cosign_tuple("node-1", &p, &o, Some("sha256/MODEL-A"), "n"), "déterministe");
        // R3 (None) reste byte-identique au tuple R3 historique (domaine RC2) — non-régression.
        assert_eq!(r3, cosign_tuple("node-1", &p, &o, None, "n"));
    }

    // ── R3+ : l'attestation-modèle (vérifie + confronte au pin) ────────────────
    #[test]
    fn model_attesting_cosig_verifies_and_matches_expected() {
        let (p, o) = hashes();
        let fp = "sha256/MODEL-A";
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]).with_model(fp);
        let mut reg = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        reg.expect_model("gpu-1", fp);
        let cosig = att.cosign(&p, &o, "challenge");
        // la co-signature couvre l'empreinte (tuple RC3) ET vérifie ;
        assert_eq!(verify_cosignature(&p, &o, &cosig, &reg), Ok(()));
        // ET l'empreinte ∈ attendues → model_hash_verified peut être tenu.
        assert_eq!(model_attestation(&cosig, &reg), ModelAttestation::Verified);
    }

    #[test]
    fn model_attestation_mismatch_when_fingerprint_not_pinned() {
        let (p, o) = hashes();
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]).with_model("sha256/ACTUAL");
        let mut reg = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        reg.expect_model("gpu-1", "sha256/EXPECTED"); // ≠ ce que le nœud co-signe
        let cosig = att.cosign(&p, &o, "challenge");
        assert_eq!(verify_cosignature(&p, &o, &cosig, &reg), Ok(()), "la co-sig reste valide…");
        assert_eq!(model_attestation(&cosig, &reg), ModelAttestation::Mismatch, "…mais mauvais modèle → pas de lift");
    }

    #[test]
    fn production_only_cosig_is_not_attested() {
        let (p, o) = hashes();
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]); // pas de modèle (R3 production-seule)
        let reg = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let cosig = att.cosign(&p, &o, "challenge");
        assert!(cosig.model_fingerprint.is_none());
        assert_eq!(verify_cosignature(&p, &o, &cosig, &reg), Ok(()));
        assert_eq!(model_attestation(&cosig, &reg), ModelAttestation::NotAttested);
    }

    #[test]
    fn tampering_the_model_fingerprint_breaks_the_cosignature() {
        let (p, o) = hashes();
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]).with_model("sha256/MODEL-A");
        let reg = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let mut cosig = att.cosign(&p, &o, "challenge");
        // un attaquant réécrit l'empreinte (faire croire à un autre modèle) → le tuple recalculé
        // diffère du signed_tuple_hash → la signature ne couvre plus → rejet.
        cosig.model_fingerprint = Some("sha256/FORGED".into());
        assert_eq!(verify_cosignature(&p, &o, &cosig, &reg), Err(CosignDowngrade::TupleMismatch));
    }

    // ── nominal : co-signe → pin → vérifie → OK ────────────────────────────────
    #[test]
    fn cosign_then_verify_against_pinned_key_succeeds() {
        let (p, o) = hashes();
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let reg = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let cosig = att.cosign(&p, &o, "challenge-0001");
        assert_eq!(verify_cosignature(&p, &o, &cosig, &reg), Ok(()));
    }

    // ── forge de clé : un AUTRE attestant signe → la pubkey pinnée ne matche pas ─
    #[test]
    fn forged_signature_from_a_different_key_is_rejected() {
        let (p, o) = hashes();
        let real = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let imposter = MockAttestor::from_seed("gpu-1", [9u8; 32]); // même node_id, AUTRE clé
        let reg = NodeRegistry::empty().with_node("gpu-1", real.public_key_hex());
        let forged = imposter.cosign(&p, &o, "challenge-0001");
        assert_eq!(verify_cosignature(&p, &o, &forged, &reg), Err(CosignDowngrade::SignatureInvalid));
    }

    // ── tuple altéré (MITM) : on change la sortie après co-signature ───────────
    #[test]
    fn tampered_output_breaks_the_tuple_binding() {
        let (p, o) = hashes();
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let reg = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let cosig = att.cosign(&p, &o, "challenge-0001");
        // le vérifieur recompose le tuple avec une AUTRE sortie → ne matche plus le signed_tuple_hash.
        let other_output = blake3_bytes(b"a different output");
        assert_eq!(
            verify_cosignature(&p, &other_output, &cosig, &reg),
            Err(CosignDowngrade::TupleMismatch)
        );
    }

    // ── nœud non pinné / révoqué ───────────────────────────────────────────────
    #[test]
    fn unpinned_node_is_rejected() {
        let (p, o) = hashes();
        let att = MockAttestor::from_seed("ghost", [3u8; 32]);
        let reg = NodeRegistry::empty(); // rien d'enrôlé
        let cosig = att.cosign(&p, &o, "n");
        assert_eq!(verify_cosignature(&p, &o, &cosig, &reg), Err(CosignDowngrade::NodeNotPinned));
    }

    #[test]
    fn revoked_node_is_rejected() {
        let (p, o) = hashes();
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let mut reg = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let cosig = att.cosign(&p, &o, "n");
        assert_eq!(verify_cosignature(&p, &o, &cosig, &reg), Ok(()), "actif → OK");
        reg.revoke("gpu-1", "clé compromise");
        assert_eq!(verify_cosignature(&p, &o, &cosig, &reg), Err(CosignDowngrade::NodeRevoked), "révoqué → rejet");
    }

    // ── malformés ──────────────────────────────────────────────────────────────
    #[test]
    fn malformed_key_and_signature_are_rejected() {
        let (p, o) = hashes();
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let good = att.cosign(&p, &o, "n");
        // clé pinnée malformée — (a) hex invalide (longueur impaire / non-hex).
        let reg_bad_key = NodeRegistry::empty().with_node("gpu-1", "zz");
        assert_eq!(verify_cosignature(&p, &o, &good, &reg_bad_key), Err(CosignDowngrade::MalformedKey));
        // (b) hex VALIDE mais mauvaise longueur (≠ 32 octets) → la conversion en [u8;32] échoue.
        let reg_short_key = NodeRegistry::empty().with_node("gpu-1", "abcd"); // 2 octets valides
        assert_eq!(verify_cosignature(&p, &o, &good, &reg_short_key), Err(CosignDowngrade::MalformedKey));
        // signature malformée — le tuple doit rester cohérent pour atteindre l'étape signature.
        let reg = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let mut bad_sig = good.clone();
        bad_sig.signature_hex = "abcd".into(); // 2 octets, pas 64
        assert_eq!(verify_cosignature(&p, &o, &bad_sig, &reg), Err(CosignDowngrade::MalformedSignature));
        // (c) hex non-hex pour la signature → MalformedSignature aussi.
        let mut nonhex_sig = good.clone();
        nonhex_sig.signature_hex = "zz".repeat(32); // 64 chars mais non-hex
        assert_eq!(verify_cosignature(&p, &o, &nonhex_sig, &reg), Err(CosignDowngrade::MalformedSignature));
    }

    #[test]
    fn never_prints_the_private_key() {
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let dbg = format!("{att:?}");
        assert!(dbg.contains("gpu-1"));
        assert!(!dbg.contains("signing"), "le Debug n'expose pas la clé privée");
    }

    // ── l'estampillage conditionnel : R4 → R3 vérifiable ───────────────────────
    #[test]
    fn cosign_stamp_lifts_r4_to_a_verifiable_r3() {
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let (p, o) = hashes();
        let stamped =
            cosign_stamp(&r4("gpu.internal"), Some(&att), &registry, &p, Some(&o), "challenge-1");
        match &stamped {
            ComputeAttestation::R3RemoteCosigned { cosignature, .. } => {
                // boucle fermée : l'estampillage n'a hissé QU'APRÈS vérification, et le R3 produit
                // re-vérifie (nonce du défi co-signé, signature valide contre la pubkey pinnée).
                assert_eq!(cosignature.nonce, "challenge-1");
                assert_eq!(verify_cosignature(&p, &o, cosignature, &registry), Ok(()));
            }
            other => panic!("attendu R3, obtenu {:?}", other.rung_label()),
        }
    }

    #[test]
    fn cosign_stamp_keeps_base_without_attestor_output_or_on_local() {
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let (p, o) = hashes();
        // pas d'attestant → reste R4.
        assert_eq!(cosign_stamp(&r4("h"), None, &registry, &p, Some(&o), "n"), r4("h"));
        // pas de sortie (appel échoué) → reste R4.
        assert_eq!(cosign_stamp(&r4("h"), Some(&att), &registry, &p, None, "n"), r4("h"));
        // R2 local n'est JAMAIS hissé en R3 (un nœud distant n'atteste pas une computation locale).
        let r2 = ComputeAttestation::R2LocalWitnessed;
        assert_eq!(cosign_stamp(&r2, Some(&att), &registry, &p, Some(&o), "n"), r2);
    }

    /// **[SIMULÉ]** Un attestant **malveillant/buggé** : il **ignore** le nonce-défi de l'orchestrateur
    /// et co-signe SON propre nonce. Sa cryptographie reste réelle (vraie clé, signature qui vérifie
    /// sur le tuple), donc `verify_cosignature` l'accepte hors ligne. C'est le cas que `cosign_stamp`
    /// attrape au point d'émission (il compare le nonce co-signé au défi qu'il a émis) — un nœud qui
    /// n'a pas signé MON défi. (La fraîcheur PLEINE — défi unique, jamais rejoué d'un run passé —
    /// reste l'affaire de l'émission orchestrateur ; le vérifieur hors ligne, lui, attrape la
    /// réutilisation INTRA-RUN d'un nonce.)
    struct StaleNonceAttestor {
        inner: MockAttestor,
        stale_nonce: String,
    }

    impl Attestor for StaleNonceAttestor {
        fn node_id(&self) -> &str {
            self.inner.node_id()
        }
        fn public_key_hex(&self) -> String {
            self.inner.public_key_hex()
        }
        fn cosign(&self, final_prompt_hash: &Blake3Hash, output_hash: &Blake3Hash, _nonce: &str) -> NodeCoSignature {
            // ignore le défi reçu → co-signe son nonce périmé à la place.
            self.inner.cosign(final_prompt_hash, output_hash, &self.stale_nonce)
        }
    }

    // ── co-sig invalide (mauvaise clé / nonce périmé) → PAS d'élévation, R4 conservé ────
    #[test]
    fn cosign_stamp_rejects_invalid_cosignature_and_keeps_r4() {
        let (p, o) = hashes();

        // (a) MAUVAISE CLÉ : la pubkey pinnée n'est pas celle de l'attestant (même node_id, autre
        //     clé) → la signature ne vérifie pas → on ne hisse pas, R4 conservé.
        let real = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let imposter = MockAttestor::from_seed("gpu-1", [9u8; 32]);
        let reg = NodeRegistry::empty().with_node("gpu-1", real.public_key_hex());
        assert_eq!(
            cosign_stamp(&r4("h"), Some(&imposter), &reg, &p, Some(&o), "challenge-1"),
            r4("h"),
            "co-sig à clé forgée → pas d'élévation"
        );

        // (b) NONCE PÉRIMÉ : l'attestant co-signe un nonce ≠ défi émis. La co-signature VÉRIFIE
        //     pourtant hors ligne (clé pinnée, tuple cohérent) — seul le contrôle de fraîcheur de
        //     `cosign_stamp` l'attrape → R4 conservé.
        let honest = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let reg_ok = NodeRegistry::empty().with_node("gpu-1", honest.public_key_hex());
        let stale = StaleNonceAttestor { inner: honest, stale_nonce: "OLD-CHALLENGE".into() };
        // garde-fou : la co-signature périmée vérifierait bien hors ligne (le trou que l'on ferme).
        let leaked = stale.cosign(&p, &o, "challenge-1");
        assert_eq!(verify_cosignature(&p, &o, &leaked, &reg_ok), Ok(()));
        assert_eq!(
            cosign_stamp(&r4("h"), Some(&stale), &reg_ok, &p, Some(&o), "challenge-1"),
            r4("h"),
            "nonce périmé (≠ défi émis) → pas d'élévation"
        );
    }
}
