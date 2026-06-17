// SPDX-License-Identifier: UNLICENSED
//! # `record` — l'**atome** de la provenance par-agent.
//!
//! Un [`AgentCallRecord`] répond, par construction, à la question du moat :
//!
//! > **« quel agent a dit quoi, quand, à partir de quoi ? »**
//!
//! | question        | champ(s)                                               |
//! |-----------------|--------------------------------------------------------|
//! | **qui**         | `agent_id`, `agent_role`, `agent_kind`                 |
//! | **quoi**        | `output_hash`, `writes`                                |
//! | **quand**       | `seq` (ordre LOGIQUE déterministe) ; `wall_at` advisory|
//! | **à partir de** | `input_hash`, `reads`, `causal_parents`                |
//! | **re-prouvable**| `payload_hash` (SHA-256 canonique) + `hmac` (chaîne)   |
//!
//! ## Pourquoi un atome séparé du chaînage
//! L'atome **se hashe seul** (`payload_hash` = SHA-256 du corps canonique). Sa
//! **position** dans la chaîne (`seq`, `chain_parent_hmac`, `hmac`) est posée par la
//! `SwarmChain` (Inc.2). Cette séparation rend l'atome **testable et content-
//! addressable** dès Inc.1, indépendamment de toute clé.
//!
//! ## Ce qui entre (ou non) dans `payload_hash`
//! Le corps canonique commet **tout le sens** (qui/quoi/à-partir-de) **sauf** les
//! champs *advisory*/*position* : `wall_at` (horloge murale, non déterministe — cf.
//! ADR-SWARM-007), `seq`, `chain_parent_hmac`, `hmac`, `payload_hash` lui-même.
//! L'ordre est commis par la chaîne HMAC (Inc.2), pas par le corps — exactement
//! comme `efe-nano` commet la causalité (`parent_trace_ids`) dans le corps mais
//! l'ordre via `chain_parent_id` dans le HMAC.

use std::time::SystemTime;

use efe_nano_sdk::{AgentId, Blake3Hash, ProvenanceId, SwarmId, TenantId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use uuid::Uuid;

use crate::canon;
use crate::hash;

// ─────────────────────────────────────────────────────────────────────────────
//  Rôle & nature de l'agent
// ─────────────────────────────────────────────────────────────────────────────

/// Le rôle de l'agent dans une topologie (la frontière d'appel instrumentée).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Generator,
    Verifier,
    Stage,
    Decomposer,
    Merger,
    Solver,
    Judge,
    /// **ADR-019 (MEM-2)** : la **lecture de mémoire inter-runs** (recall). Pas un agent au
    /// sens d'une topologie, mais un appel tracé : son record (un `MemoryRead`) ancre dans la
    /// `provenance_root` ce qu'un run a LU de la mémoire (famille, version, `content_hash`) —
    /// I-MEM2. Variant additif : aucun record déjà persisté ne le porte (rétro-compatible).
    Memory,
}

impl AgentRole {
    /// Forme canonique snake_case (utilisée dans le corps hashé).
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentRole::Generator => "generator",
            AgentRole::Verifier => "verifier",
            AgentRole::Stage => "stage",
            AgentRole::Decomposer => "decomposer",
            AgentRole::Merger => "merger",
            AgentRole::Solver => "solver",
            AgentRole::Judge => "judge",
            AgentRole::Memory => "memory",
        }
    }
}

/// La nature de l'agent : Mode A (Rust pur, gratuit) ou Mode C (LLM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    /// Agent déterministe pur Rust — **le gros de l'essaim, aujourd'hui muet**.
    ModeA,
    /// Agent LLM (lié à un `LlmProvenanceRecord` via `llm_provenance_id`).
    ModeC,
}

impl AgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::ModeA => "mode_a",
            AgentKind::ModeC => "mode_c",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  CellRef — référence à une cellule du Blackboard (pointer @ version)
// ─────────────────────────────────────────────────────────────────────────────

/// Une cellule lue/écrite : `pointer` + `version` (la version est l'ordre atomique
/// déterministe du substrat). L'ordre des champs (`pointer` puis `version`) fixe le
/// tri `Ord` utilisé pour la canonicalisation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CellRef {
    pub pointer: String,
    pub version: u64,
}

impl CellRef {
    pub fn new(pointer: impl Into<String>, version: u64) -> Self {
        Self { pointer: pointer.into(), version }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Vue interne sur le CONTENU (partagée Draft ↔ Record) → corps canonique unique
// ─────────────────────────────────────────────────────────────────────────────

/// Vue empruntée sur les champs **de contenu** (hors position/advisory). Un seul
/// constructeur de corps canonique : Draft et Record le partagent ⇒ `payload_hash`
/// d'un Draft et `recompute_payload_hash` d'un Record sont **garantis identiques**.
struct ContentRef<'a> {
    record_id: Uuid,
    swarm_id: SwarmId,
    tenant_id: TenantId,
    agent_id: &'a AgentId,
    agent_role: AgentRole,
    agent_kind: AgentKind,
    input_hash: Blake3Hash,
    reads: &'a [CellRef],
    output_hash: Blake3Hash,
    writes: &'a [CellRef],
    variant: Option<u64>,
    causal_parents: &'a [Uuid],
    llm_provenance_id: Option<ProvenanceId>,
}

impl ContentRef<'_> {
    /// Construit le corps canonique (clés triées par `canon`, hashs en hex,
    /// `reads`/`writes`/`causal_parents` **triés** pour neutraliser l'ordre de
    /// capture). C'est l'unique source de vérité du format.
    fn body_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("agent_id".into(), Value::String(self.agent_id.as_str().to_string()));
        m.insert("agent_kind".into(), Value::String(self.agent_kind.as_str().into()));
        m.insert("agent_role".into(), Value::String(self.agent_role.as_str().into()));
        // parents triés (par forme string, convention efe-nano) → ordre neutre.
        let mut parents: Vec<String> = self.causal_parents.iter().map(|u| u.to_string()).collect();
        parents.sort();
        m.insert("causal_parents".into(), Value::Array(parents.into_iter().map(Value::String).collect()));
        m.insert("input_hash".into(), Value::String(self.input_hash.to_hex()));
        m.insert(
            "llm_provenance_id".into(),
            match self.llm_provenance_id {
                Some(p) => Value::String(p.as_uuid().to_string()),
                None => Value::Null,
            },
        );
        m.insert("output_hash".into(), Value::String(self.output_hash.to_hex()));
        m.insert("reads".into(), cellrefs_value(self.reads));
        m.insert("record_id".into(), Value::String(self.record_id.to_string()));
        m.insert("swarm_id".into(), Value::String(self.swarm_id.as_uuid().to_string()));
        m.insert("tenant_id".into(), Value::String(self.tenant_id.as_uuid().to_string()));
        m.insert(
            "variant".into(),
            match self.variant {
                Some(v) => Value::Number(Number::from(v)),
                None => Value::Null,
            },
        );
        m.insert("writes".into(), cellrefs_value(self.writes));
        Value::Object(m)
    }

    fn payload_hash(&self) -> [u8; 32] {
        hash::payload_hash_of(&self.body_value())
    }
}

/// Tableau canonique de `CellRef` : **trié** (pointer, version) puis objets
/// `{pointer, version}`. Le tri rend le hash insensible à l'ordre de capture.
fn cellrefs_value(refs: &[CellRef]) -> Value {
    let mut sorted = refs.to_vec();
    sorted.sort();
    Value::Array(
        sorted
            .iter()
            .map(|c| {
                let mut m = Map::new();
                m.insert("pointer".into(), Value::String(c.pointer.clone()));
                m.insert("version".into(), Value::Number(Number::from(c.version)));
                Value::Object(m)
            })
            .collect(),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
//  AgentCallDraft — l'intention d'enregistrer (contenu sans position de chaîne)
// ─────────────────────────────────────────────────────────────────────────────

/// Brouillon d'un appel d'agent : tout le **contenu** (qui/quoi/à-partir-de) **avant**
/// d'être placé dans la chaîne. Se hashe seul (`payload_hash`). Scellé par
/// [`AgentCallDraft::seal`] (appelé par la `SwarmChain` en Inc.2, après calcul du HMAC).
#[derive(Debug, Clone)]
pub struct AgentCallDraft {
    pub record_id: Uuid,
    pub swarm_id: SwarmId,
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub agent_role: AgentRole,
    pub agent_kind: AgentKind,
    pub input_hash: Blake3Hash,
    pub reads: Vec<CellRef>,
    pub output_hash: Blake3Hash,
    pub writes: Vec<CellRef>,
    pub variant: Option<u64>,
    pub causal_parents: Vec<Uuid>,
    pub llm_provenance_id: Option<ProvenanceId>,
    pub wall_at: SystemTime,
}

impl AgentCallDraft {
    /// Brouillon minimal : `record_id` frais, `wall_at = now`, listes vides.
    #[allow(clippy::too_many_arguments)] // constructeur explicite de l'atome de provenance (champs nommés voulus)
    pub fn new(
        swarm_id: SwarmId,
        tenant_id: TenantId,
        agent_id: AgentId,
        agent_role: AgentRole,
        agent_kind: AgentKind,
        input_hash: Blake3Hash,
        output_hash: Blake3Hash,
    ) -> Self {
        Self {
            record_id: Uuid::new_v4(),
            swarm_id,
            tenant_id,
            agent_id,
            agent_role,
            agent_kind,
            input_hash,
            reads: Vec::new(),
            output_hash,
            writes: Vec::new(),
            variant: None,
            causal_parents: Vec::new(),
            llm_provenance_id: None,
            wall_at: SystemTime::now(),
        }
    }

    /// Brouillon à partir des **valeurs** d'entrée/sortie (hashées en BLAKE3 canonique).
    /// C'est la voie qu'empruntera le décorateur (Inc.3).
    #[allow(clippy::too_many_arguments)] // constructeur explicite de l'atome de provenance (champs nommés voulus)
    pub fn from_io(
        swarm_id: SwarmId,
        tenant_id: TenantId,
        agent_id: AgentId,
        agent_role: AgentRole,
        agent_kind: AgentKind,
        input: &Value,
        output: &Value,
    ) -> Self {
        Self::new(
            swarm_id,
            tenant_id,
            agent_id,
            agent_role,
            agent_kind,
            hash::blake3_value(input),
            hash::blake3_value(output),
        )
    }

    // ── setters chaînables ──────────────────────────────────────────────────
    pub fn with_record_id(mut self, id: Uuid) -> Self {
        self.record_id = id;
        self
    }
    pub fn with_variant(mut self, v: u64) -> Self {
        self.variant = Some(v);
        self
    }
    pub fn read(mut self, pointer: impl Into<String>, version: u64) -> Self {
        self.reads.push(CellRef::new(pointer, version));
        self
    }
    pub fn wrote(mut self, pointer: impl Into<String>, version: u64) -> Self {
        self.writes.push(CellRef::new(pointer, version));
        self
    }
    pub fn caused_by(mut self, record_id: Uuid) -> Self {
        self.causal_parents.push(record_id);
        self
    }
    pub fn with_llm_provenance(mut self, id: ProvenanceId) -> Self {
        self.llm_provenance_id = Some(id);
        self
    }
    pub fn with_wall_at(mut self, t: SystemTime) -> Self {
        self.wall_at = t;
        self
    }

    fn content(&self) -> ContentRef<'_> {
        ContentRef {
            record_id: self.record_id,
            swarm_id: self.swarm_id,
            tenant_id: self.tenant_id,
            agent_id: &self.agent_id,
            agent_role: self.agent_role,
            agent_kind: self.agent_kind,
            input_hash: self.input_hash,
            reads: &self.reads,
            output_hash: self.output_hash,
            writes: &self.writes,
            variant: self.variant,
            causal_parents: &self.causal_parents,
            llm_provenance_id: self.llm_provenance_id,
        }
    }

    /// Octets du corps canonique (l'entrée du SHA-256).
    pub fn canonical_body(&self) -> Vec<u8> {
        canon::canonical_bytes(&self.content().body_value())
    }

    /// `payload_hash` = SHA-256 du corps canonique. **Le livrable d'Inc.1.**
    pub fn payload_hash(&self) -> [u8; 32] {
        self.content().payload_hash()
    }

    /// Scelle le brouillon en enregistrement, à une **position de chaîne** donnée.
    /// `hmac` est calculé par la `SwarmChain` (Inc.2) à partir de `payload_hash()` ;
    /// ici on le **pose** simplement (l'atome ne connaît pas la clé).
    pub fn seal(self, seq: u64, chain_parent_hmac: Option<[u8; 32]>, hmac: [u8; 32]) -> AgentCallRecord {
        let payload_hash = self.payload_hash();
        AgentCallRecord {
            record_id: self.record_id,
            swarm_id: self.swarm_id,
            tenant_id: self.tenant_id,
            agent_id: self.agent_id,
            agent_role: self.agent_role,
            agent_kind: self.agent_kind,
            input_hash: self.input_hash,
            reads: self.reads,
            output_hash: self.output_hash,
            writes: self.writes,
            variant: self.variant,
            causal_parents: self.causal_parents,
            llm_provenance_id: self.llm_provenance_id,
            wall_at: self.wall_at,
            seq,
            chain_parent_hmac,
            payload_hash,
            hmac,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  AgentCallRecord — l'enregistrement scellé (hashé + position de chaîne)
// ─────────────────────────────────────────────────────────────────────────────

/// L'enregistrement scellé d'un appel d'agent. Sérialisable, vérifiable seul
/// (`verify_payload_hash`) et — via `hmac`/`chain_parent_hmac` — chaînable (Inc.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCallRecord {
    // ── contenu (commis dans payload_hash) ──────────────────────────────────
    pub record_id: Uuid,
    pub swarm_id: SwarmId,
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub agent_role: AgentRole,
    pub agent_kind: AgentKind,
    pub input_hash: Blake3Hash,
    pub reads: Vec<CellRef>,
    pub output_hash: Blake3Hash,
    pub writes: Vec<CellRef>,
    pub variant: Option<u64>,
    pub causal_parents: Vec<Uuid>,
    pub llm_provenance_id: Option<ProvenanceId>,
    // ── advisory (HORS payload_hash — ADR-SWARM-007) ────────────────────────
    pub wall_at: SystemTime,
    // ── position de chaîne (posée par SwarmChain, Inc.2) ────────────────────
    pub seq: u64,
    #[serde(with = "sd_hex32_opt", default)]
    pub chain_parent_hmac: Option<[u8; 32]>,
    // ── intégrité ───────────────────────────────────────────────────────────
    #[serde(with = "sd_hex32")]
    pub payload_hash: [u8; 32],
    #[serde(with = "sd_hex32")]
    pub hmac: [u8; 32],
}

impl AgentCallRecord {
    fn content(&self) -> ContentRef<'_> {
        ContentRef {
            record_id: self.record_id,
            swarm_id: self.swarm_id,
            tenant_id: self.tenant_id,
            agent_id: &self.agent_id,
            agent_role: self.agent_role,
            agent_kind: self.agent_kind,
            input_hash: self.input_hash,
            reads: &self.reads,
            output_hash: self.output_hash,
            writes: &self.writes,
            variant: self.variant,
            causal_parents: &self.causal_parents,
            llm_provenance_id: self.llm_provenance_id,
        }
    }

    /// Recalcule le `payload_hash` depuis le contenu (la base de la vérification).
    pub fn recompute_payload_hash(&self) -> [u8; 32] {
        self.content().payload_hash()
    }

    /// **Invariant de l'atome** : le `payload_hash` stocké correspond-il au contenu ?
    /// `false` ⇒ le contenu a été altéré après scellement (sortie/entrée substituée,
    /// `reads` forgés…). C'est la première ligne de la tamper-evidence (Inc.4 l'étend
    /// avec la vérification de la chaîne HMAC).
    pub fn verify_payload_hash(&self) -> bool {
        self.recompute_payload_hash() == self.payload_hash
    }

    /// Octets du corps canonique (debug / cross-vérification).
    pub fn canonical_body(&self) -> Vec<u8> {
        canon::canonical_bytes(&self.content().body_value())
    }

    /// Reconstruit le **brouillon** (contenu sans position de chaîne) — utilisé par le
    /// rejeu déterministe pour re-sceller dans l'ordre canonique (ADR-SWARM-007).
    pub fn to_draft(&self) -> AgentCallDraft {
        AgentCallDraft {
            record_id: self.record_id,
            swarm_id: self.swarm_id,
            tenant_id: self.tenant_id,
            agent_id: self.agent_id.clone(),
            agent_role: self.agent_role,
            agent_kind: self.agent_kind,
            input_hash: self.input_hash,
            reads: self.reads.clone(),
            output_hash: self.output_hash,
            writes: self.writes.clone(),
            variant: self.variant,
            causal_parents: self.causal_parents.clone(),
            llm_provenance_id: self.llm_provenance_id,
            wall_at: self.wall_at,
        }
    }

    pub fn payload_hex(&self) -> String {
        hex32(&self.payload_hash)
    }
    pub fn hmac_hex(&self) -> String {
        hex32(&self.hmac)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  hex 32 octets (encodage + serde) — sans dépendance `hex`
// ─────────────────────────────────────────────────────────────────────────────

/// Encode 32 octets en hex minuscule (64 caractères).
pub fn hex32(b: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for &x in b.iter() {
        s.push(HEX[(x >> 4) as usize] as char);
        s.push(HEX[(x & 0x0f) as usize] as char);
    }
    s
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let hi = nibble(b[i * 2])?;
        let lo = nibble(b[i * 2 + 1])?;
        *byte = (hi << 4) | lo;
    }
    Some(out)
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

mod sd_hex32 {
    use super::{hex32, unhex32};
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(b: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex32(b))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        unhex32(&s).ok_or_else(|| serde::de::Error::custom("hex32 malformé"))
    }
}

mod sd_hex32_opt {
    use super::{hex32, unhex32};
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(b: &Option<[u8; 32]>, s: S) -> Result<S::Ok, S::Error> {
        match b {
            Some(b) => s.serialize_some(&hex32(b)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 32]>, D::Error> {
        let o: Option<String> = Option::deserialize(d)?;
        match o {
            Some(s) => unhex32(&s).map(Some).ok_or_else(|| serde::de::Error::custom("hex32 malformé")),
            None => Ok(None),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests (oracles)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn bh(byte: u8) -> Blake3Hash {
        Blake3Hash::from_bytes([byte; 32])
    }

    /// Brouillon DÉTERMINISTE (record_id fixe) pour des hashs reproductibles.
    fn draft() -> AgentCallDraft {
        AgentCallDraft::new(
            SwarmId::from_uuid(Uuid::from_u128(0x2222)),
            TenantId::system(),
            AgentId::new("generator").unwrap(),
            AgentRole::Generator,
            AgentKind::ModeA,
            bh(1),
            bh(2),
        )
        .with_record_id(Uuid::from_u128(0x1111))
        .with_wall_at(UNIX_EPOCH)
    }

    // ── rôles / nature ──────────────────────────────────────────────────────
    #[test]
    fn role_as_str_and_serde_snake_case() {
        assert_eq!(AgentRole::Judge.as_str(), "judge");
        assert_eq!(serde_json::to_string(&AgentRole::Decomposer).unwrap(), "\"decomposer\"");
        let r: AgentRole = serde_json::from_str("\"merger\"").unwrap();
        assert_eq!(r, AgentRole::Merger);
    }

    #[test]
    fn kind_as_str_and_serde_snake_case() {
        assert_eq!(AgentKind::ModeA.as_str(), "mode_a");
        assert_eq!(serde_json::to_string(&AgentKind::ModeC).unwrap(), "\"mode_c\"");
        let k: AgentKind = serde_json::from_str("\"mode_a\"").unwrap();
        assert_eq!(k, AgentKind::ModeA);
    }

    // ── CellRef ─────────────────────────────────────────────────────────────
    #[test]
    fn cellref_sorts_by_pointer_then_version() {
        let mut v = vec![CellRef::new("/b", 1), CellRef::new("/a", 2), CellRef::new("/a", 1)];
        v.sort();
        assert_eq!(v, vec![CellRef::new("/a", 1), CellRef::new("/a", 2), CellRef::new("/b", 1)]);
    }

    // ── payload_hash : déterminisme + exclusions ────────────────────────────
    #[test]
    fn payload_hash_is_deterministic() {
        assert_eq!(draft().payload_hash(), draft().payload_hash());
    }

    #[test]
    fn payload_hash_excludes_wall_at() {
        let a = draft().with_wall_at(UNIX_EPOCH);
        let b = draft().with_wall_at(UNIX_EPOCH + Duration::from_secs(86_400));
        assert_eq!(a.payload_hash(), b.payload_hash(), "wall_at est advisory (ADR-007)");
    }

    // ── payload_hash : sensibilité au sens (qui/quoi/à-partir-de) ────────────
    #[test]
    fn payload_hash_changes_with_agent_id() {
        let a = draft();
        let mut b = draft();
        b.agent_id = AgentId::new("judge_001").unwrap();
        assert_ne!(a.payload_hash(), b.payload_hash());
    }

    #[test]
    fn payload_hash_changes_with_output_hash() {
        let a = draft();
        let mut b = draft();
        b.output_hash = bh(9);
        assert_ne!(a.payload_hash(), b.payload_hash(), "QUOI a changé");
    }

    #[test]
    fn payload_hash_changes_with_input_hash() {
        let a = draft();
        let mut b = draft();
        b.input_hash = bh(9);
        assert_ne!(a.payload_hash(), b.payload_hash(), "À PARTIR DE QUOI a changé");
    }

    #[test]
    fn payload_hash_changes_with_variant() {
        assert_ne!(draft().payload_hash(), draft().with_variant(3).payload_hash());
    }

    #[test]
    fn payload_hash_changes_when_read_added() {
        assert_ne!(draft().payload_hash(), draft().read("/cand/0", 1).payload_hash());
    }

    #[test]
    fn payload_hash_invariant_under_read_reorder() {
        let a = draft().read("/a", 1).read("/b", 2);
        let b = draft().read("/b", 2).read("/a", 1);
        assert_eq!(a.payload_hash(), b.payload_hash(), "reads triés → ordre de capture neutre");
    }

    #[test]
    fn payload_hash_invariant_under_parent_reorder_but_sensitive_to_set() {
        let p1 = Uuid::from_u128(0xA);
        let p2 = Uuid::from_u128(0xB);
        let a = draft().caused_by(p1).caused_by(p2);
        let b = draft().caused_by(p2).caused_by(p1);
        assert_eq!(a.payload_hash(), b.payload_hash(), "parents triés → ordre neutre");
        let c = draft().caused_by(p1);
        assert_ne!(a.payload_hash(), c.payload_hash(), "mais l'ENSEMBLE compte");
    }

    #[test]
    fn payload_hash_changes_with_llm_provenance_presence() {
        let a = draft();
        let b = draft().with_llm_provenance(ProvenanceId::from_uuid(Uuid::from_u128(7)));
        assert_ne!(a.payload_hash(), b.payload_hash(), "pont Mode C présent vs absent");
    }

    // ── seal + verify ───────────────────────────────────────────────────────
    #[test]
    fn seal_preserves_payload_hash_and_verifies() {
        let d = draft();
        let ph = d.payload_hash();
        let rec = d.seal(0, None, [9u8; 32]);
        assert_eq!(rec.payload_hash, ph, "seal n'altère pas le payload_hash");
        assert!(rec.verify_payload_hash(), "atome intègre");
    }

    #[test]
    fn tampering_breaks_verification() {
        let mut rec = draft().seal(0, None, [9u8; 32]);
        // un attaquant substitue la sortie après coup…
        rec.output_hash = bh(99);
        assert!(!rec.verify_payload_hash(), "substitution de sortie détectée");
    }

    // ── serde round-trip (hex wire form) ────────────────────────────────────
    #[test]
    fn record_serde_round_trip_is_lossless() {
        let rec = draft()
            .read("/cand/0", 3)
            .wrote("/ballot/0", 4)
            .with_variant(2)
            .with_llm_provenance(ProvenanceId::from_uuid(Uuid::from_u128(0x99)))
            .seal(5, Some([7u8; 32]), [8u8; 32]);
        let s = serde_json::to_string(&rec).unwrap();
        // hashs en hex (pas en tableau de nombres).
        assert!(s.contains(&hex32(&rec.payload_hash)));
        assert!(s.contains("\"seq\":5"));
        let back: AgentCallRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(back, rec, "round-trip sans perte");
        assert!(back.verify_payload_hash());
    }

    #[test]
    fn hex32_round_trips() {
        assert_eq!(hex32(&[0u8; 32]).len(), 64);
        assert_eq!(hex32(&[0xabu8; 32]), "ab".repeat(32));
        assert_eq!(unhex32(&hex32(&[0x5au8; 32])), Some([0x5au8; 32]));
        assert_eq!(unhex32("xyz"), None);
    }

    // ── smoke « qui a dit quoi, quand, à partir de quoi » ────────────────────
    #[test]
    fn atom_answers_the_moat_question() {
        // Un juge Mode C lit /cand/0@3 et /cand/1@3, produit /ballot/2@4, à partir
        // d'un générateur (record p0), lié à un appel LLM.
        let p0 = Uuid::from_u128(0xF00D);
        let rec = AgentCallDraft::from_io(
            SwarmId::from_uuid(Uuid::from_u128(0x2222)),
            TenantId::system(),
            AgentId::new("judge_002").unwrap(),
            AgentRole::Judge,
            AgentKind::ModeC,
            &serde_json::json!({ "candidates": ["a", "b"] }),
            &serde_json::json!({ "scores": [1.0, 0.0] }),
        )
        .read("/cand/0", 3)
        .read("/cand/1", 3)
        .wrote("/ballot/2", 4)
        .caused_by(p0)
        .with_llm_provenance(ProvenanceId::from_uuid(Uuid::from_u128(0xBEEF)))
        .seal(7, Some([1u8; 32]), [2u8; 32]);

        assert_eq!(rec.agent_id.as_str(), "judge_002"); // QUI
        assert_eq!(rec.agent_role, AgentRole::Judge);
        assert_eq!(rec.writes, vec![CellRef::new("/ballot/2", 4)]); // QUOI
        assert_eq!(rec.seq, 7); // QUAND (logique)
        assert_eq!(rec.reads.len(), 2); // À PARTIR DE QUOI
        assert_eq!(rec.causal_parents, vec![p0]);
        assert!(rec.llm_provenance_id.is_some(), "pont Mode C");
        assert!(rec.verify_payload_hash(), "re-prouvable");
    }
}
