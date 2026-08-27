// SPDX-License-Identifier: UNLICENSED
//! Sprint **D19 (P1-Foundation)** — AST lint hot-path P1.
//!
//! Cf. `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 6.3 (test AST lint) et
//! § 3.6 (hot-path policy : ce qui est interdit).
//!
//! ## Mission
//!
//! Parser tous les modules listés dans `HOT_PATH_MODULES` via `syn`
//! et **rejeter** toute occurrence de :
//! - `Vec::new()`, `Vec::with_capacity()` (sauf annotation explicite)
//! - `Box::new(_)`, `Box::leak(_)`
//! - `String::new()`, `String::from(_)`, `format!()`
//! - `tokio::time::sleep`, `std::thread::sleep`
//! - `.unwrap()`, `.expect()` sans annotation `#[allow(p1_unwrap)]`
//! - macros `println!`, `eprintln!`, `dbg!`
//! - macros `info!`, `warn!` sans annotation `#[allow(p1_hot_path_log)]`
//!
//! Une violation **fait fail le test** et imprime le path, la ligne
//! approximative, et la nature de la violation. Le path est imprimé
//! pour qu'un dev voie immédiatement où c'est cassé.
//!
//! ## Escape hatch
//!
//! Si une violation est légitime (ex. allocation au boot, dans une
//! branche `#[cold]`, dans un test inline), elle peut être annotée :
//!
//! ```ignore
//! #[allow(p1_alloc)]    // justifie : boot only, see § 7.2 of P1 doc
//! let buf: Vec<u8> = Vec::with_capacity(4096);
//! ```
//!
//! Le visiteur respecte uniquement les `#[allow(p1_*)]` POSITIONNÉS
//! sur l'item parent (function, block, statement) — pas les
//! `#[allow]` génériques (`#[allow(unused)]` etc.).
//!
//! ## Limites connues
//!
//! - Le visiteur ne descend PAS dans les expressions générées par
//!   macros (le `syn::ExprMacro` est inspecté par son nom mais pas
//!   par son corps). Une `Vec::new()` cachée dans un `vec![]` n'est
//!   pas détectée — c'est OK car `vec![]` est lui-même listé.
//! - Le visiteur ne fait pas de type-inference : il regarde le nom
//!   du `Path` brut. Cela peut produire des faux positifs sur des
//!   `MyType::new()` qui se trouvent s'appeler comme un banned.
//!   Solution : utiliser le `#[allow(p1_*)]` escape hatch.
//!
//! ## Cibles couvertes
//!
//! Cf. § 3.1 du doc P1. Cette liste est la **source de vérité unique** ;
//! le test échoue si un module listé n'existe pas (= un refactor a
//! cassé un path).

use std::path::PathBuf;

use syn::spanned::Spanned;
use syn::visit::Visit;

// ====================================================================
// Liste des modules hot-path (cf. doc P1 § 3.1)
// ====================================================================

/// Liste exhaustive des fichiers Rust qui constituent le hot-path P1.
///
/// Chaque entrée est un path **relatif à la racine du repo**. Le
/// test échoue si le path n'existe pas (≠ refactor silencieux).
const HOT_PATH_MODULES: &[&str] = &[
    // Étages 1, 2, 11, 12 (gRPC entry/exit)
    "crates/efe-server/src/services/swarm.rs",
    // Étage 3 (interceptors)
    "crates/efe-server/src/interceptors/mod.rs",
    // Étage 6 (genesis)
    "crates/efe-core/src/genesis/engine.rs",
    // Étage 7 (fusion CPU MVP)
    "crates/efe-core/src/fusion/mvp_cpu.rs",
    // Étage 8 (consensus)
    "crates/efe-core/src/consensus/multi_criteria.rs",
    // Étage 9 (dissolution)
    "crates/efe-core/src/dissolution/engine.rs",
    // Étage 10 (audit chain) — Cf. D14-bis
    // path TBD post audit chain wire-up ; pour l'instant on garde
    // mode1.rs qui contient la grande majorité de l'orchestration. **Depuis R1 il vit dans
    // `efe-mode1`** (extrait d'efe-server en feuille) : le chemin change, le lint reste — le
    // retirer de cet inventaire aurait désarmé la garde sur les 1 979 lignes les plus chaudes
    // du dépôt, ce qu'un simple déménagement ne justifie pas.
    "crates/efe-mode1/src/mode1.rs",
];

// ====================================================================
// Patterns bannis
// ====================================================================

/// Bannis : Path::method (e.g. `Vec::new`).
const BANNED_PATHS: &[&str] = &[
    "Vec::new",
    "Vec::with_capacity",
    "Box::new",
    "Box::leak",
    "String::new",
    "String::from",
    "tokio::time::sleep",
    "tokio::time::sleep_until",
    "std::thread::sleep",
    "thread::sleep",
];

/// Bannis : macros par nom de path (le segment final).
const BANNED_MACROS: &[&str] = &[
    "println", "eprintln", "print", "eprint", "dbg",
    "vec", // vec![] alloc → utiliser arena ou pool
    "format",
];

/// Bannis : method calls (e.g. `.unwrap()`). Le visiteur n'inspecte
/// que les `MethodCall` ; les `expect` avec literal sont également
/// rejetés.
const BANNED_METHODS: &[&str] = &["unwrap", "expect"];

/// Macros de logging qui doivent être annotées `#[allow(p1_hot_path_log)]`
/// pour passer.
const LOG_MACROS_REQUIRING_ALLOW: &[&str] = &[
    "info", "warn",
    "error", // exception : `error!` est autorisé sans allow en hot-path
            //            mais on le liste ici puis on le whitelist dans
            //            la logique du visiteur (cf. § 4.3 doc P1).
];

// ====================================================================
// Visiteur
// ====================================================================

#[derive(Debug)]
struct Violation {
    file: String,
    /// Spans are difficult to map to line numbers on stable Rust
    /// without the `procmacro2_semver_exempt` cfg. We surface the
    /// `Debug` of the span instead, which contains "#0 bytes(X..Y)"
    /// — good enough to grep into the file with `wc -c`. The fix
    /// is to grep the offending pattern in the file.
    span_debug: String,
    kind: &'static str,
    detail: String,
}

#[derive(Default)]
struct HotPathVisitor {
    file: String,
    violations: Vec<Violation>,
    /// Stack of "allow" annotations active at the current depth.
    /// Each entry is the set of allowed lint names at that level.
    allow_stack: Vec<Vec<String>>,
}

impl HotPathVisitor {
    fn new(file: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            violations: Vec::new(),
            allow_stack: Vec::new(),
        }
    }

    /// Returns true if any active scope allows the given lint name.
    fn is_allowed(&self, lint: &str) -> bool {
        self.allow_stack
            .iter()
            .any(|scope| scope.iter().any(|n| n == lint))
    }

    /// Returns true if any active scope allows ANY p1_* lint
    /// (escape hatch global pour les fichiers entiers via
    /// `#![allow(p1_alloc, p1_log)]`).
    fn is_p1_anywhere_allowed(&self, prefix: &str) -> bool {
        self.allow_stack
            .iter()
            .any(|scope| scope.iter().any(|n| n.starts_with(prefix)))
    }

    fn push(&mut self, attrs: &[syn::Attribute]) {
        let mut scope = Vec::new();
        for attr in attrs {
            if attr.path().is_ident("allow") {
                let _ = attr.parse_nested_meta(|meta| {
                    if let Some(ident) = meta.path.get_ident() {
                        scope.push(ident.to_string());
                    }
                    Ok(())
                });
            }
        }
        self.allow_stack.push(scope);
    }

    fn pop(&mut self) {
        self.allow_stack.pop();
    }

    fn record(&mut self, span: proc_macro2::Span, kind: &'static str, detail: String) {
        self.violations.push(Violation {
            file: self.file.clone(),
            span_debug: format!("{:?}", span),
            kind,
            detail,
        });
    }

    /// Returns the dot-joined name of a `syn::Path` (e.g. `Vec::new`).
    fn path_string(p: &syn::Path) -> String {
        p.segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::")
    }
}

impl<'ast> Visit<'ast> for HotPathVisitor {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.push(&node.attrs);
        syn::visit::visit_item_fn(self, node);
        self.pop();
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.push(&node.attrs);
        syn::visit::visit_impl_item_fn(self, node);
        self.pop();
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.push(&node.attrs);
        syn::visit::visit_trait_item_fn(self, node);
        self.pop();
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        self.push(&node.attrs);
        syn::visit::visit_local(self, node);
        self.pop();
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = node.func.as_ref() {
            let name = Self::path_string(&p.path);
            if BANNED_PATHS.iter().any(|b| *b == name) && !self.is_p1_anywhere_allowed("p1_alloc") {
                self.record(p.span(), "banned_path_call", name);
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let m = node.method.to_string();
        if BANNED_METHODS.iter().any(|b| **b == m) && !self.is_p1_anywhere_allowed("p1_unwrap") {
            self.record(node.span(), "banned_method", m.clone());
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        self.check_macro_path(&node.mac.path, node.span());
        syn::visit::visit_expr_macro(self, node);
    }

    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        // `println!("hi");` au statement level passe par `StmtMacro`,
        // pas `ExprMacro` (cf. syn 2.0 AST). On doit visiter les deux
        // sinon on rate les macros idiomatiques.
        self.check_macro_path(&node.mac.path, node.span());
        syn::visit::visit_stmt_macro(self, node);
    }
}

impl HotPathVisitor {
    /// Vérifie un `Path` de macro (= dernier segment) contre les
    /// listes `LOG_MACROS_REQUIRING_ALLOW` et `BANNED_MACROS`.
    fn check_macro_path(&mut self, p: &syn::Path, span: proc_macro2::Span) {
        let name = p
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        // Log macros (info/warn) → besoin de #[allow(p1_hot_path_log)]
        // `error!` est autorisé sans allow (cf. § 4.3 doc P1).
        if LOG_MACROS_REQUIRING_ALLOW.iter().any(|b| **b == name) && name != "error" {
            if !self.is_p1_anywhere_allowed("p1_hot_path_log") {
                self.record(span, "banned_log_macro", name.clone());
            }
        }
        if BANNED_MACROS.iter().any(|b| **b == name) && !self.is_p1_anywhere_allowed("p1_alloc") {
            self.record(span, "banned_macro", name);
        }
    }
}

// ====================================================================
// Helpers
// ====================================================================

/// Returns the workspace root (= parent of `crates/`).
fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR pointe vers crates/efe-server/ ; remonter
    // de 2 niveaux donne la racine workspace.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable from CARGO_MANIFEST_DIR")
        .to_path_buf()
}

fn parse_and_lint(path: &PathBuf) -> Vec<Violation> {
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("can't read {}: {}", path.display(), e));
    let file =
        syn::parse_file(&src).unwrap_or_else(|e| panic!("can't parse {}: {}", path.display(), e));
    let mut v = HotPathVisitor::new(path.display().to_string());
    // Inner attrs ( `#![allow(...)]` ) at the file level.
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    v.violations
}

// ====================================================================
// Tests
// ====================================================================

#[test]
fn all_hot_path_modules_exist() {
    let root = workspace_root();
    let mut missing = Vec::new();
    for rel in HOT_PATH_MODULES {
        let p = root.join(rel);
        if !p.exists() {
            missing.push(p.display().to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "Some HOT_PATH_MODULES paths are missing (refactor drift ?) : {:#?}\nCf. tests/p1_hot_path_lint.rs HOT_PATH_MODULES.",
        missing
    );
}

/// Test **ratchet** sur le baseline `tests/data/p1_hot_path_baseline.json`.
///
/// La dette pré-existante hot-path (= les ~86 violations héritées
/// des sprints D1-D18) est **gelée** dans le baseline. Le test
/// **FAIL uniquement** si :
///
/// - Un fichier listé voit son count de violations **augmenter** vs
///   baseline (= régression : un dev introduit une nouvelle alloc
///   sur le hot-path).
/// - Le baseline référence un fichier qui n'existe plus
///   (= refactor cassé sans mise à jour du baseline).
///
/// Pour **ratcheter vers 0** :
/// 1. Le dev nettoie une violation dans `<file>`.
/// 2. Re-run avec `EFE_P1_LINT_UPDATE_BASELINE=1` ; le baseline est
///    réécrit avec les nouveaux counts.
/// 3. Commit le baseline mis à jour avec le fix.
///
/// La dette terminale visée = 0 (cf. doc P1 § 16.1). Le baseline est
/// la **mémoire institutionnelle** de la dette restante.
#[test]
fn hot_path_lint_ratchet() {
    let root = workspace_root();

    // 1. Compte les violations par fichier dans l'état courant.
    let mut current: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for rel in HOT_PATH_MODULES {
        let p = root.join(rel);
        if !p.exists() {
            continue; // déjà flaggé par all_hot_path_modules_exist
        }
        let count = parse_and_lint(&p).len();
        current.insert((*rel).to_string(), count);
    }

    // 2. Lit le baseline.
    let baseline_path = root.join("crates/efe-server/tests/data/p1_hot_path_baseline.json");
    let baseline: std::collections::BTreeMap<String, usize> = if baseline_path.exists() {
        let raw = std::fs::read_to_string(&baseline_path).expect("read baseline");
        serde_json::from_str(&raw).expect("baseline JSON valid")
    } else {
        std::collections::BTreeMap::new()
    };

    // 3. Mode update : réécrit le baseline et quitte succès.
    if std::env::var("EFE_P1_LINT_UPDATE_BASELINE").as_deref() == Ok("1") {
        if let Some(parent) = baseline_path.parent() {
            std::fs::create_dir_all(parent).expect("create data dir");
        }
        let serialized = serde_json::to_string_pretty(&current).expect("serialize current");
        std::fs::write(&baseline_path, format!("{}\n", serialized)).expect("write baseline");
        eprintln!("P1 lint baseline updated at {}", baseline_path.display());
        return;
    }

    // 4. Compare. Fail si UN fichier dépasse son count baseline OU
    //    si un fichier du baseline a disparu (= refactor non documenté).
    let mut violations: Vec<String> = Vec::new();
    for (file, &cur) in &current {
        let base = baseline.get(file).copied().unwrap_or(0);
        if cur > base {
            violations.push(format!(
                "  + {} : {} violations (baseline = {}, +{} new)",
                file,
                cur,
                base,
                cur - base
            ));
        }
    }
    for file in baseline.keys() {
        if !current.contains_key(file) {
            violations.push(format!(
                "  - {} : présent dans baseline mais absent du HOT_PATH_MODULES (refactor non documenté ?)",
                file
            ));
        }
    }

    if !violations.is_empty() {
        let total_cur: usize = current.values().sum();
        let total_base: usize = baseline.values().sum();
        panic!(
            "\nP1 hot-path lint RATCHET regression :\n\nViolations diff :\n{}\n\nCurrent total : {}  /  Baseline total : {}\n\nFix : nettoyer la nouvelle violation, OR (si refactor légitime) run :\n    EFE_P1_LINT_UPDATE_BASELINE=1 cargo test --test p1_hot_path_lint -- hot_path_lint_ratchet\n\nDoc : `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 3.6.\n",
            violations.join("\n"),
            total_cur,
            total_base
        );
    }

    // 5. Si current < baseline (= dev a nettoyé), warn + suggestion
    //    de ratchet manuel.
    let total_cur: usize = current.values().sum();
    let total_base: usize = baseline.values().sum();
    if total_cur < total_base {
        eprintln!(
            "🎉 P1 lint debt decreased: {} → {} (-{}). Run `EFE_P1_LINT_UPDATE_BASELINE=1 cargo test --test p1_hot_path_lint -- hot_path_lint_ratchet` to ratchet down.",
            total_base,
            total_cur,
            total_base - total_cur
        );
    }
}

#[test]
fn visitor_detects_vec_new() {
    let src = r#"
        fn f() {
            let v: Vec<i32> = Vec::new();
        }
    "#;
    let file = syn::parse_file(src).expect("ok");
    let mut v = HotPathVisitor::new("test_inline");
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    assert!(
        v.violations
            .iter()
            .any(|x| x.kind == "banned_path_call" && x.detail == "Vec::new"),
        "should detect Vec::new ; got {:#?}",
        v.violations
    );
}

#[test]
fn visitor_detects_unwrap() {
    let src = r#"
        fn f(x: Option<i32>) -> i32 { x.unwrap() }
    "#;
    let file = syn::parse_file(src).expect("ok");
    let mut v = HotPathVisitor::new("test_inline");
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    assert!(
        v.violations
            .iter()
            .any(|x| x.kind == "banned_method" && x.detail == "unwrap"),
        "should detect unwrap ; got {:#?}",
        v.violations
    );
}

#[test]
fn visitor_detects_println() {
    let src = r#"
        fn f() { println!("hello"); }
    "#;
    let file = syn::parse_file(src).expect("ok");
    let mut v = HotPathVisitor::new("test_inline");
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    assert!(
        v.violations
            .iter()
            .any(|x| x.kind == "banned_macro" && x.detail == "println"),
        "should detect println ; got {:#?}",
        v.violations
    );
}

#[test]
fn visitor_respects_p1_alloc_escape_hatch() {
    let src = r#"
        #[allow(p1_alloc)]
        fn boot_init() {
            let v: Vec<i32> = Vec::new();
            let s = String::from("hello");
        }
    "#;
    let file = syn::parse_file(src).expect("ok");
    let mut v = HotPathVisitor::new("test_inline");
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    assert!(
        v.violations.is_empty(),
        "should respect p1_alloc escape ; got {:#?}",
        v.violations
    );
}

#[test]
fn visitor_respects_p1_unwrap_escape_hatch() {
    let src = r#"
        #[allow(p1_unwrap)]
        fn boot_init(x: Option<i32>) -> i32 { x.unwrap() }
    "#;
    let file = syn::parse_file(src).expect("ok");
    let mut v = HotPathVisitor::new("test_inline");
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    assert!(
        v.violations.is_empty(),
        "should respect p1_unwrap escape ; got {:#?}",
        v.violations
    );
}

#[test]
fn visitor_allows_error_macro_without_annotation() {
    // `error!` est WHITELISTÉ par défaut sur le hot-path (cf. § 4.3
    // du doc P1 : "ERROR | 100 % | Exception, panic catch, RPC failure").
    let src = r#"
        fn f() { error!("rpc failure"); }
        macro_rules! error { ($($t:tt)*) => {{}}; }
    "#;
    let file = syn::parse_file(src).expect("ok");
    let mut v = HotPathVisitor::new("test_inline");
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    assert!(
        v.violations.is_empty(),
        "error! should be whitelisted ; got {:#?}",
        v.violations
    );
}

#[test]
fn visitor_flags_info_macro_without_allow() {
    let src = r#"
        fn f() { info!("hi"); }
    "#;
    let file = syn::parse_file(src).expect("ok");
    let mut v = HotPathVisitor::new("test_inline");
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    assert!(
        v.violations
            .iter()
            .any(|x| x.kind == "banned_log_macro" && x.detail == "info"),
        "should flag info! without #[allow(p1_hot_path_log)] ; got {:#?}",
        v.violations
    );
}

#[test]
fn visitor_respects_p1_hot_path_log_allow() {
    let src = r#"
        #[allow(p1_hot_path_log)]
        fn f() { info!("hi"); }
    "#;
    let file = syn::parse_file(src).expect("ok");
    let mut v = HotPathVisitor::new("test_inline");
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    assert!(
        v.violations.is_empty(),
        "should respect p1_hot_path_log ; got {:#?}",
        v.violations
    );
}

#[test]
fn visitor_detects_tokio_sleep() {
    let src = r#"
        async fn f() {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    "#;
    let file = syn::parse_file(src).expect("ok");
    let mut v = HotPathVisitor::new("test_inline");
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    assert!(
        v.violations
            .iter()
            .any(|x| x.detail == "tokio::time::sleep"),
        "should detect tokio::time::sleep ; got {:#?}",
        v.violations
    );
}

#[test]
fn file_level_allow_is_respected() {
    let src = r#"
        #![allow(p1_alloc)]
        fn f() {
            let _ = Vec::<i32>::new();
            let _ = String::from("hi");
        }
    "#;
    let file = syn::parse_file(src).expect("ok");
    let mut v = HotPathVisitor::new("test_inline");
    v.push(&file.attrs);
    syn::visit::visit_file(&mut v, &file);
    v.pop();
    assert!(
        v.violations.is_empty(),
        "should respect #![allow(p1_alloc)] at file level ; got {:#?}",
        v.violations
    );
}
