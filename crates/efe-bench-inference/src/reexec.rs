// SPDX-License-Identifier: UNLICENSED
//! **HB.3 — le vérificateur arithmétique DÉPLOYABLE (re-exécution).** Le levier qui décide le régime (b).
//!
//! HB.2 a tranché, sur du raisonnement réel : sur `phi4-mini × T-MATH`, la bonne réponse **est présente**
//! dans les K candidats (`pass@K`=100 %) mais **minoritaire** sur les ~6 % d'items durs — donc le vote nu
//! (self-consistency) la rate. `pass@K` est un **oracle** (il lit le label) : non déployable. Et le
//! vérificateur-**filtre** existant ([`crate::aggregation::PositiveInteger`]) ne peut **pas** déplacer le
//! mode — les réponses majoritaires fausses sont aussi des entiers positifs, elles passent le filtre, le
//! mode ne bouge pas (c'est pourquoi `Verified` était plat à 94 %). Pour faire gagner une **minorité bien
//! raisonnée** contre une **majorité bâclée**, il faut **re-pondérer** le vote par un signal de qualité
//! **par candidat** — et ce signal doit être **sans label**, **sans GPU**, **sans 2ᵉ LLM**, et **exact**.
//!
//! Le signal retenu (panel de conception adversarial, design *ReExecWeightedVote*) : **re-exécuter
//! l'arithmétique que le candidat a lui-même écrite**. On extrait ses équations « a op b = c », on
//! recalcule chaque membre gauche en **rationnels exacts sur i128** (aucun flottant dans l'audit), et on
//! compte combien tiennent (`H`) vs combien cassent (`M`). Ce compte devient un **poids de vote** injecté
//! dans le `mode(ans, scorer, Some(&poids))` **déjà prouvé** (HB.1). Un **garde de non-infériorité** (cf.
//! [`crate::aggregation`]) interdit de dévier de la majorité tant que l'appui audité de la minorité ne
//! dépasse pas un **seuil pré-enregistré** — de sorte que, sur les items à faible signal, RXW **rend
//! exactement** le choix de la self-consistency (jamais pire par construction sur ces items).
//!
//! ## Les garde-fous d'honnêteté (chacun né d'une passe de red-team)
//! - **Sans label, par SIGNATURE** : [`audit_candidate`] reçoit `(&[i128] prompt, &str texte)` — ni
//!   `SampleSet`, ni label. La fuite du label est **inexprimable au type**. Le label n'entre qu'**après**
//!   la sélection, dans le scoring B1, exactement comme pour la self-consistency.
//! - **SÉLECTION ≤ pass@K** : RXW ne **synthétise** jamais une réponse absente ; il **re-pondère** des
//!   candidats existants. Sa justesse est donc bornée par `pass@K` (≠ médiane). La revendication honnête
//!   est « RXW > self-consistency » (récupérer du *headroom*), **jamais** « RXW dépasse pass@K ».
//! - **EXACT, frontière f64 documentée** : audit et garde en `i128`/`Ratio` purs ; le **seul** f64 est le
//!   poids remis au `mode()` existant (sommé avec sa bande 1e-9). Les poids sont des rationnels à petit
//!   dénominateur et K≤~7 → les sommes f64 sont exactes ≪ 1e-9 → déterminisme/re-prouvabilité tenus.
//! - **PAS un oracle de la structure** : on refuse explicitement le *plug-back* qui re-dériverait la
//!   fonction génératrice du corpus (`corpus.rs`) — ce serait la malhonnêteté du label déplacée vers le
//!   générateur. RXW n'audite QUE l'arithmétique **écrite par le candidat**, jamais le modèle du problème.
//! - **PAS une preuve de justesse** (contrainte irréductible) : un candidat au **mauvais montage mais bon
//!   calcul** (`12*3` au lieu de `12+3`, calculé juste) est audité **valide** → RXW le classe haut → mal
//!   sélectionné. La re-exécution valide le **calcul**, jamais la **modélisation**. La métrique est le
//!   *headroom* récupéré **mesuré** (et re-prouvé par le bootstrap couplé), jamais une garantie par item.
//!
//! Module **pur** : aucune inférence, aucune dépendance nouvelle (rationnel i128 fait main, réduit par pgcd).
//! Déterministe et total (tout chemin d'échec de parsing = équation **ignorée**, jamais une fausse note).

use serde_json::Value;

use crate::aggregation::ReasoningWeigher;
use crate::scorer::answer_text;

// ─────────────────────────────────────────────────────────────────────────────
//  Constantes PRÉ-ENREGISTRÉES (gelées AVANT de regarder quels items sont durs —
//  les régler sur le jeu scellé n=50 serait pass@K fuyant par l'expérimentateur).
// ─────────────────────────────────────────────────────────────────────────────

/// Base du numérateur de poids : un candidat sans aucune équation auditée ne part pas à zéro absolu.
pub(crate) const W_BASE: i64 = 2;
/// Pénalité par équation FAUSSE (cassée) — pèse double dans le dénominateur (`PENALTY*2*M`).
pub(crate) const PENALTY: i64 = 2;
/// Marge d'override du garde : la minorité ne renverse la majorité que si son appui audité (Σ H2) la
/// dépasse d'**au moins** `MARGIN` (en demi-points H2, soit ≥ 1 équation vérifiée genuine d'avance).
pub(crate) const MARGIN: i64 = 2;
/// Poids minimal (rationnel exact 1/4) — un candidat ne tombe jamais à 0 (il vote, faiblement).
pub const W_MIN: f64 = 0.25;
/// Poids maximal (1) — un candidat tout-juste vote à plein.
pub const W_MAX: f64 = 1.0;
/// Plancher du **sans-preuve** (E=0, aucune équation lisible) : il vote au plancher, jamais à plein.
pub(crate) const W_FLOOR: f64 = 0.25;

// ─────────────────────────────────────────────────────────────────────────────
//  Rationnel exact sur i128 (fait main, réduit par pgcd ; zéro dépendance, total).
//  Plage du corpus : opérandes < 100, produits < ~10⁴ → i128 est exact et sans débordement ;
//  toute opération qui déborderait renvoie None (l'équation est alors ignorée, pas mal-notée).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ratio {
    num: i128,
    den: i128, // invariant : den > 0, pgcd(|num|,den) = 1
}
impl Ratio {
    fn new(num: i128, den: i128) -> Option<Ratio> {
        if den == 0 {
            return None;
        }
        let (mut n, mut d) = (num, den);
        if d < 0 {
            n = n.checked_neg()?;
            d = d.checked_neg()?;
        }
        let g = gcd(n.unsigned_abs(), d.unsigned_abs()) as i128;
        if g > 1 {
            n /= g;
            d /= g;
        }
        Some(Ratio { num: n, den: d })
    }
    fn from_int(n: i128) -> Ratio {
        Ratio { num: n, den: 1 }
    }
    fn add(self, o: Ratio) -> Option<Ratio> {
        Ratio::new(
            self.num.checked_mul(o.den)?.checked_add(o.num.checked_mul(self.den)?)?,
            self.den.checked_mul(o.den)?,
        )
    }
    fn sub(self, o: Ratio) -> Option<Ratio> {
        Ratio::new(
            self.num.checked_mul(o.den)?.checked_sub(o.num.checked_mul(self.den)?)?,
            self.den.checked_mul(o.den)?,
        )
    }
    fn mul(self, o: Ratio) -> Option<Ratio> {
        Ratio::new(self.num.checked_mul(o.num)?, self.den.checked_mul(o.den)?)
    }
    fn div(self, o: Ratio) -> Option<Ratio> {
        if o.num == 0 {
            return None; // division par zéro → équation ignorée
        }
        Ratio::new(self.num.checked_mul(o.den)?, self.den.checked_mul(o.num)?)
    }
}
fn gcd(a: u128, b: u128) -> u128 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

// ─────────────────────────────────────────────────────────────────────────────
//  Lexer / parseur d'expression arithmétique (descente récursive, précédence
//  standard, parenthèses, moins unaire). Total : toute anomalie → None → ignorée.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Num(i128),
    Add,
    Sub,
    Mul,
    Div,
    LP,
    RP,
}

/// Tokenise une chaîne **déjà normalisée** (alphabet `[0-9 , . + - * / ( ) espace]`). Renvoie `None`
/// (l'équation est ignorée) si elle contient un **décimal** (`.` dans un nombre — le corpus est entier ;
/// `10/3=3.33` ne doit pas être noté FAUX), ou un caractère inattendu, ou une virgule isolée.
fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let cs: Vec<char> = s.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            ' ' | '\t' => i += 1,
            '+' => {
                toks.push(Tok::Add);
                i += 1;
            }
            '*' => {
                toks.push(Tok::Mul);
                i += 1;
            }
            '/' => {
                toks.push(Tok::Div);
                i += 1;
            }
            '(' => {
                toks.push(Tok::LP);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RP);
                i += 1;
            }
            '-' => {
                toks.push(Tok::Sub);
                i += 1;
            }
            '0'..='9' => {
                let mut digits = String::new();
                let mut has_dot = false;
                while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == ',' || cs[i] == '.') {
                    if cs[i] == '.' {
                        has_dot = true;
                    }
                    if cs[i].is_ascii_digit() {
                        digits.push(cs[i]);
                    }
                    i += 1; // les virgules de milliers sont absorbées et ignorées
                }
                if has_dot {
                    return None; // décimal → on ignore l'équation entière (jamais un faux MISS)
                }
                toks.push(Tok::Num(digits.parse().ok()?));
            }
            ',' => i += 1, // virgule isolée (ponctuation de phrase « 4 each, so ») → séparateur, pas un bail
            _ => return None, // tout autre caractère résiduel → on ignore l'équation
        }
    }
    Some(toks)
}

/// Descente récursive : `expr := term (('+'|'-') term)* ; term := factor (('*'|'/') factor)* ;
/// factor := number | '(' expr ')' | '-' factor`. Renvoie `None` si l'expression ne se consomme pas
/// **entièrement** (jonc résiduel ⇒ équation ignorée — le garde strict contre les faux parses).
fn eval(toks: &[Tok]) -> Option<Ratio> {
    let mut p = Parser { t: toks, i: 0 };
    let v = p.expr()?;
    if p.i == toks.len() {
        Some(v)
    } else {
        None
    }
}
struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
}
impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn expr(&mut self) -> Option<Ratio> {
        let mut v = self.term()?;
        while let Some(op) = self.peek() {
            match op {
                Tok::Add => {
                    self.i += 1;
                    v = v.add(self.term()?)?;
                }
                Tok::Sub => {
                    self.i += 1;
                    v = v.sub(self.term()?)?;
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn term(&mut self) -> Option<Ratio> {
        let mut v = self.factor()?;
        while let Some(op) = self.peek() {
            match op {
                Tok::Mul => {
                    self.i += 1;
                    v = v.mul(self.factor()?)?;
                }
                Tok::Div => {
                    self.i += 1;
                    v = v.div(self.factor()?)?;
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn factor(&mut self) -> Option<Ratio> {
        match self.peek()? {
            Tok::Sub => {
                self.i += 1;
                let f = self.factor()?;
                Ratio::from_int(0).sub(f)
            }
            Tok::Num(n) => {
                let n = *n;
                self.i += 1;
                Some(Ratio::from_int(n))
            }
            Tok::LP => {
                self.i += 1;
                let e = self.expr()?;
                match self.peek()? {
                    Tok::RP => {
                        self.i += 1;
                        Some(e)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

/// Une équation est **triviale** (padding sans valeur de preuve) si elle est de la forme identité à deux
/// opérandes (`x*1`, `1*x`, `x+0`, `0+x`, `x-0`, `x/1`, ou `x-x`). Elle ne compte alors ni dans H ni dans
/// E (garde anti-gonflage : `5*1=5`, `7+0=7` ne doivent pas créditer un candidat).
fn is_trivial(toks: &[Tok]) -> bool {
    if toks.len() == 3 {
        if let (Tok::Num(a), op, Tok::Num(b)) = (toks[0], toks[1], toks[2]) {
            return match op {
                Tok::Mul => a == 1 || b == 1,
                Tok::Add => a == 0 || b == 0,
                Tok::Sub => b == 0 || a == b,
                Tok::Div => b == 1,
                _ => false,
            };
        }
    }
    false
}

// ─────────────────────────────────────────────────────────────────────────────
//  Normalisation du texte CoT → alphabet arithmétique (ancrée sur le VRAI format
//  phi4 : LaTeX `\(..\)`, `\times`, `$..$`, `\boxed{N}`, annotations `(initial)`).
// ─────────────────────────────────────────────────────────────────────────────

/// Mappe LaTeX/Unicode mathématiques vers l'ASCII (`\times`,`×`,`·`→`*` ; `\div`,`÷`→`/` ; `−`→`-`),
/// retire les délimiteurs (`\(`,`\)`,`$`,`{`,`}`,`\boxed`,`\frac`), efface les **parenthèses-mots**
/// (`(initial)` → vide) puis remplace tout caractère **non arithmétique** par une espace. Déterministe.
fn normalize_math(t: &str) -> String {
    let mut s = t.to_string();
    for (from, to) in [("\\times", " * "), ("\\cdot", " * "), ("\\div", " / "), ("\\frac", " ")] {
        s = s.replace(from, to);
    }
    s = s.replace('×', " * ").replace('÷', " / ").replace('·', " * ").replace('−', " - ");
    for d in ["\\(", "\\)", "\\[", "\\]", "\\boxed"] {
        s = s.replace(d, " ");
    }
    s = s.replace(['$', '{', '}'], " ");
    let s = strip_word_parens(&s);
    s.chars()
        .map(|c| if c.is_ascii_digit() || matches!(c, '+' | '-' | '*' | '/' | '(' | ')' | '=' | '.' | ',' | ' ') { c } else { ' ' })
        .collect()
}

/// Efface les parenthèses dont l'intérieur est **uniquement lettres/espaces** (annotation d'unité, ex.
/// `68 (initial) - 23 (taken)` → `68  - 23`). Préserve `(5+3)` (intérieur numérique). Non imbriqué.
fn strip_word_parens(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(cs.len());
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '(' {
            if let Some(rel) = cs[i + 1..].iter().position(|&c| c == ')') {
                let interior = &cs[i + 1..i + 1 + rel];
                if !interior.is_empty() && interior.iter().all(|c| c.is_alphabetic() || c.is_whitespace()) {
                    for _ in 0..rel + 2 {
                        out.push(' ');
                    }
                    i += rel + 2;
                    continue;
                }
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

/// Le **premier entier** d'une chaîne (le membre droit `= 45 remaining` → 45 ; `= 68 \)` → 68 ; `= -5` →
/// −5). `None` si le premier nombre est **décimal** (cohérent avec `tokenize` : on ignore l'équation).
/// Un `-` **collé** au début du nombre est un signe (`-5`) ; un `-` séparé par une espace (`x - 5`) est
/// un opérateur binaire (résultat positif). Revue HB.3 : sans ça, un membre droit négatif était mal lu.
fn first_int(s: &str) -> Option<i128> {
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if cs[i].is_ascii_digit() {
            let neg = i > 0 && cs[i - 1] == '-'; // signe collé au nombre
            let mut digits = String::new();
            let mut has_dot = false;
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == ',' || cs[i] == '.') {
                if cs[i] == '.' {
                    has_dot = true;
                }
                if cs[i].is_ascii_digit() {
                    digits.push(cs[i]);
                }
                i += 1;
            }
            return if has_dot { None } else { digits.parse::<i128>().ok().map(|v| if neg { -v } else { v }) };
        }
        i += 1;
    }
    None
}

/// Tous les **entiers** d'un texte (les littéraux de l'énoncé, pour la décote d'écho). Décimaux ignorés.
pub(crate) fn ints_in_text(t: &str) -> Vec<i128> {
    let norm = normalize_math(t);
    let cs: Vec<char> = norm.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i].is_ascii_digit() {
            let neg = i > 0 && cs[i - 1] == '-'; // signe collé (cohérent avec first_int, revue HB.3)
            let mut digits = String::new();
            let mut has_dot = false;
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == ',' || cs[i] == '.') {
                if cs[i] == '.' {
                    has_dot = true;
                }
                if cs[i].is_ascii_digit() {
                    digits.push(cs[i]);
                }
                i += 1;
            }
            if !has_dot {
                if let Ok(v) = digits.parse::<i128>() {
                    out.push(if neg { -v } else { v });
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
//  L'audit d'un candidat : (H2, M, E) en entiers exacts.
// ─────────────────────────────────────────────────────────────────────────────

/// Le score d'audit d'**un** candidat. `h2` est en **demi-points** : `2` par équation vérifiée *genuine*,
/// `1` par équation vérifiée *écho* (tous opérandes + résultat déjà dans l'énoncé — un fait recopié, pas
/// une preuve de raisonnement). `m` = équations fausses ; `e` = équations comptées (hits+miss, hors
/// triviales/ignorées). `e == 0` ⇒ candidat **non vérifiable** (vote au plancher).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct AuditScore {
    pub h2: i64,
    pub m: i64,
    pub e: i64,
}

/// Le **membre gauche** d'une équation : le plus **long suffixe valide** de la séquence de tokens qui
/// forme une expression complète avec ≥1 opérateur (non triviale). Deux raisons, ancrées sur le VRAI
/// format phi4 : (1) après `split('=')`, le morceau de gauche commence par le **résultat de l'équation
/// précédente** (chaîne « …= 68  68 - 23 = 45 ») — on doit ignorer ce préfixe parasite et ne garder que
/// `68 - 23` ; (2) on **rogne les opérateurs traînants** (`4 * 17 /` issu de « 17 pencils/row » → `4 *
/// 17`), pour récupérer l'équation sans introduire de division parasite. Renvoie (valeur, opérandes).
fn parse_trailing_expr(toks: &[Tok]) -> Option<(Ratio, Vec<i128>)> {
    let mut end = toks.len();
    while end > 0 && matches!(toks[end - 1], Tok::Add | Tok::Sub | Tok::Mul | Tok::Div | Tok::LP) {
        end -= 1; // un opérateur (ou '(') traînant ne peut terminer une expression
    }
    let toks = &toks[..end];
    for start in 0..toks.len() {
        let suf = &toks[start..]; // suffixes du plus long au plus court
        // Une expression COMMENCE par un facteur (nombre ou '('), OU par un **moins unaire** — mais ce
        // dernier n'est légitime qu'à `start == 0` (le vrai début du membre gauche). À `start > 0`, un
        // `-` en tête est un opérateur BINAIRE dont l'opérande gauche a été rognée → faux parse (« 4 - 4 »
        // donnerait « - 4 » → −4). On autorise donc `Sub` en tête seulement quand start == 0 (revue HB.3).
        let starts_ok = matches!(suf[0], Tok::Num(_) | Tok::LP) || (start == 0 && suf[0] == Tok::Sub);
        if !starts_ok {
            continue;
        }
        if !suf.iter().any(|t| matches!(t, Tok::Add | Tok::Sub | Tok::Mul | Tok::Div)) {
            continue; // pas d'opérateur → pas une équation (un nombre nu)
        }
        if is_trivial(suf) {
            continue; // identité de padding
        }
        if let Some(v) = eval(suf) {
            let operands = suf.iter().filter_map(|t| if let Tok::Num(n) = t { Some(*n) } else { None }).collect();
            return Some((v, operands));
        }
    }
    None
}

/// Audite **une** chaîne de raisonnement : extrait les équations `<expr> = <nombre>`, recalcule chaque
/// membre gauche en rationnel exact, et compte hits (avec décote d'écho) / miss. `prompt_ints` = les
/// entiers de l'énoncé (pour la décote). **Sans label par signature.**
pub(crate) fn audit_candidate(prompt_ints: &[i128], text: &str) -> AuditScore {
    let norm = normalize_math(text);
    let parts: Vec<&str> = norm.split('=').collect();
    let mut score = AuditScore::default();
    for w in parts.windows(2) {
        let rhs = match first_int(w[1]) {
            Some(r) => r,
            None => continue, // pas de membre droit entier lisible → ignorée
        };
        let toks = match tokenize(w[0].trim()) {
            Some(t) => t,
            None => continue, // membre gauche non tokenisable (décimal, jonc) → ignorée
        };
        let (lhs, operands) = match parse_trailing_expr(&toks) {
            Some(x) => x,
            None => continue, // aucun suffixe valide → ignorée (jamais un faux MISS)
        };
        score.e += 1;
        if lhs == Ratio::from_int(rhs) {
            // Hit. Écho ⇔ tous les opérandes ET le résultat figurent déjà dans l'énoncé.
            let echo = !operands.is_empty()
                && operands.iter().all(|o| prompt_ints.contains(o))
                && prompt_ints.contains(&rhs);
            score.h2 += if echo { 1 } else { 2 };
        } else {
            score.m += 1;
        }
    }
    score
}

/// Le **poids de vote** dérivé d'un audit : `(W_BASE + H2) / (W_BASE + H2 + 2·PENALTY·M)`, borné
/// `[w_min, w_max]`. `E == 0` (aucune équation) ⇒ **plancher** `W_FLOOR` (vote faible, jamais à plein).
/// Seul point f64 du module — justifié : dénominateurs petits, K≤~7 → somme exacte ≪ 1e-9 dans `mode()`.
pub(crate) fn weight_from(a: &AuditScore, w_min: f64, w_max: f64) -> f64 {
    if a.e == 0 {
        return W_FLOOR;
    }
    let wn = (W_BASE + a.h2) as f64;
    let wd = (W_BASE + a.h2 + PENALTY * 2 * a.m) as f64;
    (wn / wd).clamp(w_min, w_max)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Le poids "objet" : le seam public, SANS LABEL PAR SIGNATURE.
// ─────────────────────────────────────────────────────────────────────────────

/// L'auditeur de re-exécution, implémentant [`ReasoningWeigher`]. Son `weight(task, answer)` ne reçoit
/// **ni** `SampleSet` **ni** label — la fuite est inexprimable. `Default` = bornes `[1/4, 1]`.
pub struct ReExecAudit {
    pub w_min: f64,
    pub w_max: f64,
}
impl Default for ReExecAudit {
    fn default() -> Self {
        Self { w_min: W_MIN, w_max: W_MAX }
    }
}
impl ReasoningWeigher for ReExecAudit {
    fn weight(&self, task: &Value, answer: &Value) -> f64 {
        let prompt_ints = ints_in_text(task.get("text").and_then(|v| v.as_str()).unwrap_or(""));
        let a = audit_candidate(&prompt_ints, &answer_text(answer));
        weight_from(&a, self.w_min, self.w_max)
    }
    fn w_min(&self) -> f64 {
        self.w_min
    }
    fn w_max(&self) -> f64 {
        self.w_max
    }
    fn name(&self) -> &str {
        "reexec_audit"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn r(n: i128) -> Ratio {
        Ratio::from_int(n)
    }

    // ── arithmétique exacte & précédence ──

    #[test]
    fn eval_respects_precedence_and_parens() {
        assert_eq!(eval(&tokenize("4 * 17").unwrap()), Some(r(68)));
        assert_eq!(eval(&tokenize("68 - 23").unwrap()), Some(r(45)));
        assert_eq!(eval(&tokenize("2 + 3 * 4").unwrap()), Some(r(14)), "précédence : *  avant +");
        assert_eq!(eval(&tokenize("(2 + 3) * 4").unwrap()), Some(r(20)), "parenthèses respectées");
        assert_eq!(eval(&tokenize("10 / 2").unwrap()), Some(r(5)), "division exacte entière");
        assert_eq!(eval(&tokenize("10 / 3").unwrap()), Ratio::new(10, 3), "division rationnelle exacte");
    }

    #[test]
    fn tokenize_handles_thousands_and_rejects_decimals() {
        assert_eq!(eval(&tokenize("1,000 + 234").unwrap()), Some(r(1234)), "virgule de milliers absorbée");
        assert!(tokenize("3.33").is_none(), "décimal → None (équation ignorée, pas un MISS)");
    }

    #[test]
    fn eval_rejects_trailing_junk() {
        // « 68 ( ) - 23 » (parenthèses vides après effacement d'annotation) ne se consomme pas → None.
        assert!(eval(&tokenize("68 ( ) - 23").unwrap()).is_none(), "parenthèses vides → parse échoue → ignorée");
    }

    // ── audit : hit / miss / écho / trivial ──

    #[test]
    fn audit_hit_basic_uses_exact_rational() {
        // « 3 * 4 = 12 » : 3,4 dans l'énoncé, 12 absent → hit GENUINE → H2=2.
        let a = audit_candidate(&[3, 4], "3 boxes of 4 each, so 3 * 4 = 12 total");
        assert_eq!(a, AuditScore { h2: 2, m: 0, e: 1 });
        assert!((weight_from(&a, W_MIN, W_MAX) - 1.0).abs() < 1e-12, "tout-juste → poids max");
    }

    #[test]
    fn audit_miss_penalizes() {
        // « 12 + 3 = 16 » faux (15) → M=1 → poids (2)/(2+4)=1/3.
        let a = audit_candidate(&[12, 3], "then 12 + 3 = 16");
        assert_eq!(a, AuditScore { h2: 0, m: 1, e: 1 });
        assert!((weight_from(&a, W_MIN, W_MAX) - 1.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn audit_decimal_rhs_is_skipped_not_missed() {
        // « 10 / 3 = 3.33 » : RHS décimal → équation IGNORÉE (E inchangé), pas un MISS.
        let a = audit_candidate(&[10, 3], "10 / 3 = 3.33 approx");
        assert_eq!(a, AuditScore { h2: 0, m: 0, e: 0 }, "décimal RHS ignoré");
        // « 10 / 2 = 5 » : hit exact.
        let b = audit_candidate(&[10, 2], "10 / 2 = 5");
        assert_eq!(b.e, 1);
    }

    #[test]
    fn audit_trivial_identities_do_not_count() {
        for t in ["2 = 2", "5 * 1 = 5", "7 + 0 = 7", "9 - 0 = 9", "8 / 1 = 8", "4 - 4 = 0"] {
            let a = audit_candidate(&[], t);
            assert_eq!(a.e, 0, "identité triviale ignorée : {t}");
        }
    }

    #[test]
    fn audit_echo_is_half_weight() {
        // Tous opérandes + résultat dans l'énoncé → écho (H2 += 1), vs dérivé (H2 += 2).
        let echo = audit_candidate(&[3, 4, 12], "3 * 4 = 12"); // 12 aussi dans l'énoncé
        assert_eq!(echo, AuditScore { h2: 1, m: 0, e: 1 }, "fait recopié → demi-poids");
        let genuine = audit_candidate(&[3, 4], "3 * 4 = 12"); // 12 absent de l'énoncé
        assert_eq!(genuine.h2, 2, "résultat dérivé → plein poids");
    }

    // ── normalisation ancrée sur le VRAI format phi4 ──

    #[test]
    fn audit_handles_real_phi4_latex() {
        // Échantillon réel (temp=0.2) : LaTeX `\(..\)`, `\times`, `\boxed`.
        let cot = "\\( 4 \\times 17 = 68 \\)\n\\( 68 - 23 = 45 \\)\n\\(\\boxed{45}\\)";
        let a = audit_candidate(&[4, 17, 23], cot);
        assert_eq!(a, AuditScore { h2: 4, m: 0, e: 2 }, "deux équations LaTeX vérifiées (68 dérivé, 45 dérivé)");
    }

    #[test]
    fn audit_handles_dollar_math_and_units() {
        // temp=0.8 : `$17 * 4 = 68$`, `$68 - 23=45$` (sans espaces autour de =).
        let a = audit_candidate(&[17, 4, 23], "$17 * 4 = 68$ then $68 - 23=45$");
        assert_eq!(a.e, 2);
        assert_eq!(a.m, 0, "les deux tiennent");
    }

    #[test]
    fn audit_strips_word_parentheticals() {
        // temp=0.5 : « 68 (initial) - 23 (taken) = 45 » → annotations effacées → 68 - 23 = 45 hit.
        let a = audit_candidate(&[68, 23], "= 68 (initial) - 23 (taken) = 45 remaining");
        assert_eq!(a.m, 0);
        assert!(a.e >= 1, "l'équation annotée est récupérée");
    }

    #[test]
    fn audit_units_slash_does_not_become_division() {
        // « 4 rows * 17 pencils/row = 68 » : 'pencils/row' a des LETTRES → la barre est isolée par des
        // espaces (mots effacés) → « 4 * 17 / 68 »? Non : 'pencils' et 'row' deviennent espaces, donc le
        // membre gauche est « 4   * 17        » et le '/' (collé à 'row', une lettre) disparaît avec le mot.
        let a = audit_candidate(&[4, 17], "4 rows * 17 pencils/row = 68 pencils");
        // 4 * 17 = 68 doit tenir (le /row n'a pas introduit de division parasite).
        assert_eq!(a.m, 0, "aucune division parasite : {a:?}");
        assert!(a.e >= 1);
    }

    // ── poids ──

    #[test]
    fn weight_floor_for_no_evidence() {
        let a = AuditScore { h2: 0, m: 0, e: 0 };
        assert!((weight_from(&a, W_MIN, W_MAX) - W_FLOOR).abs() < 1e-12, "sans équation → plancher");
    }

    #[test]
    fn weight_two_hits_two_misses() {
        let a = AuditScore { h2: 4, m: 2, e: 4 }; // (2+4)/(2+4+8) = 6/14 = 3/7
        assert!((weight_from(&a, W_MIN, W_MAX) - 3.0 / 7.0).abs() < 1e-12);
    }

    // ── ints_in_text & seam sans-label ──

    #[test]
    fn ints_in_text_extracts_prompt_literals() {
        assert_eq!(ints_in_text("There are 4 boxes with 24 stickers in each."), vec![4, 24]);
        assert_eq!(ints_in_text("hold 1,000 and 23 more"), vec![1000, 23], "milliers");
    }

    #[test]
    fn reexec_audit_weigher_is_label_free_by_signature() {
        // Le seam : weight(task, answer) ne voit jamais le label ; il dépend du SEUL texte du candidat.
        let w = ReExecAudit::default();
        let task = json!({"text": "4 rows of 17, minus 23"});
        let good = json!({"value": "4 * 17 = 68 ; 68 - 23 = 45"});
        let bad = json!({"value": "the answer is probably 50"});
        assert!(w.weight(&task, &good) > w.weight(&task, &bad), "le candidat audité-juste pèse plus");
        assert!((w.weight(&task, &bad) - W_FLOOR).abs() < 1e-12, "sans équation → plancher");
    }

    #[test]
    fn determinism_same_input_same_score() {
        let cot = "\\( 12 \\times 3 = 36 \\), then 36 + 4 = 40";
        assert_eq!(audit_candidate(&[12, 3, 4], cot), audit_candidate(&[12, 3, 4], cot));
    }

    // ── négatifs / moins unaire (revue HB.3) ──

    #[test]
    fn first_int_reads_attached_negative_sign() {
        assert_eq!(first_int(" -5 remaining"), Some(-5), "signe collé → négatif");
        assert_eq!(first_int(" 45 left"), Some(45));
        assert_eq!(first_int("68 - 23"), Some(68), "premier nombre");
        assert_eq!(first_int("5 - 3"), Some(5), "le '-' séparé n'est pas un signe ici");
        assert_eq!(first_int("3.5 apples"), None, "décimal → None");
    }

    #[test]
    fn ints_in_text_reads_negative_literals() {
        assert_eq!(ints_in_text("a change of -5 then 10 more"), vec![-5, 10]);
        assert_eq!(ints_in_text("17 - 23 difference"), vec![17, 23], "moins binaire séparé → positifs");
    }

    #[test]
    fn audit_handles_unary_minus_and_negative_rhs() {
        // Régression (revue HIGH) : « -4 + 3 = -1 » DOIT s'auditer comme un HIT, pas un faux MISS.
        let a = audit_candidate(&[], "-4 + 3 = -1");
        assert_eq!(a, AuditScore { h2: 2, m: 0, e: 1 }, "moins unaire au début + RHS négatif → hit");
        // « 10 - 15 = -5 » : membre droit négatif lu correctement.
        let b = audit_candidate(&[10, 15], "10 - 15 = -5");
        assert_eq!(b, AuditScore { h2: 2, m: 0, e: 1 }, "RHS négatif (signe collé)");
        // « 23 - 68 = -45 » (modèle ayant inversé l'ordre) : calcul juste, audité hit.
        let c = audit_candidate(&[23, 68], "so 23 - 68 = -45");
        assert_eq!(c.m, 0, "calcul négatif juste n'est pas un faux MISS : {c:?}");
    }

    #[test]
    fn audit_still_ignores_trivial_x_minus_x() {
        // Le correctif du moins unaire ne doit PAS ré-ouvrir le faux parse « 4 - 4 » → « - 4 ».
        assert_eq!(audit_candidate(&[], "4 - 4 = 0"), AuditScore { h2: 0, m: 0, e: 0 }, "identité triviale ignorée");
        assert_eq!(audit_candidate(&[], "7 - 7 = 1"), AuditScore { h2: 0, m: 0, e: 0 }, "x-x trivial, pas un faux MISS");
    }
}
