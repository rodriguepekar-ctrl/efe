// SPDX-License-Identifier: UNLICENSED
//! # `attest` — R3 / RC-2 : le **seam de production** côté orchestrateur (fraîcheur I-RC12).
//!
//! La revue adversariale de R3 a confirmé un gap d'honnêteté : l'architecture promettait une
//! *garde de fraîcheur* (I-RC12 — défi unique, jamais rejoué) que le code ne tenait que pour
//! l'**écho** (« le nœud a-t-il signé MON défi ? ») et l'**unicité intra-run** (côté vérifieur
//! hors ligne). Manquait la pièce **mint-time** : qui **génère un défi unique** et **suit les
//! nonces consommés** pour qu'un même défi ne soit jamais honoré deux fois ? C'est ce module.
//!
//! Il vit côté **orchestrateur** (état mutable, jamais scellé) :
//! - [`ChallengeIssuer`] : émet des nonces-défis **uniques et imprévisibles** (sans graine, on ne
//!   peut pas pré-deviner le prochain défi → anti-pré-signature) et **suit leur consommation**
//!   (un défi rendu une fois ne peut pas l'être deux fois → anti-rejeu mint-time).
//! - [`attest_remote_call`] : le flux complet §6 de l'architecture — **émet** un défi, fait
//!   **co-signer** le nœud (via le trait [`Attestor`], que le transport soit in-process ou réseau),
//!   puis **n'estampille R3 que si** le nœud a signé CE défi (écho), que la co-signature **vérifie**
//!   contre la pubkey pinnée, ET que le défi est **frais** (rendu avec succès). Tout échec ⇒ on
//!   garde le R4 de base (I-RC9 : aucune élévation sans preuve).
//!
//! ## Frontière de déploiement (honnête)
//! Le **transport** vers un vrai nœud distant (un démon attestant détenant la clé privée, joint par
//! le réseau) est une implémentation du trait [`Attestor`] — du **déploiement**, hors de ce crate
//! pur. `attest_remote_call` est agnostique au transport : un [`MockAttestor`](crate::cosign::MockAttestor)
//! in-process (gates) ou un futur `RemoteAttestor` réseau s'y branchent à l'identique.
//!
//! ## Déterminisme (ADR-007)
//! La graine du [`ChallengeIssuer`] est **fournie par l'appelant** (un secret côté serveur) ; les
//! défis en découlent par `blake3(domaine ‖ graine ‖ compteur)` — **déterministes à graine fixée**
//! (tests reproductibles), **imprévisibles à graine secrète** (le nœud ne peut pas pré-signer). Le
//! nonce n'entre jamais dans l'ordre canonique (qui trie par `provenance_id`) ; R3 n'est de toute
//! façon jamais R1-reproductible. Aucune horloge, aucun aléa non maîtrisé dans le chemin scellé.

use std::collections::HashSet;

use efe_nano_sdk::{Blake3Hash, ComputeAttestation};

use crate::cosign::{verify_cosignature, Attestor, NodeRegistry};
use crate::hash::blake3_bytes;

/// Pourquoi un nonce-défi n'a pas pu être **rendu** (consommé) — la garde anti-rejeu mint-time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessError {
    /// Le nonce n'a jamais été émis par cet émetteur (forgé, ou défi d'un autre émetteur).
    NonceNotIssued,
    /// Le nonce a **déjà été consommé** — c'est exactement le rejeu que la garde refuse.
    NonceAlreadyConsumed,
}

impl FreshnessError {
    /// Libellé stable (forensique).
    pub fn label(self) -> &'static str {
        match self {
            FreshnessError::NonceNotIssued => "nonce-not-issued",
            FreshnessError::NonceAlreadyConsumed => "nonce-already-consumed",
        }
    }
}

/// **R3 / I-RC12 (volet mint-time)** — émet des **défis uniques** et **suit leur consommation**.
///
/// La fraîcheur pleine est une propriété de *liveness* que le vérifieur hors ligne ne peut PAS
/// fournir (il ne connaît pas les défis émis) : c'est ici, à l'**émission**, qu'elle est tenue.
/// - **Unicité** : un compteur monotone garantit que deux défis ne collisionnent jamais.
/// - **Imprévisibilité** : le défi dérive d'une **graine secrète** (`blake3(domaine ‖ graine ‖
///   compteur)`) — sans la graine, on ne peut pas pré-deviner le prochain défi (anti-pré-signature).
/// - **Anti-rejeu** : [`redeem`](ChallengeIssuer::redeem) ne rend un défi **qu'une fois** ; un défi
///   déjà consommé (ou jamais émis) est refusé.
///
/// État **mutable, côté orchestrateur, jamais scellé**. Non-`Sync` par `&mut` : l'appelant
/// concurrent l'enveloppe (p. ex. `Arc<Mutex<…>>`).
#[derive(Debug, Clone)]
pub struct ChallengeIssuer {
    seed: [u8; 32],
    counter: u64,
    issued: HashSet<String>,
    consumed: HashSet<String>,
}

impl ChallengeIssuer {
    /// Construit un émetteur à partir d'une **graine secrète** (fournie par l'orchestrateur ; en
    /// production, dérivée d'un secret côté serveur — jamais journalisée). Graine fixée ⇒ suite de
    /// défis déterministe (tests) ; graine secrète ⇒ défis imprévisibles (production).
    pub fn new(seed: [u8; 32]) -> Self {
        Self { seed, counter: 0, issued: HashSet::new(), consumed: HashSet::new() }
    }

    /// **Émet** un nonce-défi frais et unique (et le marque *en attente*). Anti-pré-signature
    /// (imprévisible sans la graine) + anti-collision (compteur monotone).
    pub fn issue(&mut self) -> String {
        let mut buf = Vec::with_capacity(21 + 32 + 8);
        buf.extend_from_slice(b"EFE-RC2-CHALLENGE-v1\x00"); // séparation de domaine
        buf.extend_from_slice(&self.seed);
        buf.extend_from_slice(&self.counter.to_le_bytes());
        self.counter = self.counter.wrapping_add(1);
        let nonce = blake3_bytes(&buf).to_hex();
        self.issued.insert(nonce.clone());
        nonce
    }

    /// **Rend** (consomme) un nonce-défi : succès **une seule fois**, si le défi a bien été émis et
    /// n'a pas déjà été consommé. C'est la garde « nonce déjà consommé → rejeté ».
    pub fn redeem(&mut self, nonce: &str) -> Result<(), FreshnessError> {
        if self.consumed.contains(nonce) {
            return Err(FreshnessError::NonceAlreadyConsumed);
        }
        if !self.issued.remove(nonce) {
            return Err(FreshnessError::NonceNotIssued);
        }
        self.consumed.insert(nonce.to_string());
        Ok(())
    }

    /// Nombre de défis émis mais pas encore rendus (en attente).
    pub fn pending(&self) -> usize {
        self.issued.len()
    }
    /// Nombre de défis consommés.
    pub fn consumed_count(&self) -> usize {
        self.consumed.len()
    }
}

/// **R3 — le flux de production complet (orchestrateur)**, agnostique au transport.
///
/// À l'issue d'un appel **distant** (barreau de base R4), avec un attestant disponible et une sortie
/// produite :
/// 1. **émet** un défi frais ([`ChallengeIssuer::issue`]) ;
/// 2. fait **co-signer** le nœud sur ce défi (via [`Attestor::cosign`] — transport quelconque) ;
/// 3. n'**estampille R3** que si les **trois** garanties tiennent, sinon garde le R4 de base
///    (I-RC9 — aucune élévation sans preuve) :
///    - **écho** (I-RC12 a) : le nœud a signé *exactement* le défi émis (pas un autre nonce) ;
///    - **validité** (I-RC6) : la co-signature vérifie contre la pubkey **pinnée** (`registry`) ;
///    - **fraîcheur** (I-RC12 c) : le défi est **rendu** avec succès (unique, non rejoué).
///
/// Renvoie le [`ComputeAttestation`] estampillé. Le défi émis est rendu (consommé) **ssi** l'écho
/// tient — sinon il reste en attente (jamais ré-honoré ; un défi gaspillé n'élève rien).
pub fn attest_remote_call(
    base: &ComputeAttestation,
    attestor: Option<&dyn Attestor>,
    registry: &NodeRegistry,
    issuer: &mut ChallengeIssuer,
    final_prompt_hash: &Blake3Hash,
    output_hash: Option<&Blake3Hash>,
) -> ComputeAttestation {
    match (attestor, base, output_hash) {
        (Some(att), ComputeAttestation::R4RemoteChannel { endpoint, .. }, Some(oh)) => {
            let challenge = issuer.issue(); // 1. défi frais et unique
            let cosignature = att.cosign(final_prompt_hash, oh, &challenge); // 2. co-signature distante
            // 3. écho + validité + fraîcheur. L'écho conditionne le redeem : un nœud qui n'a pas
            //    signé NOTRE défi ne consomme pas le défi émis (il reste en attente, jamais honoré).
            let echoed = cosignature.nonce == challenge;
            let valid = verify_cosignature(final_prompt_hash, oh, &cosignature, registry).is_ok();
            let fresh = echoed && issuer.redeem(&challenge).is_ok();
            if echoed && valid && fresh {
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
    use crate::cosign::MockAttestor;
    use efe_nano_sdk::{CoSignatureScheme, EndpointIdentity, NodeCoSignature};

    fn hashes() -> (Blake3Hash, Blake3Hash) {
        (blake3_bytes(b"the prompt"), blake3_bytes(b"the output"))
    }
    fn r4() -> ComputeAttestation {
        ComputeAttestation::R4RemoteChannel {
            endpoint: EndpointIdentity::new("gpu.internal", "sha256/CHANNELPIN"),
            keyed: false,
        }
    }

    // ── ChallengeIssuer : unicité, imprévisibilité, anti-rejeu ─────────────────
    #[test]
    fn issued_challenges_are_unique() {
        let mut iss = ChallengeIssuer::new([1u8; 32]);
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(iss.issue()), "chaque défi est unique");
        }
        assert_eq!(iss.pending(), 1000);
    }

    #[test]
    fn distinct_seeds_yield_distinct_challenge_streams() {
        // imprévisibilité (proxy) : sans la graine, la suite de défis diffère totalement.
        let mut a = ChallengeIssuer::new([1u8; 32]);
        let mut b = ChallengeIssuer::new([2u8; 32]);
        assert_ne!(a.issue(), b.issue(), "graines distinctes → défis distincts au même compteur");
    }

    #[test]
    fn a_challenge_redeems_once_then_replay_is_rejected() {
        let mut iss = ChallengeIssuer::new([7u8; 32]);
        let n = iss.issue();
        assert_eq!(iss.redeem(&n), Ok(()), "premier rendu : OK");
        assert_eq!(iss.redeem(&n), Err(FreshnessError::NonceAlreadyConsumed), "rejeu : refusé");
        assert_eq!(iss.consumed_count(), 1);
        assert_eq!(iss.pending(), 0);
    }

    #[test]
    fn redeeming_an_unissued_challenge_is_rejected() {
        let mut iss = ChallengeIssuer::new([7u8; 32]);
        assert_eq!(iss.redeem("never-issued"), Err(FreshnessError::NonceNotIssued));
    }

    // ── attest_remote_call : produit un R3 frais et vérifiable ─────────────────
    #[test]
    fn attest_remote_call_produces_a_fresh_verifiable_r3() {
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let mut issuer = ChallengeIssuer::new([42u8; 32]);
        let (p, o) = hashes();
        let stamped = attest_remote_call(&r4(), Some(&att), &registry, &mut issuer, &p, Some(&o));
        match &stamped {
            ComputeAttestation::R3RemoteCosigned { cosignature, .. } => {
                // la co-signature vérifie hors ligne, ET le défi a été consommé (fraîcheur tenue).
                assert_eq!(verify_cosignature(&p, &o, cosignature, &registry), Ok(()));
                assert_eq!(issuer.consumed_count(), 1, "le défi est consommé");
                assert_eq!(issuer.pending(), 0, "aucun défi en attente");
            }
            other => panic!("attendu R3, obtenu {}", other.rung_label()),
        }
    }

    #[test]
    fn attest_remote_call_keeps_r4_when_node_ignores_the_challenge() {
        // un nœud qui co-signe SON nonce au lieu du défi émis → écho faux → pas d'élévation.
        struct IgnoresChallenge {
            inner: MockAttestor,
        }
        impl Attestor for IgnoresChallenge {
            fn node_id(&self) -> &str {
                self.inner.node_id()
            }
            fn public_key_hex(&self) -> String {
                self.inner.public_key_hex()
            }
            fn cosign(&self, fph: &Blake3Hash, oh: &Blake3Hash, _challenge: &str) -> NodeCoSignature {
                self.inner.cosign(fph, oh, "a-nonce-of-my-own") // ignore le défi
            }
        }
        let inner = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", inner.public_key_hex());
        let att = IgnoresChallenge { inner };
        let mut issuer = ChallengeIssuer::new([42u8; 32]);
        let (p, o) = hashes();
        let stamped = attest_remote_call(&r4(), Some(&att), &registry, &mut issuer, &p, Some(&o));
        assert_eq!(stamped, r4(), "le nœud n'a pas signé MON défi → R4 conservé");
        assert_eq!(issuer.consumed_count(), 0, "le défi n'est pas consommé (écho faux)");
        // garde-fou : la co-signature périmée vérifierait pourtant bien hors ligne (le trou que
        // seule la fraîcheur mint-time ferme).
        let leaked = att.cosign(&p, &o, "ignored");
        assert_eq!(verify_cosignature(&p, &o, &leaked, &registry), Ok(()));
    }

    #[test]
    fn attest_remote_call_keeps_base_without_attestor_or_output_or_on_local() {
        let att = MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let mut issuer = ChallengeIssuer::new([42u8; 32]);
        let (p, o) = hashes();
        // pas d'attestant.
        assert_eq!(attest_remote_call(&r4(), None, &registry, &mut issuer, &p, Some(&o)), r4());
        // pas de sortie.
        assert_eq!(attest_remote_call(&r4(), Some(&att), &registry, &mut issuer, &p, None), r4());
        // base locale (R2) jamais hissée.
        let r2 = ComputeAttestation::R2LocalWitnessed;
        assert_eq!(attest_remote_call(&r2, Some(&att), &registry, &mut issuer, &p, Some(&o)), r2);
        assert_eq!(issuer.consumed_count(), 0, "aucun défi consommé sur les chemins non-R3");
    }

    #[test]
    fn attest_remote_call_keeps_r4_when_node_not_pinned() {
        // l'attestant n'est pas dans le registre → la co-signature ne vérifie pas → R4.
        let att = MockAttestor::from_seed("ghost", [3u8; 32]);
        let registry = NodeRegistry::empty(); // rien d'enrôlé
        let mut issuer = ChallengeIssuer::new([42u8; 32]);
        let (p, o) = hashes();
        let _ = CoSignatureScheme::Ed25519; // (garde l'import explicite)
        assert_eq!(attest_remote_call(&r4(), Some(&att), &registry, &mut issuer, &p, Some(&o)), r4());
    }
}
