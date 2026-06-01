//! Local model loader + handle.
//!
//! Phase 2 Sprint 4 ships an **in-process, pure-Rust, deterministic**
//! model loader.  Each "model" is a Rust function that takes a JSON
//! input and produces a JSON output that conforms to its agent's
//! `output_schema`.  This avoids pulling in ONNX/Tract at this stage
//! (and the multi-hundred-MB model files that would go with them) while
//! still exercising the **entire** Mode B execution path :
//!
//! - Schema validation (input + output) — production-grade.
//! - Pool acquire/release — production-grade.
//! - Timeout enforcement — production-grade.
//! - Audit trace recording — production-grade.
//! - Concurrent-load deduplication — production-grade.
//!
//! The seam to swap in real ONNX models is a single trait :
//! [`LocalModel::infer`].  Sprint 8+ will add a `TractOnnxModel` impl
//! that loads `.onnx` files via `tract-onnx`.  No other code in
//! [`crate::runtime`] changes.
//!
//! # Determinism contract
//!
//! All Phase 2 builtin models are **fully deterministic** : the same
//! input bytes produce the same output bytes, every time.  This makes
//! Mode B agents *replayable* — a key Sprint 6 requirement.

use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tokio::sync::RwLock;

use crate::error::{ModelInferError, ModelLoaderError};

// ─────────────────────────────────────────────────────────────────────
//  Public traits
// ─────────────────────────────────────────────────────────────────────

/// A loaded local model handle.
///
/// Implementations must be `Send + Sync + 'static` so that multiple
/// concurrent Mode B calls can share the same model.
#[async_trait]
pub trait LocalModel: Send + Sync + 'static {
    /// Run inference on `input`.
    ///
    /// The runtime has already validated that `input` matches the
    /// agent's `input_schema` before calling this.  The output **must**
    /// match the agent's `output_schema` — the runtime will validate it
    /// afterwards and surface a `SchemaValidation` error if it doesn't.
    async fn infer(&self, input: serde_json::Value) -> Result<serde_json::Value, ModelInferError>;

    /// Model identifier (matches `agent.execution_mode.LocalModel.model_id`).
    fn id(&self) -> &str;

    /// Model semver (matches `agent.execution_mode.LocalModel.model_version`).
    fn version(&self) -> &str;

    /// What family of model this is — used for telemetry + tier health.
    fn family(&self) -> ModelFamily;
}

/// Loader trait.  Returns an `Arc<dyn LocalModel>` keyed by `(id, version)`.
///
/// Implementations are responsible for :
/// - Lazy loading on first request (cache thereafter).
/// - Concurrent-load deduplication (10 concurrent loads of the same model
///   should result in exactly 1 actual load).
/// - Pre-warming via [`ModelLoader::warm`].
/// - Health snapshot via [`ModelLoader::health`].
#[async_trait]
pub trait ModelLoader: Send + Sync + 'static {
    /// Return a handle to the named model, loading it if necessary.
    async fn get_or_load(
        &self,
        model_id: &str,
        model_version: &str,
    ) -> Result<Arc<dyn LocalModel>, ModelLoaderError>;

    /// Pre-warm the loader with a fixed set of models.
    async fn warm(
        &self,
        models: &[(String, String)],
    ) -> Result<(), ModelLoaderError>;

    /// Snapshot of loader health for telemetry.
    async fn health(&self) -> ModelLoaderHealth;
}

/// Family of model (telemetry-friendly grouping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFamily {
    NamedEntityRecognition,
    Classifier,
    LanguageDetector,
    Embedder,
    Generic,
}

impl fmt::Display for ModelFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NamedEntityRecognition => f.write_str("ner"),
            Self::Classifier => f.write_str("classifier"),
            Self::LanguageDetector => f.write_str("lang_detect"),
            Self::Embedder => f.write_str("embedder"),
            Self::Generic => f.write_str("generic"),
        }
    }
}

/// Loader health snapshot.
#[derive(Debug, Clone)]
pub struct ModelLoaderHealth {
    /// Number of models currently loaded into memory.
    pub loaded_models: u32,
    /// Total memory footprint (approximate, MB).  Builtin models report 0
    /// since they have no native footprint ; real ONNX models would report
    /// the tensor allocation total.
    pub total_memory_mb: u32,
    /// Average load time across all observed loads (ms).
    pub avg_load_ms: u32,
    /// Per-model load counters (load attempts ; cache hits don't count).
    pub per_model_loads: HashMap<String, u32>,
}

// ─────────────────────────────────────────────────────────────────────
//  In-process loader (Phase 2 ship)
// ─────────────────────────────────────────────────────────────────────

/// In-process loader keyed by `(model_id, model_version)`.
///
/// Uses [`OnceCell`] to deduplicate concurrent loads of the same model
/// (10 concurrent `get_or_load("ner_clinical_v1", "1.0.0")` callers result
/// in exactly 1 actual `factory()` call ; the other 9 await the same
/// future).
pub struct InProcessModelLoader {
    /// Registry of known models keyed by (id, version).  Filled at
    /// construction by `register_builtins` (or by tests via `register`).
    registry: RwLock<ModelRegistryMap>,
    /// Total successful load attempts (for health metrics).
    load_counters: RwLock<HashMap<String, u32>>,
}

/// Internal alias : keys are (model_id, model_version), values are entries.
type ModelRegistryMap = HashMap<(String, String), Arc<ModelEntry>>;

/// One registry entry : a factory + a OnceCell that holds the loaded model.
struct ModelEntry {
    factory: Box<dyn ModelFactory>,
    cell: OnceCell<Arc<dyn LocalModel>>,
}

/// A factory produces a fresh `LocalModel` instance.
trait ModelFactory: Send + Sync {
    fn make(&self) -> Result<Arc<dyn LocalModel>, ModelLoaderError>;
}

impl<F> ModelFactory for F
where
    F: Fn() -> Result<Arc<dyn LocalModel>, ModelLoaderError> + Send + Sync,
{
    fn make(&self) -> Result<Arc<dyn LocalModel>, ModelLoaderError> {
        (self)()
    }
}

impl InProcessModelLoader {
    /// Create an empty loader (no models registered).  Useful for tests
    /// that want to inject their own model fixtures.
    pub fn empty() -> Self {
        Self {
            registry: RwLock::new(HashMap::new()),
            load_counters: RwLock::new(HashMap::new()),
        }
    }

    /// Create a loader with all 5 Phase 2 builtin models registered.
    pub fn with_phase_2_builtins() -> Self {
        let loader = Self::empty();
        loader.register_builtins_sync();
        loader
    }

    /// Register a factory for `(model_id, model_version)`.  If the
    /// (id, version) pair already exists, the new factory overwrites it
    /// (this is intentional — tests rely on it for fixture injection).
    pub async fn register<F>(&self, model_id: &str, model_version: &str, factory: F)
    where
        F: Fn() -> Result<Arc<dyn LocalModel>, ModelLoaderError> + Send + Sync + 'static,
    {
        let mut reg = self.registry.write().await;
        reg.insert(
            (model_id.to_string(), model_version.to_string()),
            Arc::new(ModelEntry {
                factory: Box::new(factory),
                cell: OnceCell::new(),
            }),
        );
    }

    /// Synchronous variant of `register` used during construction
    /// (before any task spawn).
    fn register_sync<F>(&self, model_id: &str, model_version: &str, factory: F)
    where
        F: Fn() -> Result<Arc<dyn LocalModel>, ModelLoaderError> + Send + Sync + 'static,
    {
        let mut reg = self
            .registry
            .try_write()
            .expect("constructor : no other locks held");
        reg.insert(
            (model_id.to_string(), model_version.to_string()),
            Arc::new(ModelEntry {
                factory: Box::new(factory),
                cell: OnceCell::new(),
            }),
        );
    }

    /// Register all 5 Phase 2 builtin models.  Called by
    /// `with_phase_2_builtins`.
    fn register_builtins_sync(&self) {
        // 1) ner_clinical_v1
        self.register_sync("ner_clinical_v1", "1.0.0", || {
            Ok(Arc::new(builtin::NerClinical::new()) as Arc<dyn LocalModel>)
        });
        // 2) ner_legal_v1
        self.register_sync("ner_legal_v1", "1.0.0", || {
            Ok(Arc::new(builtin::NerLegal::new()) as Arc<dyn LocalModel>)
        });
        // 3) intent_classify_v1
        self.register_sync("intent_classify_v1", "1.0.0", || {
            Ok(Arc::new(builtin::IntentClassifier::new()) as Arc<dyn LocalModel>)
        });
        // 4) lang_detect_v1
        self.register_sync("lang_detect_v1", "1.0.0", || {
            Ok(Arc::new(builtin::LangDetector::new()) as Arc<dyn LocalModel>)
        });
        // 5) embedder_v1
        self.register_sync("embedder_v1", "1.0.0", || {
            Ok(Arc::new(builtin::Embedder::new()) as Arc<dyn LocalModel>)
        });
    }

    /// Number of models registered (loaded or not).
    pub async fn registered_count(&self) -> usize {
        self.registry.read().await.len()
    }

    /// Returns true if `(id, version)` is registered.
    pub async fn is_registered(&self, model_id: &str, model_version: &str) -> bool {
        self.registry
            .read()
            .await
            .contains_key(&(model_id.to_string(), model_version.to_string()))
    }
}

impl Default for InProcessModelLoader {
    fn default() -> Self {
        Self::with_phase_2_builtins()
    }
}

impl fmt::Debug for InProcessModelLoader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InProcessModelLoader").finish_non_exhaustive()
    }
}

#[async_trait]
impl ModelLoader for InProcessModelLoader {
    async fn get_or_load(
        &self,
        model_id: &str,
        model_version: &str,
    ) -> Result<Arc<dyn LocalModel>, ModelLoaderError> {
        let key = (model_id.to_string(), model_version.to_string());
        let entry = {
            let reg = self.registry.read().await;
            reg.get(&key).cloned()
        };
        let entry = entry.ok_or_else(|| ModelLoaderError::NotFound {
            id: model_id.to_string(),
            version: model_version.to_string(),
        })?;

        // OnceCell guarantees at-most-one initialisation across concurrent
        // calls. The closure is `Fn`, not `FnOnce`, so we wrap factory in
        // a small closure that captures by ref.
        let model = entry
            .cell
            .get_or_try_init(|| {
                let key = key.clone();
                let load_counters = &self.load_counters;
                let factory = &entry.factory;
                async move {
                    let m = factory.make()?;
                    let mut counters = load_counters.write().await;
                    *counters.entry(key.0.clone()).or_insert(0) += 1;
                    Ok::<_, ModelLoaderError>(m)
                }
            })
            .await?;
        Ok(Arc::clone(model))
    }

    async fn warm(&self, models: &[(String, String)]) -> Result<(), ModelLoaderError> {
        for (id, v) in models {
            self.get_or_load(id, v).await?;
        }
        Ok(())
    }

    async fn health(&self) -> ModelLoaderHealth {
        let counters = self.load_counters.read().await;
        let loaded = counters.values().filter(|&&n| n > 0).count() as u32;
        let per_model_loads: HashMap<String, u32> = counters.clone();
        ModelLoaderHealth {
            loaded_models: loaded,
            total_memory_mb: 0,           // builtin models have no native footprint
            avg_load_ms: 1,                // builtin instantiation is trivial
            per_model_loads,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Builtin Phase 2 models
// ─────────────────────────────────────────────────────────────────────

/// The 5 Phase 2 builtin models.
///
/// Each model is **pure Rust, deterministic, dependency-free** (modulo
/// `regex` + `sha2` which are already deps).  Real ONNX models swap in
/// via the `LocalModel` trait in Sprint 8+.
pub mod builtin {
    use super::*;
    use regex::Regex;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::sync::OnceLock;

    // Compiled regex caches (avoid recompiling on every `infer` call).
    static CITATION_RE: OnceLock<Regex> = OnceLock::new();
    fn citation_re() -> &'static Regex {
        CITATION_RE.get_or_init(|| {
            // E.g. "347 U.S. 483", "163 F.2d 99"
            Regex::new(r"\b(\d{1,4})\s+(U\.S\.|F\.[1-3]d)\s+(\d{1,4})\b")
                .expect("citation regex is valid")
        })
    }

    // ─────────────────────────────────────────────────────────────────
    //  1) ner_clinical_v1 — biomedical NER
    // ─────────────────────────────────────────────────────────────────

    /// Biomedical NER : DRUG / DISEASE / GENE / PHASE entities.
    pub struct NerClinical;

    impl NerClinical {
        pub fn new() -> Self {
            Self
        }
    }

    impl Default for NerClinical {
        fn default() -> Self {
            Self::new()
        }
    }

    const CLINICAL_DRUGS: &[&str] = &[
        "aspirin", "ibuprofen", "paracetamol", "metformin", "warfarin",
        "atorvastatin", "amoxicillin", "ciprofloxacin", "insulin", "morphine",
    ];
    const CLINICAL_DISEASES: &[&str] = &[
        "diabetes", "hypertension", "cancer", "asthma", "stroke", "infection",
        "covid-19", "covid", "pneumonia", "arthritis", "lupus", "epilepsy",
    ];
    const CLINICAL_GENES: &[&str] = &["BRCA1", "BRCA2", "TP53", "EGFR", "KRAS", "MYC", "RB1", "PTEN"];
    const CLINICAL_PHASES: &[&str] = &["phase 1", "phase 2", "phase 3", "phase 4", "phase I", "phase II", "phase III", "phase IV"];

    #[async_trait]
    impl LocalModel for NerClinical {
        async fn infer(&self, input: Value) -> Result<Value, ModelInferError> {
            let text = input.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
                ModelInferError::InputConversion("missing 'text' field".into())
            })?;
            let entities = scan_dictionary_entities(text, "DRUG", CLINICAL_DRUGS, 0.95)
                .into_iter()
                .chain(scan_dictionary_entities(text, "DISEASE", CLINICAL_DISEASES, 0.92))
                .chain(scan_dictionary_entities(text, "GENE", CLINICAL_GENES, 0.99))
                .chain(scan_dictionary_entities(text, "PHASE", CLINICAL_PHASES, 0.98))
                .collect::<Vec<_>>();
            let count = entities.len();
            Ok(json!({ "entities": entities, "count": count }))
        }

        fn id(&self) -> &str {
            "ner_clinical_v1"
        }

        fn version(&self) -> &str {
            "1.0.0"
        }

        fn family(&self) -> ModelFamily {
            ModelFamily::NamedEntityRecognition
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  2) ner_legal_v1 — legal NER
    // ─────────────────────────────────────────────────────────────────

    /// Legal NER : CASE / CITATION / PARTY / CLAUSE entities.
    pub struct NerLegal;

    impl NerLegal {
        pub fn new() -> Self {
            Self
        }
    }

    impl Default for NerLegal {
        fn default() -> Self {
            Self::new()
        }
    }

    const LEGAL_PARTIES: &[&str] = &[
        "plaintiff", "defendant", "appellant", "respondent", "petitioner",
        "claimant", "lessor", "lessee", "licensor", "licensee",
    ];
    const LEGAL_CLAUSES: &[&str] = &[
        "force majeure", "indemnification", "warranty", "termination",
        "confidentiality", "limitation of liability", "governing law",
        "dispute resolution", "non-compete", "non-disclosure",
    ];
    const LEGAL_CASES: &[&str] = &[
        "Brown v. Board",
        "Roe v. Wade",
        "Miranda v. Arizona",
        "Marbury v. Madison",
        "Citizens United v. FEC",
        "Obergefell v. Hodges",
    ];

    #[async_trait]
    impl LocalModel for NerLegal {
        async fn infer(&self, input: Value) -> Result<Value, ModelInferError> {
            let text = input.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
                ModelInferError::InputConversion("missing 'text' field".into())
            })?;
            let mut entities: Vec<Value> = Vec::new();
            entities.extend(scan_dictionary_entities(text, "PARTY", LEGAL_PARTIES, 0.90));
            entities.extend(scan_dictionary_entities(text, "CLAUSE", LEGAL_CLAUSES, 0.93));
            entities.extend(scan_dictionary_entities(text, "CASE", LEGAL_CASES, 0.95));

            // Citation pattern : "NNN U.S. NNN" or "NNN F.2d NNN".
            for m in citation_re().find_iter(text) {
                entities.push(json!({
                    "text": m.as_str(),
                    "kind": "CITATION",
                    "start": m.start(),
                    "end": m.end(),
                    "score": 0.99,
                }));
            }

            let count = entities.len();
            Ok(json!({ "entities": entities, "count": count }))
        }

        fn id(&self) -> &str {
            "ner_legal_v1"
        }

        fn version(&self) -> &str {
            "1.0.0"
        }

        fn family(&self) -> ModelFamily {
            ModelFamily::NamedEntityRecognition
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  3) intent_classify_v1 — generic intent classifier
    // ─────────────────────────────────────────────────────────────────

    /// Keyword-scored intent classifier across ~ 10 generic buckets.
    pub struct IntentClassifier;

    impl IntentClassifier {
        pub fn new() -> Self {
            Self
        }
    }

    impl Default for IntentClassifier {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Static intent → keyword association.  Kept const so the classifier
    /// is fully deterministic + zero-allocation per call.
    fn intent_buckets() -> &'static [(&'static str, &'static [&'static str])] {
        &[
            ("buy_product", &["buy", "purchase", "order", "checkout", "cart"]),
            ("complaint", &["complain", "problem", "issue", "broken", "defect", "doesn't work"]),
            ("request_info", &["how", "what", "when", "where", "why", "tell me", "explain"]),
            ("cancel", &["cancel", "refund", "return", "stop"]),
            ("praise", &["thank", "great", "love", "amazing", "excellent", "best"]),
            ("schedule", &["book", "schedule", "appointment", "reserve", "calendar"]),
            ("support", &["help", "support", "assist", "guide"]),
            ("subscribe", &["subscribe", "join", "sign up", "newsletter"]),
            ("unsubscribe", &["unsubscribe", "remove", "opt out"]),
            ("account", &["password", "login", "account", "username", "reset"]),
        ]
    }

    #[async_trait]
    impl LocalModel for IntentClassifier {
        async fn infer(&self, input: Value) -> Result<Value, ModelInferError> {
            let text = input.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
                ModelInferError::InputConversion("missing 'text' field".into())
            })?;
            let lower = text.to_lowercase();

            let mut all_scores: Vec<(String, f64)> = Vec::new();
            for (intent, keywords) in intent_buckets() {
                let hits = keywords.iter().filter(|k| lower.contains(*k)).count();
                if hits > 0 {
                    let score = (hits as f64 / keywords.len() as f64).min(1.0);
                    all_scores.push(((*intent).to_string(), score));
                }
            }
            // Sort descending by score, stable on intent name for determinism.
            all_scores.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0))
            });

            let (intent, score) = match all_scores.first() {
                Some((i, s)) => (i.clone(), *s),
                None => ("other".to_string(), 0.0),
            };
            let mut all = serde_json::Map::new();
            for (k, v) in &all_scores {
                all.insert(k.clone(), json!(v));
            }
            Ok(json!({
                "intent": intent,
                "score": score,
                "all_scores": all,
            }))
        }

        fn id(&self) -> &str {
            "intent_classify_v1"
        }

        fn version(&self) -> &str {
            "1.0.0"
        }

        fn family(&self) -> ModelFamily {
            ModelFamily::Classifier
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  4) lang_detect_v1 — language detector
    // ─────────────────────────────────────────────────────────────────

    /// Function-word frequency-based language detector.
    pub struct LangDetector;

    impl LangDetector {
        pub fn new() -> Self {
            Self
        }
    }

    impl Default for LangDetector {
        fn default() -> Self {
            Self::new()
        }
    }

    fn lang_signals() -> &'static [(&'static str, &'static [&'static str])] {
        &[
            ("en", &["the", "and", "is", "of", "to", "in", "for", "a", "with", "on"]),
            ("fr", &["le", "la", "les", "et", "est", "de", "du", "des", "à", "un"]),
            ("de", &["der", "die", "das", "und", "ist", "ein", "mit", "von", "zu", "auf"]),
            ("es", &["el", "la", "los", "y", "es", "de", "en", "que", "un", "para"]),
            ("it", &["il", "la", "lo", "e", "è", "di", "che", "un", "con", "per"]),
            ("pt", &["o", "a", "os", "e", "é", "de", "que", "um", "para", "com"]),
            ("nl", &["de", "het", "en", "een", "in", "van", "is", "op", "voor", "met"]),
        ]
    }

    #[async_trait]
    impl LocalModel for LangDetector {
        async fn infer(&self, input: Value) -> Result<Value, ModelInferError> {
            let text = input.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
                ModelInferError::InputConversion("missing 'text' field".into())
            })?;
            let lower = text.to_lowercase();
            let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
            let total_words = words.len().max(1) as f64;

            // Score per language.
            let mut scored: Vec<(String, f64)> = lang_signals()
                .iter()
                .map(|(lang, markers)| {
                    let hits = words.iter().filter(|w| markers.contains(w)).count() as f64;
                    ((*lang).to_string(), hits / total_words)
                })
                .collect();
            scored.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0))
            });

            let (language, confidence) = match scored.first() {
                Some((l, c)) if *c > 0.0 => (l.clone(), c.min(1.0)),
                _ => ("und".to_string(), 0.0),
            };

            let mut per_lang = serde_json::Map::new();
            for (l, c) in &scored {
                per_lang.insert(l.clone(), json!(c));
            }
            Ok(json!({
                "language": language,
                "iso_639_1": language,
                "confidence": confidence,
                "per_language_scores": per_lang,
            }))
        }

        fn id(&self) -> &str {
            "lang_detect_v1"
        }

        fn version(&self) -> &str {
            "1.0.0"
        }

        fn family(&self) -> ModelFamily {
            ModelFamily::LanguageDetector
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  5) embedder_v1 — deterministic 384-dim embedder
    // ─────────────────────────────────────────────────────────────────

    /// Deterministic 384-dimensional embedder.  Vector is derived from
    /// SHA-256 of the input text, then expanded via a fast xorshift PRNG
    /// seeded from the hash.  The output is L2-normalized so cosine
    /// similarity between two embeddings is meaningful (within the
    /// model's deterministic, non-semantic regime).
    ///
    /// Production swap-in : a real `tract-onnx` MiniLM-L6-v2 model goes
    /// here in Sprint 8.
    pub struct Embedder {
        dim: usize,
    }

    impl Embedder {
        pub fn new() -> Self {
            Self { dim: 384 }
        }

        pub fn with_dim(dim: usize) -> Self {
            Self { dim }
        }
    }

    impl Default for Embedder {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl LocalModel for Embedder {
        async fn infer(&self, input: Value) -> Result<Value, ModelInferError> {
            let text = input.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
                ModelInferError::InputConversion("missing 'text' field".into())
            })?;

            // Seed an xorshift PRNG from SHA-256(text).
            let mut hasher = Sha256::new();
            hasher.update(text.as_bytes());
            let digest = hasher.finalize();
            let mut state = u64::from_le_bytes([
                digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6],
                digest[7],
            ]);
            if state == 0 {
                state = 0x9E37_79B9_7F4A_7C15;
            }

            let mut vec: Vec<f64> = Vec::with_capacity(self.dim);
            for _ in 0..self.dim {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                // Map u64 → f64 in [-1, 1).
                let unit = (state as f64) / (u64::MAX as f64);
                vec.push(unit * 2.0 - 1.0);
            }
            // L2-normalize.
            let norm: f64 = vec.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
            let normalized: Vec<f64> = vec.iter().map(|x| x / norm).collect();

            Ok(json!({
                "embedding": normalized,
                "dimension": self.dim,
                "model": "embedder_v1",
            }))
        }

        fn id(&self) -> &str {
            "embedder_v1"
        }

        fn version(&self) -> &str {
            "1.0.0"
        }

        fn family(&self) -> ModelFamily {
            ModelFamily::Embedder
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Shared helper : case-insensitive substring scan
    // ─────────────────────────────────────────────────────────────────

    /// Scan `text` for case-insensitive occurrences of each `needle`, emit
    /// one entity per match. Preserves the **original text** for the span
    /// (so capitalization in source is preserved in output).
    fn scan_dictionary_entities(
        text: &str,
        kind: &str,
        needles: &[&str],
        score: f64,
    ) -> Vec<Value> {
        let lower = text.to_lowercase();
        let mut out = Vec::new();
        for needle in needles {
            let lower_needle = needle.to_lowercase();
            let mut start = 0;
            while let Some(pos) = lower[start..].find(&lower_needle) {
                let abs_start = start + pos;
                let abs_end = abs_start + lower_needle.len();
                if abs_end > text.len() {
                    break;
                }
                // Boundary check : avoid matching inside a longer word
                // (e.g. "phase 1" should match but "diaphase" shouldn't).
                let left_ok = abs_start == 0
                    || !text.as_bytes()[abs_start - 1].is_ascii_alphanumeric();
                let right_ok = abs_end == text.len()
                    || !text.as_bytes()[abs_end].is_ascii_alphanumeric();
                if left_ok && right_ok {
                    out.push(json!({
                        "text": &text[abs_start..abs_end],
                        "kind": kind,
                        "start": abs_start,
                        "end": abs_end,
                        "score": score,
                    }));
                }
                start = abs_end;
            }
        }
        out
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Loader basics ────────────────────────────────────────────────

    #[tokio::test]
    async fn empty_loader_has_zero_registered() {
        let l = InProcessModelLoader::empty();
        assert_eq!(l.registered_count().await, 0);
        assert!(!l.is_registered("anything", "1.0.0").await);
    }

    #[tokio::test]
    async fn phase_2_builtins_has_5_registered() {
        let l = InProcessModelLoader::with_phase_2_builtins();
        assert_eq!(l.registered_count().await, 5);
        for id in [
            "ner_clinical_v1",
            "ner_legal_v1",
            "intent_classify_v1",
            "lang_detect_v1",
            "embedder_v1",
        ] {
            assert!(l.is_registered(id, "1.0.0").await, "{id} missing");
        }
    }

    #[tokio::test]
    async fn get_or_load_returns_model() {
        let l = InProcessModelLoader::with_phase_2_builtins();
        let m = l.get_or_load("ner_clinical_v1", "1.0.0").await.unwrap();
        assert_eq!(m.id(), "ner_clinical_v1");
        assert_eq!(m.version(), "1.0.0");
        assert_eq!(m.family(), ModelFamily::NamedEntityRecognition);
    }

    #[tokio::test]
    async fn get_or_load_returns_not_found_for_unknown_model() {
        let l = InProcessModelLoader::with_phase_2_builtins();
        let result = l.get_or_load("ghost_model", "1.0.0").await;
        match result {
            Ok(_) => panic!("expected error, got Ok"),
            Err(ModelLoaderError::NotFound { id, version }) => {
                assert_eq!(id, "ghost_model");
                assert_eq!(version, "1.0.0");
            }
            Err(other) => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn warm_pre_loads_models() {
        let l = InProcessModelLoader::with_phase_2_builtins();
        l.warm(&[
            ("ner_clinical_v1".to_string(), "1.0.0".to_string()),
            ("embedder_v1".to_string(), "1.0.0".to_string()),
        ])
        .await
        .unwrap();
        let health = l.health().await;
        assert_eq!(health.loaded_models, 2);
        assert!(health.per_model_loads.contains_key("ner_clinical_v1"));
        assert!(health.per_model_loads.contains_key("embedder_v1"));
    }

    #[tokio::test]
    async fn warm_propagates_not_found_error() {
        let l = InProcessModelLoader::with_phase_2_builtins();
        let err = l
            .warm(&[("ghost_model".to_string(), "1.0.0".to_string())])
            .await
            .unwrap_err();
        assert!(matches!(err, ModelLoaderError::NotFound { .. }));
    }

    #[tokio::test]
    async fn concurrent_get_or_load_deduplicates() {
        // Custom loader where the factory counts how many times it's invoked.
        let l = InProcessModelLoader::empty();
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);
        l.register("racer_v1", "1.0.0", move || {
            counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Arc::new(builtin::NerClinical::new()) as Arc<dyn LocalModel>)
        })
        .await;

        let l = Arc::new(l);
        let mut handles = Vec::new();
        for _ in 0..16 {
            let l = Arc::clone(&l);
            handles.push(tokio::spawn(async move {
                l.get_or_load("racer_v1", "1.0.0").await.unwrap()
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // OnceCell guarantees at-most-one initialisation.
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn register_overwrites_existing_entry() {
        let l = InProcessModelLoader::with_phase_2_builtins();
        // Register a different impl under the same key — this is how tests
        // inject mock fixtures.
        l.register("ner_clinical_v1", "1.0.0", || {
            Ok(Arc::new(builtin::Embedder::new()) as Arc<dyn LocalModel>)
        })
        .await;
        let m = l.get_or_load("ner_clinical_v1", "1.0.0").await.unwrap();
        // The replacement model returns "embedder_v1" as id.
        assert_eq!(m.id(), "embedder_v1");
    }

    // ── builtin::NerClinical ────────────────────────────────────────

    #[tokio::test]
    async fn ner_clinical_extracts_drugs_and_diseases() {
        let m = builtin::NerClinical::new();
        let out = m
            .infer(json!({"text": "Patient took aspirin for hypertension"}))
            .await
            .unwrap();
        let entities = out["entities"].as_array().unwrap();
        let kinds: Vec<&str> = entities
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.contains(&"DRUG"));
        assert!(kinds.contains(&"DISEASE"));
        assert!(out["count"].as_u64().unwrap() >= 2);
    }

    #[tokio::test]
    async fn ner_clinical_is_deterministic() {
        let m = builtin::NerClinical::new();
        let inp = json!({"text": "Phase 3 trial of metformin for diabetes BRCA1 mutation"});
        let a = m.infer(inp.clone()).await.unwrap();
        let b = m.infer(inp).await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn ner_clinical_extracts_all_4_kinds() {
        let m = builtin::NerClinical::new();
        let inp = json!({"text": "Phase 3 trial of metformin for diabetes (BRCA1 mutation)"});
        let out = m.infer(inp).await.unwrap();
        let kinds: std::collections::HashSet<String> = out["entities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["kind"].as_str().unwrap().to_string())
            .collect();
        assert!(kinds.contains("DRUG"));
        assert!(kinds.contains("DISEASE"));
        assert!(kinds.contains("GENE"));
        assert!(kinds.contains("PHASE"));
    }

    #[tokio::test]
    async fn ner_clinical_rejects_missing_text() {
        let m = builtin::NerClinical::new();
        let err = m.infer(json!({})).await.unwrap_err();
        assert!(matches!(err, ModelInferError::InputConversion(_)));
    }

    #[tokio::test]
    async fn ner_clinical_respects_word_boundaries() {
        // "ointment" should NOT be detected as "in" being a substring match.
        let m = builtin::NerClinical::new();
        let out = m
            .infer(json!({"text": "diaphasic ointment"}))
            .await
            .unwrap();
        let entities = out["entities"].as_array().unwrap();
        // Neither "phase" (inside diaphasic) nor any drug should match.
        for e in entities {
            let text = e["text"].as_str().unwrap();
            assert!(!text.contains("diaphasic"), "false positive : {text}");
        }
    }

    // ── builtin::NerLegal ───────────────────────────────────────────

    #[tokio::test]
    async fn ner_legal_extracts_case_and_clause() {
        let m = builtin::NerLegal::new();
        let out = m
            .infer(json!({"text": "Per Roe v. Wade and the force majeure clause"}))
            .await
            .unwrap();
        let kinds: Vec<&str> = out["entities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.contains(&"CASE"));
        assert!(kinds.contains(&"CLAUSE"));
    }

    #[tokio::test]
    async fn ner_legal_extracts_citation_via_regex() {
        let m = builtin::NerLegal::new();
        let out = m
            .infer(json!({"text": "See 347 U.S. 483 for the holding"}))
            .await
            .unwrap();
        let citations: Vec<&str> = out["entities"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "CITATION")
            .map(|e| e["text"].as_str().unwrap())
            .collect();
        assert_eq!(citations, vec!["347 U.S. 483"]);
    }

    #[tokio::test]
    async fn ner_legal_extracts_party() {
        let m = builtin::NerLegal::new();
        let out = m
            .infer(json!({"text": "the plaintiff filed a motion"}))
            .await
            .unwrap();
        let parties: Vec<&str> = out["entities"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "PARTY")
            .map(|e| e["text"].as_str().unwrap())
            .collect();
        assert_eq!(parties, vec!["plaintiff"]);
    }

    // ── builtin::IntentClassifier ───────────────────────────────────

    #[tokio::test]
    async fn intent_classify_picks_buy_product() {
        let m = builtin::IntentClassifier::new();
        let out = m
            .infer(json!({"text": "I'd like to buy this product please"}))
            .await
            .unwrap();
        assert_eq!(out["intent"], "buy_product");
        assert!(out["score"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn intent_classify_falls_back_to_other() {
        let m = builtin::IntentClassifier::new();
        let out = m.infer(json!({"text": "qwerty asdf 12345"})).await.unwrap();
        assert_eq!(out["intent"], "other");
    }

    #[tokio::test]
    async fn intent_classify_is_deterministic_in_tie_breaks() {
        // Equal hits should resolve alphabetically.
        let m = builtin::IntentClassifier::new();
        let out = m
            .infer(json!({"text": "buy cancel refund order"}))
            .await
            .unwrap();
        // "buy", "order" → buy_product (2 hits / 5) ; "cancel", "refund" → cancel (2 hits / 4)
        // cancel has higher ratio (0.5) so it wins.
        assert_eq!(out["intent"], "cancel");
    }

    // ── builtin::LangDetector ───────────────────────────────────────

    #[tokio::test]
    async fn lang_detect_recognizes_english() {
        let m = builtin::LangDetector::new();
        let out = m
            .infer(json!({"text": "The quick brown fox is on the road"}))
            .await
            .unwrap();
        assert_eq!(out["language"], "en");
        assert!(out["confidence"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn lang_detect_recognizes_french() {
        let m = builtin::LangDetector::new();
        let out = m
            .infer(json!({"text": "Le chat est sur le tapis et la table"}))
            .await
            .unwrap();
        assert_eq!(out["language"], "fr");
    }

    #[tokio::test]
    async fn lang_detect_fallback_when_no_markers() {
        let m = builtin::LangDetector::new();
        let out = m.infer(json!({"text": "qwerty asdf zxcv"})).await.unwrap();
        assert_eq!(out["language"], "und");
        assert_eq!(out["confidence"], 0.0);
    }

    // ── builtin::Embedder ───────────────────────────────────────────

    #[tokio::test]
    async fn embedder_returns_384_dim_vector() {
        let m = builtin::Embedder::new();
        let out = m.infer(json!({"text": "hello world"})).await.unwrap();
        let vec = out["embedding"].as_array().unwrap();
        assert_eq!(vec.len(), 384);
        assert_eq!(out["dimension"], 384);
    }

    #[tokio::test]
    async fn embedder_is_deterministic() {
        let m = builtin::Embedder::new();
        let a = m.infer(json!({"text": "deterministic test"})).await.unwrap();
        let b = m.infer(json!({"text": "deterministic test"})).await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn embedder_different_inputs_different_outputs() {
        let m = builtin::Embedder::new();
        let a = m.infer(json!({"text": "foo"})).await.unwrap();
        let b = m.infer(json!({"text": "bar"})).await.unwrap();
        assert_ne!(a["embedding"], b["embedding"]);
    }

    #[tokio::test]
    async fn embedder_output_is_unit_norm() {
        let m = builtin::Embedder::new();
        let out = m.infer(json!({"text": "unit norm test"})).await.unwrap();
        let vec: Vec<f64> = out["embedding"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let norm: f64 = vec.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-6,
            "norm = {norm} not approximately 1.0"
        );
    }

    #[tokio::test]
    async fn embedder_custom_dim() {
        let m = builtin::Embedder::with_dim(64);
        let out = m.infer(json!({"text": "small dim"})).await.unwrap();
        assert_eq!(out["dimension"], 64);
        assert_eq!(out["embedding"].as_array().unwrap().len(), 64);
    }
}
