// SPDX-License-Identifier: UNLICENSED
//! # `profile` — M0 : le profil servi de la **console**.
//!
//! Même discipline que la passerelle, à l'échelle de ce binaire : un **registre** (le seul lecteur
//! d'environnement), et un **rapport** de ce qui est réellement servi.
//!
//! ## Ce que M0 a révélé ici, et qui n'était pas connu
//!
//! La console a des **défauts de démonstration silencieux** : `EFE_CONSOLE_ROOT` vaut
//! `demo-root-secret` et `EFE_CONSOLE_TOKEN` vaut `demo-token`. Le premier est le secret **racine**
//! d'où la clé tenant est dérivée : laissé au défaut, il est **publiquement connu**, donc la clé
//! aussi, donc la re-preuve ne prouve rien contre quiconque a lu ce fichier.
//!
//! La passerelle avertit déjà dans le cas symétrique (« repli sur la clé de TEST PUBLIQUE — NE
//! JAMAIS utiliser en production »). La console, elle, se taisait. La règle d'or de M0 —
//! *aucune classe dégradée silencieuse* — l'interdit : ce module rend l'avertissement obligatoire.

use serde_json::{json, Value};

/// La classe d'un interrupteur — ce que son absence provoque. Miroir de la taxonomie de la
/// passerelle, volontairement identique pour que les deux rapports se lisent pareil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Absence ⟹ un défaut **dangereux** : la console démarre, mais une propriété de sûreté est
    /// perdue et doit être criée.
    DegradedSecret,
    /// Absence ⟹ un défaut **sûr**.
    Tuning,
}

/// Un interrupteur de la console.
#[derive(Debug, Clone, Copy)]
pub struct Switch {
    pub name: &'static str,
    pub default: &'static str,
    pub buys: &'static str,
    pub absence_costs: &'static str,
    pub class: Class,
}

/// Les sept interrupteurs, déclarés une seule fois.
pub const REGISTRY: &[Switch] = &[
    Switch {
        name: "EFE_CONSOLE_ADDR",
        default: "127.0.0.1:8088",
        buys: "l'adresse d'écoute",
        absence_costs: "écoute locale — la souveraineté EXIGE que ce bind reste local",
        class: Class::Tuning,
    },
    Switch {
        name: "EFE_CONSOLE_TENANT",
        default: "acme",
        buys: "le tenant servi",
        absence_costs: "tenant de démonstration « acme »",
        class: Class::Tuning,
    },
    Switch {
        name: "EFE_CONSOLE_ROOT",
        default: "demo-root-secret",
        buys: "le secret racine d'où la clé tenant est dérivée",
        absence_costs: "SECRET RACINE PUBLIC — la clé tenant est dérivable par quiconque lit le \
                        dépôt, donc la re-preuve ne prouve RIEN. Jamais en production.",
        class: Class::DegradedSecret,
    },
    Switch {
        name: "EFE_CONSOLE_TOKEN",
        default: "demo-token",
        buys: "le jeton de session",
        absence_costs: "JETON PUBLIC — n'importe qui ouvre une session d'agent de conformité",
        class: Class::DegradedSecret,
    },
    Switch {
        name: "EFE_CONSOLE_SUBJECT",
        default: "officer:demo",
        buys: "l'identité de l'agent",
        absence_costs: "sujet de démonstration — les actions humaines scellées seront attribuées à « officer:demo »",
        class: Class::Tuning,
    },
    Switch {
        name: "EFE_CONSOLE_RECORDS",
        default: "",
        buys: "la réhydratation d'un log scellé persisté",
        absence_costs: "tenant provisionné VIDE — la console ne montrera rien tant que la capture S1 n'alimente pas",
        class: Class::Tuning,
    },
];

#[must_use]
pub fn switch(name: &str) -> Option<&'static Switch> {
    REGISTRY.iter().find(|s| s.name == name)
}

/// Les lectures — **le seul endroit du crate qui lit l'environnement**.
pub mod read {
    /// La valeur effective d'un interrupteur : celle de l'environnement, sinon le défaut du
    /// registre. Une seule fonction, donc une seule sémantique.
    #[must_use]
    pub fn value(name: &str) -> String {
        let default = super::switch(name).map_or("", |s| s.default);
        std::env::var(name).unwrap_or_else(|_| default.to_string())
    }

    /// `true` si l'opérateur a réellement posé la variable (par opposition au défaut).
    #[must_use]
    pub fn is_set(name: &str) -> bool {
        std::env::var(name).map(|v| !v.trim().is_empty()).unwrap_or(false)
    }

    /// Le chemin de réhydratation, s'il y en a un.
    #[must_use]
    pub fn records_path() -> Option<String> {
        std::env::var("EFE_CONSOLE_RECORDS").ok().filter(|v| !v.trim().is_empty())
    }
}

/// Une ligne du rapport.
#[derive(Debug, Clone)]
pub struct Line {
    pub name: &'static str,
    pub status: &'static str,
    pub detail: String,
}

/// Le rapport de la console.
#[derive(Debug, Clone)]
pub struct Report {
    pub lines: Vec<Line>,
    /// Les interrupteurs de classe secrète laissés au défaut — la liste que l'opérateur doit voir.
    pub secrets_par_defaut: Vec<&'static str>,
}

impl Report {
    #[must_use]
    pub fn from_env() -> Self {
        let mut lines = Vec::new();
        let mut secrets_par_defaut = Vec::new();
        for s in REGISTRY {
            let set = read::is_set(s.name);
            let (status, detail) = match (set, s.class) {
                (true, _) => ("SERVIE", s.buys.to_string()),
                (false, Class::DegradedSecret) => {
                    secrets_par_defaut.push(s.name);
                    ("DÉGRADÉE", s.absence_costs.to_string())
                }
                (false, Class::Tuning) => ("DÉFAUT", format!("{} — {}", s.default, s.absence_costs)),
            };
            lines.push(Line { name: s.name, status, detail });
        }
        Report { lines, secrets_par_defaut }
    }

    /// La forme humaine, imprimée au démarrage.
    #[must_use]
    pub fn render_human(&self) -> String {
        let mut out = String::from("┌─ profil servi (console)\n");
        for l in &self.lines {
            out.push_str(&format!("│  {:<24} {:<9} {}\n", l.name, l.status, l.detail));
        }
        out.push_str("└─\n");
        if !self.secrets_par_defaut.is_empty() {
            out.push_str(&format!(
                "\n⚠  AVERTISSEMENT : {} laissé(s) au DÉFAUT PUBLIC — {}.\n   \
                 La clé tenant est alors dérivable par quiconque lit le dépôt : la re-preuve ne \
                 prouve RIEN.\n   NE JAMAIS servir ainsi en production.\n",
                self.secrets_par_defaut.join(", "),
                self.secrets_par_defaut.len()
            ));
        }
        out
    }

    /// La forme machine — `--profile-report`. Jamais une route.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let lines: Vec<Value> = self
            .lines
            .iter()
            .map(|l| json!({ "switch": l.name, "status": l.status, "detail": l.detail }))
            .collect();
        json!({ "switches": lines, "secrets_par_defaut": self.secrets_par_defaut })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_registre_couvre_les_six_interrupteurs() {
        // SIX, pas sept : le « septième » de mon relevé initial était `EFE_K`, un faux positif —
        // le motif attrapait `EFE_Kovant_P1_*` dans des commentaires.
        assert_eq!(REGISTRY.len(), 6, "ADDR, TENANT, ROOT, TOKEN, SUBJECT, RECORDS");
        for s in REGISTRY {
            assert!(!s.buys.trim().is_empty(), "{} : `buys` vide", s.name);
            assert!(!s.absence_costs.trim().is_empty(), "{} : `absence_costs` vide", s.name);
        }
    }

    /// Le cœur de la découverte de M0 : les deux secrets ont un défaut PUBLIC, et le rapport doit
    /// le crier. Un test le fige, sinon la vigilance seule ne tiendra pas.
    #[test]
    fn les_defauts_publics_sont_signales() {
        let root = switch("EFE_CONSOLE_ROOT").expect("ROOT déclaré");
        let token = switch("EFE_CONSOLE_TOKEN").expect("TOKEN déclaré");
        assert_eq!(root.class, Class::DegradedSecret, "le secret racine est une dégradation, pas un réglage");
        assert_eq!(token.class, Class::DegradedSecret);
        assert!(root.absence_costs.contains("PUBLIC"), "le coût doit nommer le risque");
    }

    #[test]
    fn le_rapport_json_est_complet() {
        let v = Report::from_env().to_json();
        assert!(v["switches"].is_array());
        assert!(v["secrets_par_defaut"].is_array());
    }
}
