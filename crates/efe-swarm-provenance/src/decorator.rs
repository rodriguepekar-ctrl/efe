// SPDX-License-Identifier: UNLICENSED
//! # `decorator` — **seam B (actif)** : capture entrée→sortie **sans réécrire les agents**.
//!
//! [`ProvenancedAgent<A>`] enveloppe n'importe quel agent producteur de
//! `efe-swarm-topology` ([`Generator`], [`Stage`], [`Decomposer`], [`Solver`],
//! [`Merger`], [`Judge`]) et **est lui-même** cet agent (mêmes traits) : c'est un
//! *drop-in*. À chaque appel, il hashe l'entrée et la sortie (BLAKE3 canonique) et
//! transmet un [`CallCapture`] au [`ProvenanceRecorder`]. **L'agent interne ignore
//! tout de la provenance.**
//!
//! Le **rôle** est porté par le trait appelé (l'impl `Generator` enregistre
//! `AgentRole::Generator`, etc.) : un même type peut donc être instrumenté sous
//! plusieurs rôles s'il implémente plusieurs traits.
//!
//! ```ignore
//! let rec = Arc::new(ProvenanceRecorder::new(swarm, tenant));
//! let gen: Arc<dyn Generator> = Arc::new(
//!     ProvenancedAgent::mode_a(MyGenerator, rec.clone(), AgentId::new("generator")?)
//! );
//! // `gen` se branche TEL QUEL dans une topologie ; chaque generate() est capté.
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use efe_nano_sdk::{AgentId, ProvenanceId};
use efe_swarm_topology::{Decomposer, Generator, Judge, Merger, Solver, Stage};
use serde_json::{json, Value};

use crate::record::{AgentKind, AgentRole};
use crate::recorder::{CallCapture, ProvenanceRecorder};

/// Extracteur du `ProvenanceId` N17 d'un agent **Mode C** (le pont vers N17). Reçoit la
/// **sortie** de l'appel ; renvoie l'id si l'agent l'expose (embarqué dans la sortie, ou
/// via une cellule partagée que la closure capture). Voir `bridge`.
type LlmLink = Arc<dyn Fn(&Value) -> Option<ProvenanceId> + Send + Sync>;

/// Décorateur de provenance : `inner` + un `recorder` partagé + l'identité de l'agent.
/// Le rôle n'est PAS stocké — il est déterminé par le trait appelé.
pub struct ProvenancedAgent<A> {
    inner: A,
    recorder: Arc<ProvenanceRecorder>,
    agent_id: AgentId,
    kind: AgentKind,
    llm_link: Option<LlmLink>,
}

impl<A> ProvenancedAgent<A> {
    /// Décore `inner` avec une nature explicite (Mode A / Mode C).
    pub fn new(inner: A, recorder: Arc<ProvenanceRecorder>, agent_id: AgentId, kind: AgentKind) -> Self {
        Self { inner, recorder, agent_id, kind, llm_link: None }
    }
    /// Agent **Mode A** (Rust pur — le gros de l'essaim, jusqu'ici muet).
    pub fn mode_a(inner: A, recorder: Arc<ProvenanceRecorder>, agent_id: AgentId) -> Self {
        Self::new(inner, recorder, agent_id, AgentKind::ModeA)
    }
    /// Agent **Mode C** (LLM) — pensez à attacher le pont N17 via [`Self::with_llm_link`].
    pub fn mode_c(inner: A, recorder: Arc<ProvenanceRecorder>, agent_id: AgentId) -> Self {
        Self::new(inner, recorder, agent_id, AgentKind::ModeC)
    }

    /// **Pont N17** : fournit un extracteur du `ProvenanceId` N17 (appliqué à la sortie,
    /// pour les agents **Mode C**). Sans incidence pour un agent Mode A.
    pub fn with_llm_link<F>(mut self, extractor: F) -> Self
    where
        F: Fn(&Value) -> Option<ProvenanceId> + Send + Sync + 'static,
    {
        self.llm_link = Some(Arc::new(extractor));
        self
    }

    pub fn inner(&self) -> &A {
        &self.inner
    }

    /// Raccourci interne : enregistre un appel (entrée/sortie déjà sous forme Value).
    fn record(&self, role: AgentRole, variant: Option<u64>, input: &Value, output: &Value) {
        let mut cap = CallCapture::new(self.agent_id.clone(), role, self.kind, input, output);
        if let Some(v) = variant {
            cap = cap.with_variant(v);
        }
        // pont N17 : un agent Mode C surface son ProvenanceId via l'extracteur.
        if self.kind == AgentKind::ModeC {
            if let Some(f) = &self.llm_link {
                if let Some(id) = f(output) {
                    cap = cap.with_llm_provenance(id);
                }
            }
        }
        self.recorder.capture(cap);
    }
}

#[async_trait]
impl<G: Generator> Generator for ProvenancedAgent<G> {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let out = self.inner.generate(task, variant).await;
        self.record(AgentRole::Generator, Some(variant as u64), task, &out);
        out
    }
}

#[async_trait]
impl<S: Stage> Stage for ProvenancedAgent<S> {
    async fn process(&self, input: &Value) -> Vec<Value> {
        let out = self.inner.process(input).await;
        self.record(AgentRole::Stage, None, input, &json!(out));
        out
    }
}

#[async_trait]
impl<D: Decomposer> Decomposer for ProvenancedAgent<D> {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        let out = self.inner.decompose(task).await;
        self.record(AgentRole::Decomposer, None, task, &json!(out));
        out
    }
}

#[async_trait]
impl<S: Solver> Solver for ProvenancedAgent<S> {
    async fn solve(&self, subtask: &Value) -> Value {
        let out = self.inner.solve(subtask).await;
        self.record(AgentRole::Solver, None, subtask, &out);
        out
    }
}

#[async_trait]
impl<M: Merger> Merger for ProvenancedAgent<M> {
    async fn merge(&self, task: &Value, results: Vec<Value>) -> Value {
        // l'entrée d'un merge = (tâche, résultats) ; on hashe les deux.
        let input = json!({ "task": task, "results": results });
        let out = self.inner.merge(task, results).await;
        self.record(AgentRole::Merger, None, &input, &out);
        out
    }
}

#[async_trait]
impl<J: Judge> Judge for ProvenancedAgent<J> {
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
        let scores = self.inner.assess(task, candidates).await;
        let input = json!({ "task": task, "candidates": candidates });
        self.record(AgentRole::Judge, None, &input, &json!(scores));
        scores
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::AgentRole;
    use crate::HmacKeySource;
    use efe_nano_sdk::{ProvenanceId, SwarmId, TenantId};
    use uuid::Uuid;

    fn rec() -> Arc<ProvenanceRecorder> {
        Arc::new(ProvenanceRecorder::new(SwarmId::from_uuid(Uuid::from_u128(0x2222)), TenantId::system()))
    }

    // ── agents de test (Mode A purs) ────────────────────────────────────────
    struct EchoGen;
    #[async_trait]
    impl Generator for EchoGen {
        async fn generate(&self, task: &Value, variant: usize) -> Value {
            json!({ "value": task.get("v").cloned().unwrap_or(Value::Null), "variant": variant })
        }
    }

    struct OneJudge;
    #[async_trait]
    impl Judge for OneJudge {
        async fn assess(&self, _task: &Value, candidates: &[Value]) -> Vec<f64> {
            candidates.iter().map(|_| 1.0).collect()
        }
        fn name(&self) -> &str {
            "one_judge"
        }
    }

    struct WordsStage;
    #[async_trait]
    impl Stage for WordsStage {
        async fn process(&self, input: &Value) -> Vec<Value> {
            input
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .split_whitespace()
                .map(|w| json!({ "text": w }))
                .collect()
        }
    }

    #[tokio::test]
    async fn generator_decorator_is_transparent_and_captures() {
        let r = rec();
        let gen = ProvenancedAgent::mode_a(EchoGen, r.clone(), AgentId::new("generator").unwrap());
        // se comporte EXACTEMENT comme l'agent interne (drop-in).
        let out = gen.generate(&json!({ "v": "tech" }), 3).await;
        assert_eq!(out, json!({ "value": "tech", "variant": 3 }));
        // …et a capté l'appel.
        assert_eq!(r.captured_calls(), 1);
        let sealed = r.seal_with_key(HmacKeySource::test_key().derive(TenantId::system()).unwrap());
        assert_eq!(sealed.records[0].agent_role, AgentRole::Generator);
        assert_eq!(sealed.records[0].agent_kind, AgentKind::ModeA);
        assert_eq!(sealed.records[0].variant, Some(3));
    }

    #[tokio::test]
    async fn judge_decorator_forwards_name_and_captures() {
        let r = rec();
        let judge = ProvenancedAgent::mode_c(OneJudge, r.clone(), AgentId::new("judge_000").unwrap());
        assert_eq!(judge.name(), "one_judge", "name() transparent");
        let scores = judge.assess(&json!({}), &[json!("a"), json!("b")]).await;
        assert_eq!(scores, vec![1.0, 1.0]);
        assert_eq!(r.captured_calls(), 1);
        let sealed = r.seal_with_key(HmacKeySource::test_key().derive(TenantId::system()).unwrap());
        assert_eq!(sealed.records[0].agent_role, AgentRole::Judge);
        assert_eq!(sealed.records[0].agent_kind, AgentKind::ModeC);
    }

    #[tokio::test]
    async fn stage_decorator_captures_fanout() {
        let r = rec();
        let stage = ProvenancedAgent::mode_a(WordsStage, r.clone(), AgentId::new("splitter").unwrap());
        let out = stage.process(&json!({ "text": "a b c" })).await;
        assert_eq!(out.len(), 3, "transparent : 1→N");
        assert_eq!(r.captured_calls(), 1);
    }

    /// Générateur Mode C de test : embarque son ProvenanceId N17 dans la sortie.
    struct LlmGen(String);
    #[async_trait]
    impl Generator for LlmGen {
        async fn generate(&self, _task: &Value, _variant: usize) -> Value {
            json!({ "value": "x", "_prov": self.0 })
        }
    }

    #[tokio::test]
    async fn mode_c_with_llm_link_attaches_provenance_id() {
        // PONT N17 : l'agent Mode C expose son ProvenanceId ; le décorateur l'attache.
        let r = rec();
        let prov = ProvenanceId::new();
        let gen = ProvenancedAgent::mode_c(
            LlmGen(prov.as_uuid().to_string()),
            r.clone(),
            AgentId::new("judge_llm").unwrap(),
        )
        .with_llm_link(|out| {
            out.get("_prov")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok())
                .map(ProvenanceId::from_uuid)
        });
        let _ = gen.generate(&json!({}), 0).await;
        let sealed = r.seal_with_key(HmacKeySource::test_key().derive(TenantId::system()).unwrap());
        assert_eq!(sealed.records[0].llm_provenance_id, Some(prov), "lien N17 posé");
        assert_eq!(sealed.records[0].agent_kind, AgentKind::ModeC);
    }

    #[tokio::test]
    async fn mode_a_ignores_llm_link() {
        // un décorateur Mode A ne pose jamais de lien, même si un extracteur est fourni.
        let r = rec();
        let gen = ProvenancedAgent::mode_a(LlmGen("xx".into()), r.clone(), AgentId::new("generator").unwrap())
            .with_llm_link(|_| Some(ProvenanceId::new()));
        let _ = gen.generate(&json!({}), 0).await;
        let sealed = r.seal_with_key(HmacKeySource::test_key().derive(TenantId::system()).unwrap());
        assert_eq!(sealed.records[0].llm_provenance_id, None, "Mode A : aucun lien");
    }

    #[tokio::test]
    async fn decorator_works_behind_dyn_trait_object() {
        let r = rec();
        // l'usage réel : se branche comme Arc<dyn Generator> dans une topologie.
        let gen: Arc<dyn Generator> = Arc::new(ProvenancedAgent::mode_a(
            EchoGen,
            r.clone(),
            AgentId::new("generator").unwrap(),
        ));
        for v in 0..5 {
            let _ = gen.generate(&json!({ "v": "x" }), v).await;
        }
        assert_eq!(r.captured_calls(), 5);
    }
}
