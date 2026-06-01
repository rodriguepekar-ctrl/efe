//! **Sprint 7 — N9** : Merkle epoch layer on top of the Sprint 6
//! linear HMAC chain.
//!
//! # Public surface
//!
//! - [`tree`] : pure RFC 6962 Merkle algorithm + [`tree::InclusionProof`].
//! - [`epoch`] : [`epoch::EpochRoot`] + [`epoch::EpochSealer`]
//!   orchestrator + audit-grade verifier.
//! - [`store`] : [`store::EpochRootStore`] trait + in-memory impl.
//! - [`notary`] : [`notary::Notary`] trait for external attestation
//!   (Sigstore Rekor in Phase 4).
//!
//! # Architectural picture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │  Sprint 6 :  linear HMAC chain (one row per trace)           │
//! │                                                              │
//! │  trace[i] = { payload_hash, hmac, chain_parent_id ... }      │
//! │             ↓                                                │
//! │  trace[i+1] linked via chain_parent_id + HMAC                │
//! └──────────────────────────────────────────────────────────────┘
//!         │
//!         │  every N traces (default 1 024)
//!         ▼
//! ┌──────────────────────────────────────────────────────────────┐
//! │  Sprint 7 — N9 :  Merkle epoch layer                         │
//! │                                                              │
//! │  EpochRoot { epoch_id, root_hash, previous_epoch_root, ... } │
//! │       ↓                                                      │
//! │  next EpochRoot links via previous_epoch_root                │
//! │       ↓                                                      │
//! │  (optionally)  Notary receipts attest each EpochRoot         │
//! │                externally (Sigstore Rekor, Phase 4)          │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Audit verification : two levels (three with notary)
//!
//! Pre-Sprint-7 : O(N) HMAC walk.  Sprint 7 — N9 :
//!
//! - **Level 1 — Per-epoch tree integrity** : O(N / epoch_size) +
//!   O(epoch_size) per epoch.  Each epoch's tree is independently
//!   rebuildable from the underlying traces.
//! - **Level 2 — Cross-epoch chain** : O(N / epoch_size).  Walk
//!   epoch roots, verify each linkback.
//! - **Level 3 (optional) — External notarisation** : O(1) per epoch
//!   to verify the [`notary::NotaryReceipt`] against a public
//!   transparency log.  Defeats a compromised EFE operator.
//! - **Audit-by-trace** : O(log N) — given a trace_id, build the
//!   [`tree::InclusionProof`] for its epoch (depth ~ 10 for
//!   epoch_size = 1024), verify against the notarised root.
//!
//! # Sprint 7 limits
//!
//! - In-memory implementations only (Sprint 7 ships seam, Phase 4
//!   wires Postgres + Sigstore).
//! - Sealing is operator-triggered (no background cron) ; Phase 4
//!   adds a tokio scheduler.
//! - `seal_pending_epochs` is O(N) per call because it re-pulls the
//!   full chain ; Phase 4 paginates from `start_after`.

pub mod epoch;
pub mod notary;
pub mod store;
pub mod tree;

pub use epoch::{
    canonical_root_body, verify_trace_in_epoch, EpochRoot, EpochSealer, EpochSealerConfig,
    SealError, SealerKeySource,
};
pub use notary::{commitment_hash, InMemoryNotary, Notary, NotaryError, NotaryReceipt};
pub use store::{EpochRootStore, InMemoryEpochRootStore};
pub use tree::{
    build_root, build_tree, leaf_hash, node_hash, verify_inclusion, InclusionProof,
    MerkleError, MerkleTree, LEAF_DOMAIN, NODE_DOMAIN,
};
