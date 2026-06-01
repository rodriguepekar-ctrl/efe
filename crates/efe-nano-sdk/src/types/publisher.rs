//! **Sprint 10 — N18** : Publisher identity types.
//!
//! See `docs/architecture/EFE_M50_N18_PUBLISHER_IDENTITY_ARCHITECTURE.md`.
//!
//! # Module surface
//!
//! - [`PublisherId`] — opaque per-publisher identifier.
//! - [`KeyId`] — per-signing-key identifier.
//! - [`KycTier`] — 4-rung KYC ladder (Tier 0..Tier 3).
//! - [`PublisherStatus`] — Active / SanctionsCheckStale / Suspended / Revoked.
//! - [`PublisherKey`] — public key + algorithm + validity window.
//! - [`HistoricalKey`] — pre-rotation keys retained for verification.
//! - [`PublisherSignature`] — Ed25519 signature over canonical body.
//! - [`PublisherIdentity`] — full registry record.
//! - [`NewPublisherCandidate`] — registration submission.
//! - [`RevocationCertificate`] — non-repudiable revocation record.
//! - [`RevocationReason`] — taxonomy.
//! - [`CoPublishingPolicy`] — N-of-M signature threshold (Sprint 11 runtime).
//! - [`OperatorSignature`] — counter-signature for tier promotion.
//! - [`Iso3166`] — country code newtype.
//! - [`Sha256Hash`] — canonical body hash.

use std::time::SystemTime;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Sha256Hash
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : 32-byte SHA-256 hash newtype.
///
/// Used for the canonical body hash signed by publishers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256Hash(pub [u8; 32]);

impl Sha256Hash {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(b)
    }

    pub const ZERO: Self = Self([0u8; 32]);

    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }

    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for b in &self.0 {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }

    pub fn parse_hex(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            let hi = hex_nibble(s.as_bytes()[i * 2])?;
            let lo = hex_nibble(s.as_bytes()[i * 2 + 1])?;
            *byte = (hi << 4) | lo;
        }
        Some(Self(out))
    }
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl std::fmt::Display for Sha256Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Sha256Hash {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Sha256Hash {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let s = String::deserialize(de)?;
        Self::parse_hex(&s).ok_or_else(|| Error::custom("malformed sha256 hex digest"))
    }
}

impl Default for Sha256Hash {
    fn default() -> Self {
        Self::ZERO
    }
}

// ─────────────────────────────────────────────────────────────────────
//  PublisherId + KeyId
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : opaque publisher identifier.
///
/// **Invariant I14** : never reused — even after revocation, the
/// UUID is retained forever in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublisherId(pub Uuid);

impl PublisherId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
    /// **N18** : Sentinel ID for the `Author::Efe` reserved publisher.
    pub const fn efe_reserved() -> Self {
        Self(Uuid::nil())
    }
    /// **N18** : True if this is the reserved EFE publisher.
    pub fn is_efe_reserved(&self) -> bool {
        self.0 == Uuid::nil()
    }
}

impl Default for PublisherId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PublisherId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// **Sprint 10 — N18** : per-signing-key identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyId(pub Uuid);

impl KeyId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for KeyId {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  KycTier
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : 4-rung KYC ladder.
///
/// See architecture ADR-9 + annex A1.
///
/// **Wire representation (post-audit fix)** : serde and `label()` must
/// agree — both emit `tier_0` / `tier_1` / `tier_2` / `tier_3`.  Pre-
/// audit, `rename_all = "snake_case"` produced `tier0` (no
/// underscore) because heck's snake_case doesn't separate letters
/// from digits, while `label()` (used for metrics) returned
/// `tier_0`.  That divergence made downstream JOINs between JSON
/// payloads and Prometheus labels fail.  We pin the serde
/// representation explicitly here to match `label()` exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum KycTier {
    /// Verified email only ; sandbox-only catalog ; pseudonymous allowed.
    #[serde(rename = "tier_0")]
    Tier0,
    /// Email + phone + 30d cooling period ; community catalog.
    #[serde(rename = "tier_1")]
    Tier1,
    /// KYC organisation (legal entity) ; production catalog.
    #[serde(rename = "tier_2")]
    Tier2,
    /// Tier 2 + insurance bond + enterprise SLA ; fast-path promotion.
    #[serde(rename = "tier_3")]
    Tier3,
}

impl KycTier {
    /// **N18** : Stable label for metrics + UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tier0 => "tier_0",
            Self::Tier1 => "tier_1",
            Self::Tier2 => "tier_2",
            Self::Tier3 => "tier_3",
        }
    }

    /// **N18** : True if this tier may publish to the production catalog.
    /// Per architecture G2 + I9.
    pub fn permits_production_catalog(self) -> bool {
        self >= Self::Tier2
    }

    /// **N18** : True if this tier permits pseudonymous identity.
    /// Per architecture ADR-8 + I9.
    pub fn permits_pseudonymous(self) -> bool {
        self == Self::Tier0
    }

    /// **N18** : Required cooldown duration before promote to Stable
    /// (architecture G5 + ADR-4).
    pub fn cooldown_hours(self) -> u32 {
        match self {
            Self::Tier0 | Self::Tier1 => 0, // sandbox / community: no Stable rollout
            Self::Tier2 => 24,
            Self::Tier3 => 1, // with 2-of-3 EFE sig
        }
    }
}

impl Default for KycTier {
    fn default() -> Self {
        Self::Tier0
    }
}

// ─────────────────────────────────────────────────────────────────────
//  PublisherStatus
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : publisher lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum PublisherStatus {
    /// Active — publishes + dispatches allowed.
    Active,
    /// Sanctions check stale (> 26 h since last) — publishes blocked,
    /// dispatches continue (defense in depth).
    SanctionsCheckStale,
    /// Suspended — publishes + dispatches blocked.
    Suspended {
        reason: String,
        suspended_at: SystemTime,
    },
    /// Revoked permanently.
    Revoked,
}

impl PublisherStatus {
    /// **N18** : Stable label for metrics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::SanctionsCheckStale => "sanctions_check_stale",
            Self::Suspended { .. } => "suspended",
            Self::Revoked => "revoked",
        }
    }

    /// **N18** : True if the publisher may publish a new agent version.
    pub fn can_publish(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// **N18** : True if existing dispatch should be blocked.
    pub fn dispatch_blocked(&self) -> bool {
        matches!(self, Self::Suspended { .. } | Self::Revoked)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  SignatureAlgorithm
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : signature algorithm taxonomy.
///
/// Forward-compatible : new variants added later don't invalidate
/// existing signatures (architecture I13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureAlgorithm {
    /// Ed25519 (RFC 8032).  Sprint 10 default.
    Ed25519,
}

impl SignatureAlgorithm {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ed25519 => "ed25519",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  PublisherKey + HistoricalKey
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : a publisher signing key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublisherKey {
    pub key_id: KeyId,
    pub algorithm: SignatureAlgorithm,
    /// PEM-encoded public key.
    pub public_key_pem: String,
    pub valid_from: SystemTime,
    /// `None` for the currently-active key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<SystemTime>,
}

impl PublisherKey {
    /// **N18** : True if the key was active at `at`.
    pub fn was_active_at(&self, at: SystemTime) -> bool {
        at >= self.valid_from && self.valid_until.is_none_or(|until| at < until)
    }

    /// **N18** : True if this is the current key (no upper bound).
    pub fn is_current(&self) -> bool {
        self.valid_until.is_none()
    }
}

/// **Sprint 10 — N18** : alias retained for clarity ; historical keys
/// are stored alongside the current one (I7).
pub type HistoricalKey = PublisherKey;

// ─────────────────────────────────────────────────────────────────────
//  PublisherSignature
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : signature attached to a published version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublisherSignature {
    pub publisher_id: PublisherId,
    pub key_id: KeyId,
    pub algorithm: SignatureAlgorithm,
    /// Raw signature bytes.  Ed25519 is 64 bytes.
    pub signature: Vec<u8>,
    pub signed_at: SystemTime,
    /// SHA-256 hash of the canonical body the signature covers
    /// (architecture annex A2).
    pub canonical_hash: Sha256Hash,
}

// ─────────────────────────────────────────────────────────────────────
//  Iso3166
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : ISO-3166 alpha-2 country code newtype.
///
/// Carrier for jurisdiction tagging (P12).  Validation : 2 ASCII
/// uppercase letters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Iso3166(pub String);

impl Iso3166 {
    /// **N18** : Validating constructor.
    pub fn new(code: impl Into<String>) -> Result<Self, IsoError> {
        let s = code.into();
        if s.len() != 2 || !s.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(IsoError::InvalidFormat(s));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IsoError {
    #[error("invalid ISO-3166 alpha-2 code : {0:?}")]
    InvalidFormat(String),
}

// ─────────────────────────────────────────────────────────────────────
//  RevocationCertificate + RevocationReason
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : non-repudiable revocation record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationCertificate {
    pub revoked_at: SystemTime,
    /// **Architecture G15** : when the publisher believes the key was
    /// compromised.  May be earlier than `revoked_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspected_compromise_at: Option<SystemTime>,
    pub reason: RevocationReason,
    pub revoking_operator_id: Uuid,
    /// EFE platform signature over `(publisher_id || reason ||
    /// revoked_at || suspected_compromise_at)`.
    pub signature: Vec<u8>,
}

/// **Sprint 10 — N18** : revocation reason taxonomy (annex A3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RevocationReason {
    KeyCompromise,
    SanctionsViolation,
    PolicyViolation,
    PublisherRequest,
    Operator { rationale: String },
}

impl RevocationReason {
    pub fn label(&self) -> &'static str {
        match self {
            Self::KeyCompromise => "key_compromise",
            Self::SanctionsViolation => "sanctions_violation",
            Self::PolicyViolation => "policy_violation",
            Self::PublisherRequest => "publisher_request",
            Self::Operator { .. } => "operator",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  CoPublishingPolicy
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : N-of-M signature threshold for co-published
/// agents.  Format frozen Sprint 10 ; runtime enforcement Sprint 11.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoPublishingPolicy {
    pub signers: Vec<PublisherId>,
    pub threshold: u8,
}

impl CoPublishingPolicy {
    /// **N18** : Validate that threshold ≤ signers count.
    pub fn validate(&self) -> Result<(), CoPublishingError> {
        if self.signers.is_empty() {
            return Err(CoPublishingError::EmptySigners);
        }
        if self.threshold == 0 {
            return Err(CoPublishingError::ZeroThreshold);
        }
        if usize::from(self.threshold) > self.signers.len() {
            return Err(CoPublishingError::ThresholdExceedsSigners {
                threshold: self.threshold,
                signers: self.signers.len() as u32,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoPublishingError {
    #[error("co-publishing policy must include at least one signer")]
    EmptySigners,
    #[error("co-publishing threshold must be ≥ 1")]
    ZeroThreshold,
    #[error("threshold {threshold} exceeds signer count {signers}")]
    ThresholdExceedsSigners { threshold: u8, signers: u32 },
}

// ─────────────────────────────────────────────────────────────────────
//  OperatorSignature
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : counter-signature by an EFE operator for
/// tier promotion (G14) and Tier 3 fast-path approvals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorSignature {
    pub operator_id: Uuid,
    pub signed_at: SystemTime,
    pub ticket_id: String,
    pub signature: Vec<u8>,
}

// ─────────────────────────────────────────────────────────────────────
//  PublisherIdentity
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : full publisher registry record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublisherIdentity {
    pub publisher_id: PublisherId,
    pub display_name: String,
    pub kyc_tier: KycTier,
    pub status: PublisherStatus,
    pub jurisdiction: Vec<Iso3166>,
    pub registered_at: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sanctions_check_at: Option<SystemTime>,
    pub active_key: PublisherKey,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub historical_keys: Vec<HistoricalKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kyc_dossier_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation: Option<RevocationCertificate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forks_of: Option<String>, // agent ref string ; full NanoAgentRef in Sprint 11
}

impl PublisherIdentity {
    /// **N18** : Look up the key active at a given timestamp.
    /// Used by replay (N16) + provenance verification (N17).
    pub fn key_active_at(&self, at: SystemTime) -> Option<&PublisherKey> {
        if self.active_key.was_active_at(at) {
            return Some(&self.active_key);
        }
        self.historical_keys.iter().find(|k| k.was_active_at(at))
    }

    /// **N18** : True if the publisher may currently publish.
    pub fn can_publish(&self) -> bool {
        self.status.can_publish()
    }

    /// **N18** : True if dispatch on this publisher's agents must be
    /// blocked.
    pub fn dispatch_blocked(&self) -> bool {
        self.status.dispatch_blocked()
    }

    /// **N18** : True if the publisher's sanctions check is fresh.
    /// Architecture I6 + F4.
    pub fn sanctions_check_fresh(&self, now: SystemTime, max_age_hours: u64) -> bool {
        match self.last_sanctions_check_at {
            None => false,
            Some(last) => now
                .duration_since(last)
                .map(|d| d.as_secs() < max_age_hours * 3_600)
                .unwrap_or(false),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  NewPublisherCandidate
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : registration submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPublisherCandidate {
    pub display_name: String,
    pub target_tier: KycTier,
    pub jurisdiction: Vec<Iso3166>,
    pub initial_public_key_pem: String,
    pub algorithm: SignatureAlgorithm,
    pub pseudonymous: bool,
    /// Provider-specific KYC dossier (opaque).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kyc_dossier_reference: Option<String>,
}

impl NewPublisherCandidate {
    /// **N18** : Validate the candidate.
    pub fn validate(&self) -> Result<(), CandidateError> {
        if self.display_name.trim().is_empty() {
            return Err(CandidateError::EmptyDisplayName);
        }
        if self.initial_public_key_pem.trim().is_empty() {
            return Err(CandidateError::EmptyPublicKey);
        }
        if self.pseudonymous && !self.target_tier.permits_pseudonymous() {
            return Err(CandidateError::PseudonymousAboveTier0);
        }
        if !self.pseudonymous && self.kyc_dossier_reference.is_none()
            && self.target_tier >= KycTier::Tier2
        {
            return Err(CandidateError::Tier2RequiresKycDossier);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CandidateError {
    #[error("display name must not be empty")]
    EmptyDisplayName,
    #[error("initial public key must not be empty")]
    EmptyPublicKey,
    #[error("pseudonymous publishers are restricted to Tier 0 (I9)")]
    PseudonymousAboveTier0,
    #[error("Tier 2+ registration requires a KYC dossier reference")]
    Tier2RequiresKycDossier,
}

// ─────────────────────────────────────────────────────────────────────
//  PublisherError (top-level)
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : closed taxonomy of publisher operations
/// errors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublisherError {
    /// Publisher not found.
    #[error("publisher {publisher_id} not found")]
    NotFound { publisher_id: PublisherId },

    /// Display name collision within tier (I10).
    #[error("display name {name:?} already in use at tier {tier:?}")]
    DisplayNameCollision { name: String, tier: KycTier },

    /// Candidate validation failed.
    #[error("candidate invalid : {reason}")]
    InvalidCandidate { reason: String },

    /// KYC verification failed at publish time (F3).
    #[error("KYC verification failed : {reason}")]
    KycFailed { reason: String },

    /// Sanctions check failed (P5 + F4).
    #[error("sanctions match : {list}")]
    SanctionsMatch { list: String },

    /// Sanctions check stale (> 26 h ago).
    #[error("sanctions check stale (> 26 h ago)")]
    SanctionsCheckStale,

    /// Signature verification failed (F2).
    #[error("signature verification failed : {reason}")]
    SignatureMismatch { reason: String },

    /// Pre-rotation signature proof failed (F8).
    #[error("key rotation proof signature failed")]
    RotationProofFailed,

    /// Publisher revoked.
    #[error("publisher revoked")]
    PublisherRevoked,

    /// Publisher suspended.
    #[error("publisher suspended : {reason}")]
    PublisherSuspended { reason: String },

    /// Tier insufficient for target catalog.
    #[error("tier {actual:?} insufficient — required {required:?}")]
    TierInsufficient { actual: KycTier, required: KycTier },

    /// Cooldown not elapsed before promote (G5 + F14).
    #[error("cooldown not elapsed : {remaining_seconds} s remaining")]
    CooldownNotElapsed { remaining_seconds: u64 },

    /// Co-publishing threshold not met (F5).
    #[error("co-publishing threshold not met : got {got}, need {need}")]
    CoPublishThresholdNotMet { got: u8, need: u8 },

    /// Co-publishing canonical hash mismatch (F15).
    #[error("co-signer hash mismatch")]
    CoSignerHashMismatch,

    /// Backend / store fault.
    #[error("backend error : {message}")]
    Backend { message: String },

    /// Internal error.
    #[error("internal : {message}")]
    Internal { message: String },
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ─── Sha256Hash ────────────────────────────────────────────────

    #[test]
    fn sha256_hash_round_trip_hex() {
        let bytes = [0xAB; 32];
        let h = Sha256Hash::from_bytes(bytes);
        let hex = h.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(Sha256Hash::parse_hex(&hex), Some(h));
    }

    #[test]
    fn sha256_hash_rejects_malformed() {
        assert!(Sha256Hash::parse_hex("too-short").is_none());
        assert!(Sha256Hash::parse_hex(&"z".repeat(64)).is_none());
    }

    #[test]
    fn sha256_hash_zero_helpers() {
        let z = Sha256Hash::ZERO;
        assert!(z.is_zero());
        assert!(!Sha256Hash::from_bytes([1u8; 32]).is_zero());
    }

    #[test]
    fn sha256_hash_serde() {
        let h = Sha256Hash::from_bytes([0x42; 32]);
        let s = serde_json::to_string(&h).unwrap();
        assert_eq!(s, format!("\"{}\"", "42".repeat(32)));
        let h2: Sha256Hash = serde_json::from_str(&s).unwrap();
        assert_eq!(h, h2);
    }

    // ─── PublisherId / KeyId ──────────────────────────────────────

    #[test]
    fn publisher_id_new_unique() {
        let a = PublisherId::new();
        let b = PublisherId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn publisher_id_efe_reserved_is_nil_uuid() {
        let p = PublisherId::efe_reserved();
        assert!(p.is_efe_reserved());
        assert_eq!(p.as_uuid(), Uuid::nil());
    }

    #[test]
    fn publisher_id_round_trip_uuid() {
        let u = Uuid::new_v4();
        let p = PublisherId::from_uuid(u);
        assert_eq!(p.as_uuid(), u);
    }

    #[test]
    fn key_id_round_trip_uuid() {
        let u = Uuid::new_v4();
        let k = KeyId::from_uuid(u);
        assert_eq!(k.as_uuid(), u);
    }

    // ─── KycTier ──────────────────────────────────────────────────

    #[test]
    fn kyc_tier_ordering() {
        assert!(KycTier::Tier0 < KycTier::Tier1);
        assert!(KycTier::Tier1 < KycTier::Tier2);
        assert!(KycTier::Tier2 < KycTier::Tier3);
    }

    #[test]
    fn kyc_tier_permits_production_catalog() {
        assert!(!KycTier::Tier0.permits_production_catalog());
        assert!(!KycTier::Tier1.permits_production_catalog());
        assert!(KycTier::Tier2.permits_production_catalog());
        assert!(KycTier::Tier3.permits_production_catalog());
    }

    #[test]
    fn kyc_tier_permits_pseudonymous_only_tier_0() {
        assert!(KycTier::Tier0.permits_pseudonymous());
        assert!(!KycTier::Tier1.permits_pseudonymous());
        assert!(!KycTier::Tier2.permits_pseudonymous());
        assert!(!KycTier::Tier3.permits_pseudonymous());
    }

    #[test]
    fn kyc_tier_cooldown_hours_per_arch_g5() {
        assert_eq!(KycTier::Tier0.cooldown_hours(), 0);
        assert_eq!(KycTier::Tier1.cooldown_hours(), 0);
        assert_eq!(KycTier::Tier2.cooldown_hours(), 24);
        assert_eq!(KycTier::Tier3.cooldown_hours(), 1);
    }

    #[test]
    fn kyc_tier_label_stable() {
        assert_eq!(KycTier::Tier0.label(), "tier_0");
        assert_eq!(KycTier::Tier2.label(), "tier_2");
    }

    #[test]
    fn kyc_tier_serde() {
        let cases = [KycTier::Tier0, KycTier::Tier1, KycTier::Tier2, KycTier::Tier3];
        for t in cases {
            let s = serde_json::to_string(&t).unwrap();
            let t2: KycTier = serde_json::from_str(&s).unwrap();
            assert_eq!(t, t2);
        }
    }

    /// **Post-audit fix** : serde wire format must match `label()`
    /// exactly so downstream consumers can JOIN JSON payloads
    /// against Prometheus labels.  Pre-fix : serde emitted `tier2`
    /// (no underscore) while `label()` emitted `tier_2`.
    #[test]
    fn kyc_tier_serde_matches_label() {
        for t in [KycTier::Tier0, KycTier::Tier1, KycTier::Tier2, KycTier::Tier3] {
            let json = serde_json::to_string(&t).unwrap();
            // Strip surrounding quotes.
            let unquoted = json.trim_matches('"');
            assert_eq!(unquoted, t.label(), "serde wire ({json}) must match label() ({})", t.label());
        }
    }

    // ─── PublisherStatus ──────────────────────────────────────────

    #[test]
    fn publisher_status_can_publish_only_active() {
        assert!(PublisherStatus::Active.can_publish());
        assert!(!PublisherStatus::SanctionsCheckStale.can_publish());
        assert!(!PublisherStatus::Suspended {
            reason: "x".into(),
            suspended_at: SystemTime::now()
        }
        .can_publish());
        assert!(!PublisherStatus::Revoked.can_publish());
    }

    #[test]
    fn publisher_status_dispatch_blocked() {
        assert!(!PublisherStatus::Active.dispatch_blocked());
        assert!(!PublisherStatus::SanctionsCheckStale.dispatch_blocked());
        assert!(PublisherStatus::Suspended {
            reason: "x".into(),
            suspended_at: SystemTime::now()
        }
        .dispatch_blocked());
        assert!(PublisherStatus::Revoked.dispatch_blocked());
    }

    #[test]
    fn publisher_status_label_stable() {
        assert_eq!(PublisherStatus::Active.label(), "active");
        assert_eq!(PublisherStatus::Revoked.label(), "revoked");
    }

    // ─── PublisherKey ─────────────────────────────────────────────

    fn sample_key(valid_until: Option<SystemTime>) -> PublisherKey {
        PublisherKey {
            key_id: KeyId::new(),
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: "-----BEGIN PUBLIC KEY-----\nMCo...".into(),
            valid_from: SystemTime::now() - Duration::from_secs(60),
            valid_until,
        }
    }

    #[test]
    fn publisher_key_is_current_when_no_upper_bound() {
        let k = sample_key(None);
        assert!(k.is_current());
        assert!(!sample_key(Some(SystemTime::now())).is_current());
    }

    #[test]
    fn publisher_key_was_active_at_inclusive_lower() {
        let k = sample_key(None);
        assert!(k.was_active_at(k.valid_from));
    }

    #[test]
    fn publisher_key_was_active_at_exclusive_upper() {
        let now = SystemTime::now();
        let k = PublisherKey {
            key_id: KeyId::new(),
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: "x".into(),
            valid_from: now - Duration::from_secs(3600),
            valid_until: Some(now),
        };
        assert!(k.was_active_at(now - Duration::from_secs(1)));
        assert!(!k.was_active_at(now));
        assert!(!k.was_active_at(now + Duration::from_secs(1)));
    }

    // ─── Iso3166 ──────────────────────────────────────────────────

    #[test]
    fn iso3166_valid_two_letter_uppercase() {
        assert!(Iso3166::new("US").is_ok());
        assert!(Iso3166::new("FR").is_ok());
        assert!(Iso3166::new("DE").is_ok());
    }

    #[test]
    fn iso3166_rejects_lowercase() {
        assert!(Iso3166::new("us").is_err());
    }

    #[test]
    fn iso3166_rejects_wrong_length() {
        assert!(Iso3166::new("USA").is_err());
        assert!(Iso3166::new("U").is_err());
        assert!(Iso3166::new("").is_err());
    }

    #[test]
    fn iso3166_serde() {
        let c = Iso3166::new("FR").unwrap();
        let s = serde_json::to_string(&c).unwrap();
        let c2: Iso3166 = serde_json::from_str(&s).unwrap();
        assert_eq!(c, c2);
    }

    // ─── CoPublishingPolicy ───────────────────────────────────────

    #[test]
    fn co_publishing_policy_validate_basic() {
        let p = CoPublishingPolicy {
            signers: vec![PublisherId::new(), PublisherId::new()],
            threshold: 2,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn co_publishing_policy_rejects_zero_threshold() {
        let p = CoPublishingPolicy {
            signers: vec![PublisherId::new()],
            threshold: 0,
        };
        assert!(matches!(p.validate(), Err(CoPublishingError::ZeroThreshold)));
    }

    #[test]
    fn co_publishing_policy_rejects_empty_signers() {
        let p = CoPublishingPolicy {
            signers: vec![],
            threshold: 1,
        };
        assert!(matches!(p.validate(), Err(CoPublishingError::EmptySigners)));
    }

    #[test]
    fn co_publishing_policy_rejects_threshold_above_signers() {
        let p = CoPublishingPolicy {
            signers: vec![PublisherId::new(), PublisherId::new()],
            threshold: 3,
        };
        assert!(matches!(
            p.validate(),
            Err(CoPublishingError::ThresholdExceedsSigners { .. })
        ));
    }

    // ─── NewPublisherCandidate ────────────────────────────────────

    fn candidate(tier: KycTier, pseudonymous: bool) -> NewPublisherCandidate {
        NewPublisherCandidate {
            display_name: "ACME".into(),
            target_tier: tier,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: "-----BEGIN PUBLIC KEY-----\n...".into(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous,
            kyc_dossier_reference: if tier >= KycTier::Tier2 && !pseudonymous {
                Some("kyc-1".into())
            } else {
                None
            },
        }
    }

    #[test]
    fn candidate_valid_tier_0_pseudonymous() {
        let c = candidate(KycTier::Tier0, true);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn candidate_valid_tier_2_with_kyc() {
        let c = candidate(KycTier::Tier2, false);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn candidate_rejects_pseudonymous_above_tier_0() {
        let c = candidate(KycTier::Tier1, true);
        assert!(matches!(
            c.validate(),
            Err(CandidateError::PseudonymousAboveTier0)
        ));
    }

    #[test]
    fn candidate_rejects_tier_2_without_kyc_dossier() {
        let mut c = candidate(KycTier::Tier2, false);
        c.kyc_dossier_reference = None;
        assert!(matches!(
            c.validate(),
            Err(CandidateError::Tier2RequiresKycDossier)
        ));
    }

    #[test]
    fn candidate_rejects_empty_display_name() {
        let mut c = candidate(KycTier::Tier0, false);
        c.display_name = "".into();
        assert!(matches!(c.validate(), Err(CandidateError::EmptyDisplayName)));
    }

    #[test]
    fn candidate_rejects_empty_public_key() {
        let mut c = candidate(KycTier::Tier0, false);
        c.initial_public_key_pem = "".into();
        assert!(matches!(c.validate(), Err(CandidateError::EmptyPublicKey)));
    }

    // ─── RevocationReason ─────────────────────────────────────────

    #[test]
    fn revocation_reason_serde() {
        let cases = [
            RevocationReason::KeyCompromise,
            RevocationReason::SanctionsViolation,
            RevocationReason::PolicyViolation,
            RevocationReason::PublisherRequest,
            RevocationReason::Operator {
                rationale: "x".into(),
            },
        ];
        for r in cases {
            let json = serde_json::to_string(&r).unwrap();
            let r2: RevocationReason = serde_json::from_str(&json).unwrap();
            assert_eq!(r, r2);
            assert!(!r.label().is_empty());
        }
    }

    // ─── PublisherIdentity ────────────────────────────────────────

    fn sample_identity() -> PublisherIdentity {
        PublisherIdentity {
            publisher_id: PublisherId::new(),
            display_name: "ACME Corp".into(),
            kyc_tier: KycTier::Tier2,
            status: PublisherStatus::Active,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            registered_at: SystemTime::now(),
            last_sanctions_check_at: Some(SystemTime::now()),
            active_key: sample_key(None),
            historical_keys: vec![],
            kyc_dossier_reference: Some("kyc-1".into()),
            revocation: None,
            forks_of: None,
        }
    }

    #[test]
    fn identity_can_publish_when_active() {
        let mut i = sample_identity();
        assert!(i.can_publish());
        i.status = PublisherStatus::Revoked;
        assert!(!i.can_publish());
    }

    #[test]
    fn identity_dispatch_blocked_on_revoked() {
        let mut i = sample_identity();
        assert!(!i.dispatch_blocked());
        i.status = PublisherStatus::Revoked;
        assert!(i.dispatch_blocked());
    }

    #[test]
    fn identity_sanctions_fresh_within_window() {
        let mut i = sample_identity();
        i.last_sanctions_check_at = Some(SystemTime::now() - Duration::from_secs(60));
        assert!(i.sanctions_check_fresh(SystemTime::now(), 24));
    }

    #[test]
    fn identity_sanctions_stale_beyond_window() {
        let mut i = sample_identity();
        i.last_sanctions_check_at =
            Some(SystemTime::now() - Duration::from_secs(48 * 3600));
        assert!(!i.sanctions_check_fresh(SystemTime::now(), 24));
    }

    #[test]
    fn identity_sanctions_fresh_returns_false_when_never_checked() {
        let mut i = sample_identity();
        i.last_sanctions_check_at = None;
        assert!(!i.sanctions_check_fresh(SystemTime::now(), 24));
    }

    #[test]
    fn identity_key_active_at_returns_current() {
        let i = sample_identity();
        let now = SystemTime::now();
        let k = i.key_active_at(now).unwrap();
        assert_eq!(k.key_id, i.active_key.key_id);
    }

    #[test]
    fn identity_key_active_at_falls_back_to_historical() {
        let now = SystemTime::now();
        let mut i = sample_identity();
        // Active key valid from 10 minutes ago.
        i.active_key.valid_from = now - Duration::from_secs(600);
        // Historical key was valid before that.
        let historical = PublisherKey {
            key_id: KeyId::new(),
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: "old".into(),
            valid_from: now - Duration::from_secs(7200),
            valid_until: Some(now - Duration::from_secs(600)),
        };
        let historical_id = historical.key_id;
        i.historical_keys = vec![historical];
        let k = i.key_active_at(now - Duration::from_secs(3600)).unwrap();
        assert_eq!(k.key_id, historical_id);
    }

    #[test]
    fn identity_serde_round_trip() {
        let i = sample_identity();
        let json = serde_json::to_string(&i).unwrap();
        let i2: PublisherIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(i, i2);
    }

    // ─── PublisherError ───────────────────────────────────────────

    #[test]
    fn publisher_error_serde_variants() {
        let cases: Vec<PublisherError> = vec![
            PublisherError::NotFound {
                publisher_id: PublisherId::new(),
            },
            PublisherError::DisplayNameCollision {
                name: "x".into(),
                tier: KycTier::Tier2,
            },
            PublisherError::PublisherRevoked,
            PublisherError::CooldownNotElapsed {
                remaining_seconds: 3600,
            },
            PublisherError::TierInsufficient {
                actual: KycTier::Tier0,
                required: KycTier::Tier2,
            },
            PublisherError::CoPublishThresholdNotMet { got: 1, need: 2 },
            PublisherError::Internal {
                message: "bug".into(),
            },
        ];
        for e in cases {
            let json = serde_json::to_string(&e).unwrap();
            let e2: PublisherError = serde_json::from_str(&json).unwrap();
            assert_eq!(e.to_string(), e2.to_string());
        }
    }

    #[test]
    fn publisher_error_display_messages_non_empty() {
        let e = PublisherError::PublisherSuspended {
            reason: "x".into(),
        };
        assert!(!e.to_string().is_empty());
    }

    // ─── PublisherSignature ───────────────────────────────────────

    #[test]
    fn publisher_signature_serde() {
        let sig = PublisherSignature {
            publisher_id: PublisherId::new(),
            key_id: KeyId::new(),
            algorithm: SignatureAlgorithm::Ed25519,
            signature: vec![0u8; 64],
            signed_at: SystemTime::now(),
            canonical_hash: Sha256Hash::from_bytes([1; 32]),
        };
        let json = serde_json::to_string(&sig).unwrap();
        let sig2: PublisherSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(sig, sig2);
    }
}
