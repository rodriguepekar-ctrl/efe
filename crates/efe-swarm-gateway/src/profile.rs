// SPDX-License-Identifier: UNLICENSED
//! # `profile` — M0 : **le profil servi**.
//!
//! ## Le principe
//!
//! *Le binaire dit ce qu'il sert ; le profil déclare ce qu'on attend ; un vérificateur compare les
//! deux — et aucun des trois ne peut diverger sans casser un test.*
//!
//! Un fichier `.env` documenté aurait été un **second dépôt de vérité**, et il aurait dérivé au
//! premier interrupteur ajouté. Ce module est donc à la fois le **registre** (ce qui existe), le
//! **lecteur** (le seul du crate) et le **rapport** (ce qui est réellement servi).
//!
//! ## Ce que ce module ne réinvente pas
//!
//! La discipline existait déjà : `exit 78` sur master invalide, `frontier_boot_guard` en fonction
//! pure à quatre préconditions, un préflight qui refuse d'imprimer un « OK » lisse quand la capacité
//! PII manque. M0 **généralise** ; il n'invente pas. Ce qui manquait était la **complétude** et la
//! **lisibilité machine** : les `println!("→ …")` étaient une auto-description en prose que rien ne
//! contraignait.
//!
//! ## L'invariant, tenu par un test
//!
//! `tests/profile_no_raw_env.rs` (T1) échoue si un `std::env::var` apparaît hors de ce fichier, en
//! dehors des blocs `#[cfg(test)]`. C'est ce qui rend le registre réellement unique — sans lui, il
//! ne serait qu'une documentation parallèle, c'est-à-dire le défaut qu'il existe pour supprimer.

use serde_json::{json, Value};

// ═════════════════════════════════════════════════════════════════════════════════════════════
//  1 · La taxonomie
// ═════════════════════════════════════════════════════════════════════════════════════════════

/// La classe d'un interrupteur — ce que son **absence** provoque.
///
/// La règle d'or du module : **aucune classe [`Degraded`](Class::Degraded) silencieuse**. Une
/// capacité éteinte se dit à trois endroits — le rapport au démarrage, la surface servie, et le
/// reçu. Le précédent existait déjà dans le dépôt : *racine vide ⟺ aucun scellement*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Invalidité ⟹ **refus de démarrer** (`exit 78`, `EX_CONFIG`).
    ///
    /// Deux formes existent dans ce binaire, et il fallait les distinguer plutôt que de laisser la
    /// classe vide : `EFE_NANO_AUDIT_HMAC_MASTER` est **dégradée si absente** (repli sur la clé
    /// publique) mais **fatale si fournie et invalide** (< 32 octets : chaque scellement
    /// échouerait 500) ; `EFE_LLM_FRONTIER_ENABLED=true` est **fatale si l'une des quatre
    /// préconditions manque**. D'où le drapeau [`Switch::fatal_if_invalid`] plutôt qu'une classe
    /// à part : la même variable peut relever de deux régimes selon sa valeur.
    Fatal,
    /// Absence ⟹ démarre, mais une **capacité s'éteint** — et le rapport le dit en clair.
    Degraded,
    /// Absence ⟹ un défaut **sûr**, sans conséquence de sûreté. Le silence est acceptable.
    Tuning,
}

impl Class {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Fatal => "fatale",
            Class::Degraded => "dégradée",
            Class::Tuning => "réglage",
        }
    }
}

/// Un interrupteur, déclaré **une seule fois**.
///
/// `absence_costs` n'est pas de la documentation d'agrément : c'est ce que le rapport imprime quand
/// la capacité est éteinte. Écrire « — » y serait un mensonge par omission.
#[derive(Debug, Clone, Copy)]
pub struct Switch {
    /// Le nom de la variable d'environnement.
    pub name: &'static str,
    /// Ce que sa présence **achète**.
    pub buys: &'static str,
    /// Ce que son absence **coûte** — imprimé tel quel dans le rapport.
    pub absence_costs: &'static str,
    /// La classe (voir [`Class`]).
    pub class: Class,
    /// `true` ⟹ **fournie mais invalide ⟹ `exit 78`**, même si son absence n'est que dégradante.
    ///
    /// Sans ce drapeau la taxonomie serait morte : `Class::Fatal` n'aurait aucun porteur, alors
    /// que le binaire refuse bel et bien de démarrer dans deux cas. Une classe déclarée et jamais
    /// employée fait paraître la conception plus rigoureuse que le code.
    pub fatal_if_invalid: bool,
}

// ═════════════════════════════════════════════════════════════════════════════════════════════
//  2 · Le registre — source unique de vérité
// ═════════════════════════════════════════════════════════════════════════════════════════════

/// Tous les interrupteurs de la passerelle.
///
/// **Ajouter une lecture d'environnement sans l'ajouter ici casse T1.** C'est voulu : un
/// interrupteur non déclaré est un interrupteur que l'opérateur découvrira en production.
pub const REGISTRY: &[Switch] = &[
    // ── Réseau et capacité ────────────────────────────────────────────────────────────────────
    Switch {
        name: "EFE_SWARM_GATEWAY_ADDR",
        buys: "l'adresse d'écoute",
        absence_costs: "écoute sur 0.0.0.0:8080",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_GATEWAY_MAX_INFLIGHT_RUNS",
        buys: "le plafond de runs simultanés",
        absence_costs: "défaut max(4, cœurs) — backpressure honnête en 429 au-delà",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_GATEWAY_MAX_INFLIGHT_PER_TENANT",
        buys: "l'équité entre tenants",
        absence_costs: "défaut max(1, global−1) : aucun tenant ne prend le dernier permit",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_GATEWAY_MAX_WIDTH",
        buys: "le plafond serveur de largeur",
        absence_costs: "défaut 32 — un client ne dimensionne pas la machine d'autrui",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_GATEWAY_MAX_DEADLINE_MS",
        buys: "le plafond serveur de deadline",
        absence_costs: "défaut 300 000 ms (5 min)",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_SWARM_WIDTH",
        buys: "une largeur fixe par défaut",
        absence_costs: "largeur automatique (cœurs / backend déclaré)",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    // ── Provenance : le cœur ──────────────────────────────────────────────────────────────────
    Switch {
        name: "EFE_NANO_AUDIT_HMAC_MASTER",
        buys: "une provenance NON-FORGEABLE (clé tenant dérivée d'un master serveur)",
        absence_costs: "repli sur la clé de TEST PUBLIQUE — le reçu devient FORGEABLE, \
                        `verify` cesse d'être fiable. Jamais en production.",
        class: Class::Degraded,
        // FOURNIE mais < 32 octets ⟹ `exit 78` : booter « vert » puis échouer 500 à chaque
        // scellement serait exactement le vert malhonnête que ce dépôt combat.
        fatal_if_invalid: true,
    },
    Switch {
        name: "EFE_SWARM_ANCHOR_LOG",
        buys: "le journal d'ancres — une troncature devient détectable SANS CLÉ",
        absence_costs: "aucune ancre publiée : un bundle tronqué se re-prouve parfaitement",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_SWARM_TSA_URL",
        buys: "l'ancre TIERCE — un horodateur RFC 3161 atteste que la racine existait avant T, avec \
               SA clé. Le reçu passe de « tamper-evident pour un détenteur de clé » à OPPOSABLE au \
               détenteur lui-même",
        absence_costs: "l'« ailleurs » de l'ancre reste NOTRE fichier, remis par nous : il détecte \
                        une troncature, il ne dit rien à qui doute de nous. ⚠ Poser cette URL fait \
                        DÉPENDRE la route de bundle d'un tiers : horodateur muet ⟹ export en échec",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_SWARM_TSA_DIR",
        buys: "le répertoire où déposer les jetons d'horodatage et leurs preuves",
        absence_costs: "sans objet si aucun horodateur n'est déclaré ; requis avec EFE_SWARM_TSA_URL",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_SWARM_BUNDLE_ENABLED",
        buys: "la route d'export `GET /v1/swarm/bundle/:id`",
        absence_costs: "route NON montée (404) — aucun export vers un vérifieur tiers",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_SWARM_BUNDLE_SIGNING_SEED",
        buys: "l'attestation ed25519 de la racine du bundle",
        absence_costs: "bundle servi NON attesté — le champ est absent, jamais vide",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "DATABASE_URL",
        buys: "un store DURABLE (Postgres, write-once)",
        absence_costs: "store en mémoire : la provenance ne survit pas au redémarrage",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    // ── Souveraineté et frontière ─────────────────────────────────────────────────────────────
    Switch {
        name: "EFE_LLM_FRONTIER_ENABLED",
        buys: "les appels aux modèles frontière (egress PAYANT vers un tiers)",
        absence_costs: "aucun appel frontière — `/v1/llm/*` inactif",
        class: Class::Fatal,
        // `=true` sans l'une des quatre préconditions (clé, OIDC, store durable, capacité PII)
        // ⟹ `exit 78`. Un egress PAYANT vers un tiers ne dégrade jamais en silence.
        fatal_if_invalid: true,
    },
    // P1 — les clés de fournisseur. Elles ne sont pas lues par ce crate pour APPELER quoi que ce
    // soit (c'est `efe-coordinator` qui le fait) mais leur présence décide si un coordinator est
    // câblé, donc si la frontière est servie. Sans elles au registre, `--profile-report` annonçait
    // « frontière SERVIE » là où le démarrage réel dit « REFUSÉE » : le même écart déclaré/servi,
    // un cran plus bas. Ce sont des SECRETS : `profile-check.sh` §2 interdit déjà qu'un profil les
    // pose, et le rapport ne rend jamais leur valeur — seulement leur présence.
    Switch {
        name: "ANTHROPIC_API_KEY",
        buys: "un fournisseur frontière Claude",
        absence_costs: "aucun fournisseur Claude — la frontière reste REFUSÉE si aucun autre n'est posé",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "OPENAI_API_KEY",
        buys: "un fournisseur frontière GPT",
        absence_costs: "aucun fournisseur GPT — la frontière reste REFUSÉE si aucun autre n'est posé",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "GOOGLE_AI_KEY",
        buys: "un fournisseur frontière Gemini",
        absence_costs: "aucun fournisseur Gemini — la frontière reste REFUSÉE si aucun autre n'est posé",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_LLM_TENANT_ALLOWLIST",
        buys: "l'allowlist d'egress par tenant",
        absence_costs: "registre VIDE — fail-closed total, aucun egress autorisé",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_LLM_ACCEPT_CLEARTEXT_EGRESS",
        buys: "la reconnaissance EXPLICITE que les prompts sortent en clair",
        absence_costs: "sans la feature `pii`, la frontière REFUSE de démarrer",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_LLM_MAX_BUDGET_CENTS",
        buys: "le plafond serveur de dépense par requête",
        absence_costs: "défaut 10 000 c — un client ne dimensionne pas la dépense d'autrui",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_PII_DEFAULT_POLICY",
        buys: "la politique de caviardage par défaut",
        absence_costs: "politique `Off` — aucun caviardage appliqué",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_SLM_REFORMULATOR_URL",
        buys: "le relais SLM local entre modèles frontière",
        absence_costs: "aucune médiation SLM — les dialectes ne sont pas traduits",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    // ── Capture (M2) ──────────────────────────────────────────────────────────────────────────
    Switch {
        name: "EFE_SWARM_CAPTURE_POLICY",
        buys: "l'observation NON INTRUSIVE : un échec de persistance ne fait plus échouer la \
               requête du client — la réponse part, et la brèche est dite en trois endroits",
        absence_costs: "posture BLOQUANTE (défaut) — un échec de persistance fait échouer la \
                        requête, et le client PERD une réponse pourtant calculée. Sûr pour la \
                        preuve, coûteux pour l'adoption : c'est l'engagement d'architecture qu'un \
                        client refuse de prendre pour une fonction de conformité",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    // ── Pont réglementaire (M5) ───────────────────────────────────────────────────────────────
    Switch {
        name: "EFE_SWARM_REG_EVIDENCE_DIR",
        buys: "le pont vers le REGISTRE : chaque run produit son dossier d'évidence \
               réglementaire, sans geste manuel",
        absence_costs: "aucun dossier réglementaire — le reçu reste re-prouvable, mais rien ne le \
                        met en regard des exigences (EU AI Act art. 12, GDPR, CSDDD/LkSG). Le \
                        travail de mise en correspondance reste manuel, à faire après coup",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_SWARM_REG_FRAMEWORK",
        buys: "le cadre visé par les dossiers dérivés",
        absence_costs: "aucun cadre choisi ⟹ le pont reste ÉTEINT même si un dossier de sortie est \
                        posé. On ne devine jamais un cadre : se tromper produit un dossier qui a \
                        l'air correct et répond à la mauvaise question",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    // ── Routage ───────────────────────────────────────────────────────────────────────────────
    Switch {
        name: "EFE_SWARM_ROUTER_ENDPOINT",
        buys: "le point de terminaison du routeur de modèles",
        absence_costs: "mode réseau indisponible",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_SWARM_ROUTER_MODELS",
        buys: "la table famille→modèle",
        absence_costs: "table vide — tout en passthrough",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    // ── Authentification ──────────────────────────────────────────────────────────────────────
    Switch {
        name: "EFE_OIDC_PUBLIC_KEY_PEM",
        buys: "l'authentification OIDC (clé PUBLIQUE, aucun secret manipulé)",
        absence_costs: "extraction des rôles par en-têtes `x-efe-*` — dev/standalone uniquement",
        class: Class::Degraded,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_OIDC_ISSUER_URL",
        buys: "la validation de l'émetteur du jeton",
        absence_costs: "émetteur non vérifié",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_OIDC_AUDIENCE",
        buys: "la validation d'audience (chemin essaim)",
        absence_costs: "audience non vérifiée",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
    Switch {
        name: "EFE_OIDC_LLM_AUDIENCE",
        buys: "la validation d'audience (chemin frontière)",
        absence_costs: "défaut `efe-llm`",
        class: Class::Tuning,
        fatal_if_invalid: false,
    },
];

/// Les interrupteurs **booléens stricts** : seule la chaîne exacte `"true"` active.
///
/// Cette liste existe parce que le contraire est un piège silencieux. `EFE_LLM_FRONTIER_ENABLED=1`
/// n'active rien : `flag_of` exige `"true"`. Le déploiement démarre, la capacité reste éteinte, et
/// le rapport annonce « DÉGRADÉE » — ce qui est exact, mais laisse croire à l'opérateur qu'il a
/// oublié la variable alors qu'il l'a posée. Il cherchera au mauvais endroit, potentiellement
/// longtemps.
///
/// Le dépôt savait : `main.rs` portait une fonction qui imprimait précisément cet avertissement,
/// documentée « source UNIQUE de vérité (interdit tout drift) ». **Elle n'était appelée par
/// personne**, et le diagnostic était donc perdu. Il est ici, pour les trois drapeaux au lieu d'un.
///
/// Le test `les_flags_stricts_sont_exactement_ceux_lus_par_flag` interdit à cette liste de dériver
/// des appels réels à `read::flag`.
pub const FLAGS_STRICTS: &[&str] =
    &["EFE_LLM_ACCEPT_CLEARTEXT_EGRESS", "EFE_LLM_FRONTIER_ENABLED", "EFE_SWARM_BUNDLE_ENABLED"];

/// Les écritures qui expriment sans ambiguïté « éteint ». Y répondre par un avertissement serait
/// reprocher à l'opérateur d'avoir obtenu ce qu'il demandait.
const NEGATIONS: &[&str] = &["false", "0", "no", "off", "n"];

/// Une valeur **fournie mais refusée** — le cas que ni « présente » ni « absente » ne décrit, et
/// le seul où l'opérateur a agi sans que son geste ait le moindre effet.
///
/// N'imprimer la valeur reçue est sûr **uniquement** parce que le champ est restreint aux drapeaux
/// booléens de [`FLAGS_STRICTS`] : aucun d'eux ne porte de secret. Étendre ce mécanisme à un
/// interrupteur porteur de secret exigerait de masquer `fourni`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalide {
    pub name: &'static str,
    /// La valeur que l'opérateur a réellement posée.
    pub fourni: String,
    pub attendu: &'static str,
    /// Ce qui se passe en conséquence — repris tel quel du registre, jamais réécrit.
    pub consequence: &'static str,
}

/// Les interrupteurs **énumérés** : leur valeur doit appartenir à un ensemble fini.
///
/// `FLAGS_STRICTS` couvrait les booléens (`"true"` et rien d'autre). M2 a introduit le premier
/// interrupteur à plusieurs valeurs, et le piège y est identique : `EFE_SWARM_CAPTURE_POLICY=observe`
/// retombe silencieusement sur le défaut, et l'opérateur croit avoir choisi une posture qu'il n'a
/// pas. Généraliser le mécanisme plutôt que d'ajouter un cas particulier, c'est ce qui évite que le
/// prochain interrupteur énuméré rate le filet à son tour.
pub const ENUMS: &[(&str, &[&str])] = &[
    ("EFE_SWARM_CAPTURE_POLICY", &["blocking", "observation"]),
    // M5 : la liste vit dans `reg_pont`, à côté du code qui l'interprète — la publier ici en dur
    // rouvrirait l'écart entre ce que le rapport annonce et ce que le binaire accepte.
    ("EFE_SWARM_REG_FRAMEWORK", crate::reg_pont::CADRES),
];

/// Les valeurs fournies mais refusées, depuis une table quelconque — un fichier de profil compris.
///
/// Fonction **pure**, donc un test de profil peut refuser qu'un `.env` livré contienne un footgun :
/// une faute de frappe versionnée se propagerait à chaque déploiement qui suit ce fichier.
#[must_use]
pub fn invalides_from_lookup(get: impl Fn(&str) -> Option<String>) -> Vec<Invalide> {
    FLAGS_STRICTS
        .iter()
        .filter_map(|name| {
            let v = get(name)?;
            let t = v.trim();
            // Absente ou vide : ce n'est pas une valeur refusée, c'est une absence. Le rapport la
            // traite déjà, et la confondre avec un footgun noierait le vrai signal.
            if t.is_empty() || t == "true" {
                return None;
            }
            // Une extinction DÉLIBÉRÉE n'est pas un footgun. `profiles/demo.env` écrit
            // `EFE_LLM_FRONTIER_ENABLED=false` : la capacité est éteinte, et c'est exactement ce
            // que l'opérateur a demandé. Crier ici reviendrait à reprocher à quelqu'un d'avoir
            // obtenu ce qu'il voulait — et un rapport qui crie à tort n'est plus lu du tout.
            //
            // Le critère n'est donc pas « la valeur est-elle acceptée ? » mais « le résultat
            // correspond-il à l'intention ? ». `false` → éteint : intention respectée. `1` →
            // éteint : intention trahie. La comparaison à `true` reste sensible à la casse, elle,
            // car `TRUE` est une TENTATIVE D'ACTIVATION, et c'est précisément le piège documenté.
            if NEGATIONS.contains(&t.to_ascii_lowercase().as_str()) {
                return None;
            }
            let sw = switch(name)?;
            Some(Invalide {
                name: sw.name,
                fourni: t.to_string(),
                attendu: "exactement `true` pour activer, ou `false` pour éteindre explicitement",
                consequence: sw.absence_costs,
            })
        })
        .chain(ENUMS.iter().filter_map(|(name, valeurs)| {
            let v = get(name)?;
            let t = v.trim();
            if t.is_empty() || valeurs.contains(&t) {
                return None;
            }
            let sw = switch(name)?;
            Some(Invalide {
                name: sw.name,
                fourni: t.to_string(),
                attendu: "une valeur de l'énumération déclarée",
                consequence: sw.absence_costs,
            })
        }))
        .collect()
}

/// Les valeurs fournies mais refusées dans l'environnement du process.
#[must_use]
pub fn invalides_from_env() -> Vec<Invalide> {
    invalides_from_lookup(|k| std::env::var(k).ok())
}

/// Cherche un interrupteur par nom. `None` ⟹ il n'est pas déclaré, donc T1 aurait dû mordre.
#[must_use]
pub fn switch(name: &str) -> Option<&'static Switch> {
    REGISTRY.iter().find(|s| s.name == name)
}

// ═════════════════════════════════════════════════════════════════════════════════════════════
//  3 · Les lectures — le SEUL endroit du crate qui lit l'environnement
// ═════════════════════════════════════════════════════════════════════════════════════════════

/// Les lectures brutes. **Toute** la sémantique (défaut, bornage, convention `"true"`) vit ici, et
/// nulle part ailleurs : c'est ce qui garantit que le rapport décrit le comportement réel plutôt
/// qu'une intention parallèle.
pub mod read {
    /// Convention du dépôt : **seule** la chaîne `"true"` active. Une valeur approchante
    /// (`"1"`, `"yes"`, `"True"`) n'active pas — et le binaire le dit au démarrage plutôt que de
    /// deviner ce que l'opérateur voulait.
    ///
    /// **Pure** : c'est la MÊME fonction qui sert à lire l'environnement du process et à résoudre
    /// un fichier de profil. Deux définitions dériveraient, et le rapport finirait par décrire
    /// autre chose que le comportement.
    #[must_use]
    pub fn flag_of(v: Option<&str>) -> bool {
        v == Some("true")
    }

    /// **Pure** : présence = valeur non vide après trim.
    #[must_use]
    pub fn present_of(v: Option<&str>) -> bool {
        v.map(|s| !s.trim().is_empty()).unwrap_or(false)
    }

    /// **Le goulot d'étranglement de M0.** Les trois primitives ci-dessous sont les SEULS endroits
    /// du binaire qui touchent `std::env::var` — `profile_no_raw_env.rs` (T1) l'impose pour tout le
    /// reste du crate. Chacune passe d'abord par ici.
    ///
    /// Une variable lue sans être déclarée au [`REGISTRY`] n'apparaîtrait dans **aucun** rapport :
    /// ce serait un levier invisible, que l'opérateur ne pourrait ni voir ni actionner sciemment.
    /// Comme les trois primitives reçoivent leur nom en paramètre — aucun littéral ne se cache
    /// ailleurs —, il suffit de le vérifier ici pour que le trou soit fermé partout, y compris
    /// pour un nom construit à l'exécution qu'aucun balayage de source n'attraperait.
    ///
    /// L'assertion est de **debug** : elle tombe pendant les tests et en développement, et ne coûte
    /// rien en production. Un profil est lu au démarrage, pas dans une boucle chaude.
    #[track_caller]
    fn declare(name: &str) {
        debug_assert!(
            super::switch(name).is_some(),
            "`{name}` est lu par le binaire mais N'EST PAS déclaré au registre : il n'apparaîtra \
             dans aucun rapport de profil. Ajouter son `Switch` — avec ce qu'il achète et ce que \
             son absence coûte — plutôt que de le lire en douce."
        );
    }

    fn flag(name: &str) -> bool {
        declare(name);
        flag_of(std::env::var(name).ok().as_deref())
    }

    fn raw(name: &str) -> Option<String> {
        declare(name);
        std::env::var(name).ok()
    }

    /// La valeur brute d'un interrupteur — utilisée par le rapport pour dire « présent » sans
    /// jamais imprimer un secret.
    #[must_use]
    pub fn present(name: &str) -> bool {
        declare(name);
        present_of(std::env::var(name).ok().as_deref())
    }

    // ── Réseau et capacité ────────────────────────────────────────────────────────────────────

    #[must_use]
    pub fn gateway_addr() -> String {
        raw("EFE_SWARM_GATEWAY_ADDR").unwrap_or_else(|| "0.0.0.0:8080".to_string())
    }

    #[must_use]
    pub fn max_inflight_runs() -> usize {
        raw("EFE_GATEWAY_MAX_INFLIGHT_RUNS")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).max(4))
            .max(1)
    }

    #[must_use]
    pub fn max_inflight_per_tenant(global: usize) -> usize {
        raw("EFE_GATEWAY_MAX_INFLIGHT_PER_TENANT")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or_else(|| global.saturating_sub(1).max(1))
            .clamp(1, global.max(1))
    }

    #[must_use]
    pub fn max_width() -> usize {
        raw("EFE_GATEWAY_MAX_WIDTH").and_then(|s| s.parse::<usize>().ok()).unwrap_or(32).max(1)
    }

    /// Plancher repris de `dto.rs` : un deadline sous 50 ms ne laisse pas le temps d'un aller-retour.
    #[must_use]
    pub fn max_deadline_ms(min: u64) -> u64 {
        raw("EFE_GATEWAY_MAX_DEADLINE_MS")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(300_000)
            .max(min)
    }

    /// `None` ⟹ largeur automatique.
    #[must_use]
    pub fn swarm_width() -> Option<usize> {
        raw("EFE_SWARM_WIDTH").and_then(|s| s.parse::<usize>().ok())
    }

    // ── Provenance ────────────────────────────────────────────────────────────────────────────

    /// **Présence** du master, jamais sa valeur : un secret ne traverse pas cette fonction.
    #[must_use]
    pub fn master_present() -> bool {
        present("EFE_NANO_AUDIT_HMAC_MASTER")
    }

    #[must_use]
    pub fn anchor_log() -> Option<String> {
        raw("EFE_SWARM_ANCHOR_LOG")
    }

    /// **M9** — l'URL de l'horodateur tiers, et où déposer ses jetons.
    #[must_use]
    pub fn tsa_url() -> Option<String> {
        raw("EFE_SWARM_TSA_URL")
    }

    #[must_use]
    pub fn tsa_dir() -> Option<String> {
        raw("EFE_SWARM_TSA_DIR")
    }

    #[must_use]
    pub fn bundle_enabled() -> bool {
        flag("EFE_SWARM_BUNDLE_ENABLED")
    }

    #[must_use]
    pub fn bundle_signing_seed() -> Option<String> {
        raw("EFE_SWARM_BUNDLE_SIGNING_SEED")
    }

    #[must_use]
    pub fn database_url() -> Option<String> {
        raw("DATABASE_URL")
    }

    // ── Souveraineté et frontière ─────────────────────────────────────────────────────────────

    /// La valeur BRUTE : `main.rs` distingue « absente », « `true` » et « autre chose » — ce
    /// dernier cas méritant un avertissement plutôt qu'un silence.
    #[must_use]
    pub fn frontier_raw() -> Option<String> {
        raw("EFE_LLM_FRONTIER_ENABLED")
    }

    #[must_use]
    pub fn frontier_enabled() -> bool {
        flag("EFE_LLM_FRONTIER_ENABLED")
    }

    #[must_use]
    pub fn tenant_allowlist() -> Option<String> {
        raw("EFE_LLM_TENANT_ALLOWLIST")
    }

    /// **P1** — au moins un fournisseur frontière est-il configuré ?
    ///
    /// `coordinator_from_env()` enregistre chaque fournisseur dont la clé est présente et rend
    /// `None` si aucun ne l'est. Sans cette lecture, le rapport dérivé de l'environnement ne
    /// pouvait pas distinguer « frontière demandée et servie » de « demandée et sans fournisseur ».
    /// La valeur n'est jamais rendue — seulement le fait qu'elle existe.
    #[must_use]
    pub fn frontier_provider_present() -> bool {
        ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GOOGLE_AI_KEY"].iter().any(|k| present(k))
    }

    #[must_use]
    pub fn accept_cleartext_egress() -> bool {
        flag("EFE_LLM_ACCEPT_CLEARTEXT_EGRESS")
    }

    #[must_use]
    pub fn max_budget_cents() -> u32 {
        raw("EFE_LLM_MAX_BUDGET_CENTS")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(10_000)
            .max(1) // un plafond à 0 rendrait toute mission impossible.
    }

    #[must_use]
    pub fn pii_default_policy() -> Option<String> {
        raw("EFE_PII_DEFAULT_POLICY")
    }

    #[must_use]
    pub fn slm_reformulator_url() -> Option<String> {
        raw("EFE_SLM_REFORMULATOR_URL")
    }

    /// **M2** : la posture de capture. Toute valeur hors de l'énumération retombe sur le défaut
    /// sûr — et [`super::invalides_from_env`] la signale, pour que l'opérateur sache que son geste
    /// n'a rien fait.
    #[must_use]
    pub fn capture_policy() -> Option<String> {
        raw("EFE_SWARM_CAPTURE_POLICY")
    }

    /// **M5** : où écrire les dossiers réglementaires. Absent ⟹ pont éteint.
    #[must_use]
    pub fn reg_evidence_dir() -> Option<String> {
        raw("EFE_SWARM_REG_EVIDENCE_DIR")
    }

    /// **M5** : le cadre visé. Une valeur hors énumération est signalée par
    /// [`super::invalides_from_env`] et laisse le pont ÉTEINT — jamais un cadre deviné.
    #[must_use]
    pub fn reg_framework() -> Option<String> {
        raw("EFE_SWARM_REG_FRAMEWORK")
    }

    // ── Routage ───────────────────────────────────────────────────────────────────────────────

    #[must_use]
    pub fn router_endpoint() -> Option<String> {
        raw("EFE_SWARM_ROUTER_ENDPOINT")
    }

    #[must_use]
    pub fn router_models() -> Option<String> {
        raw("EFE_SWARM_ROUTER_MODELS")
    }

    // ── Authentification ──────────────────────────────────────────────────────────────────────

    #[must_use]
    pub fn oidc_public_key_pem() -> Option<String> {
        raw("EFE_OIDC_PUBLIC_KEY_PEM")
    }

    #[must_use]
    pub fn oidc_issuer_url() -> Option<String> {
        raw("EFE_OIDC_ISSUER_URL")
    }

    #[must_use]
    pub fn oidc_audience() -> Option<String> {
        raw("EFE_OIDC_AUDIENCE")
    }

    #[must_use]
    pub fn oidc_llm_audience() -> String {
        raw("EFE_OIDC_LLM_AUDIENCE").unwrap_or_else(|| "efe-llm".to_string())
    }
}

// ═════════════════════════════════════════════════════════════════════════════════════════════
//  4 · Le rapport — dérivé du même code qui décide
// ═════════════════════════════════════════════════════════════════════════════════════════════

/// L'état servi d'une capacité.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// La capacité est active et complète.
    Servie,
    /// La capacité est **éteinte ou amoindrie** — et le détail dit ce que cela coûte.
    Degradee,
    /// La capacité est refusée par configuration (et le binaire ne démarrera pas si elle est
    /// exigée ailleurs).
    Refusee,
}

impl Status {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Servie => "SERVIE",
            Status::Degradee => "DÉGRADÉE",
            Status::Refusee => "REFUSÉE",
        }
    }
}

/// Ce que le binaire a **réellement câblé**, observé APRÈS l'assemblage de l'état.
///
/// ## Pourquoi ce type existe — et c'est un correctif, pas un raffinement
///
/// [`Resolved`] décrit l'**intention** lue dans l'environnement ; `Observed` décrit le **service**.
/// Les deux divergent : `DATABASE_URL` présent ne prouve pas qu'un store durable soit branché, et
/// `EFE_LLM_FRONTIER_ENABLED=true` sans la feature `frontier-llm` ne sert **aucune** frontière.
///
/// Un rapport calculé depuis la seule intention porterait donc exactement le défaut que M0 existe
/// pour supprimer : *décrire le déclaré en croyant décrire le servi*. C'est ce qu'il faisait avant
/// ce type — et le rapport s'imprimait de surcroît AVANT le câblage.
#[derive(Debug, Clone, Copy, Default)]
pub struct Observed {
    /// La clé de provenance est-elle dérivée d'un master (et non la clé de test publique) ?
    pub provenance_non_forgeable: bool,
    pub anchor: bool,
    /// **M9** — l'ancre est-elle TIERCE (horodateur RFC 3161) ou seulement la nôtre ?
    ///
    /// Sans ce bit, « ancre : SERVIE » recouvrirait deux choses opposées : notre propre fichier,
    /// remis par nous, et l'attestation d'un tiers avec SA clé. Dire la même chose des deux
    /// cacherait très exactement ce que M9 apporte.
    pub anchor_tierce: bool,
    pub bundle: bool,
    pub attestation: bool,
    pub durable_store: bool,
    /// Un coordinator frontière est-il réellement câblé ?
    pub frontier: bool,
    /// La frontière était-elle **demandée** ? Demandée sans être câblée ⟹ [`Status::Refusee`].
    pub frontier_intended: bool,
    pub allowlist: bool,
    pub pii: bool,
    /// Le relais SLM est-il **réellement câblé** ? Il vit DANS le coordinator frontière : sans
    /// coordinator, l'URL est lue par ce rapport et consommée par personne.
    pub slm: bool,
    /// Une URL de relais a-t-elle été **posée** ? Posée sans câblage ⟹ [`Status::Refusee`], sur le
    /// même patron que la frontière.
    pub slm_intended: bool,
    /// **M5** : le pont réglementaire est-il réellement câblé sur l'état servi ?
    pub reg_pont: bool,
    /// **M2** : la posture d'observation est-elle réellement câblée sur l'état servi ?
    pub capture_observation: bool,
    /// Câblé — mais l'hôte est-il DANS le périmètre ? Câblé hors périmètre, la capacité fonctionne
    /// et la garantie de souveraineté est nulle : c'est une dégradation, pas une absence.
    pub slm_local: bool,
    pub oidc: bool,
    pub router: bool,
}

/// Le régime d'un rapport : ce qu'il décrit réellement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Calculé depuis l'environnement, **sans rien câbler** (`--profile-report`). Décrit ce que la
    /// configuration *demande*. Le dire évite qu'un lecteur y voie une garantie de service.
    Intention,
    /// Calculé depuis l'état assemblé, après câblage. Décrit ce qui est **servi**.
    Servi,
}

impl Mode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Intention => "intention",
            Mode::Servi => "servi",
        }
    }
}

/// Une ligne du rapport : une capacité, son état, et **ce que son absence coûte** si elle est
/// dégradée.
#[derive(Debug, Clone)]
pub struct CapabilityReport {
    pub capability: &'static str,
    pub status: Status,
    pub detail: String,
    /// L'interrupteur qui la gouverne, pour que l'opérateur sache quoi poser.
    pub governed_by: &'static str,
}

/// Le rapport complet — ce que le binaire **sert réellement**.
#[derive(Debug, Clone)]
pub struct Report {
    /// Ce que ce rapport décrit : l'intention, ou le service. Jamais implicite.
    pub mode: Mode,
    pub capabilities: Vec<CapabilityReport>,
    /// Les features compilées, qui ne sont pas des variables d'environnement mais changent
    /// autant le comportement servi.
    pub features: Vec<&'static str>,
    /// Les interrupteurs posés avec une valeur que le binaire **refuse**. Vide dans le cas normal ;
    /// non vide, c'est le renseignement le plus utile du rapport, car il désigne une action de
    /// l'opérateur restée sans effet.
    pub invalides: Vec<Invalide>,
}

/// Fabrique la ligne d'une capacité gouvernée par un interrupteur du registre.
fn line(governed_by: &'static str, capability: &'static str, on: bool) -> CapabilityReport {
    let sw = switch(governed_by);
    let (status, detail) = if on {
        (Status::Servie, sw.map(|s| s.buys.to_string()).unwrap_or_default())
    } else {
        (Status::Degradee, sw.map(|s| s.absence_costs.to_string()).unwrap_or_default())
    };
    CapabilityReport { capability, status, detail, governed_by }
}

/// La ligne d'une capacité **demandée mais non servie** — le seul cas qui mérite `REFUSÉE`.
///
/// L'exemple concret : `EFE_LLM_FRONTIER_ENABLED=true` sans la feature `frontier-llm` compilée, ou
/// sans clé provider. Annoncer « DÉGRADÉE » y serait trop doux : l'opérateur a **demandé** la
/// capacité et ne l'aura pas. Il doit lire un refus, pas une nuance.
/// La ligne du **relais SLM**, seule capacité à connaître trois issues distinctes.
///
/// * URL posée, mais rien ne la consomme ⟹ [`Status::Refusee`]. Le relais vit dans le coordinator
///   frontière : feature `frontier-llm` absente, ou `EFE_LLM_FRONTIER_ENABLED` non activé, et l'URL
///   est alors lue par ce rapport et par personne d'autre. La dire « SERVIE » était le défaut.
/// * Câblé hors périmètre ⟹ dégradée, motif en capitales.
/// * Sinon, la ligne ordinaire : servie si câblée, absente sinon.
fn slm_line(o: &Observed) -> CapabilityReport {
    const VAR: &str = "EFE_SLM_REFORMULATOR_URL";
    if o.slm_intended && !o.slm {
        line_refusee(
            VAR,
            "relais SLM",
            "une URL est posée, mais aucun client ne la consomme : le relais vit DANS le coordinator \
             frontière, et celui-ci n'est pas câblé (feature `frontier-llm` absente, ou \
             `EFE_LLM_FRONTIER_ENABLED` non activé). Aucun prompt ne passera par ce relais.",
        )
    } else if o.slm && !o.slm_local {
        line_hors_perimetre(
            VAR,
            "relais SLM",
            "le relais est câblé et fonctionne, mais son hôte n'est PAS la machine locale : chaque \
             prompt reformulé quitte le périmètre, et cet egress n'est ni gardé par l'allowlist \
             tenant ni décompté du budget. La souveraineté n'est pas garantie sur ce chemin.",
        )
    } else {
        line(VAR, "relais SLM", o.slm)
    }
}

/// La ligne d'une capacité **servie, mais hors du périmètre de souveraineté**.
///
/// Ni « absente » ni « servie » ne décrit ce cas, et les confondre est exactement le défaut que P1
/// ferme : le relais SLM pointé sur un hôte distant **fonctionne** — les prompts sont bien
/// reformulés — mais ils quittent la machine, et leur egress n'est ni gardé ni décompté. Annoncer
/// « SERVIE » serait vrai sur la fonction et faux sur la promesse.
///
/// Le statut reste [`Status::Degradee`] plutôt qu'une quatrième valeur : pour le lecteur, « je ne
/// peux pas m'appuyer là-dessus pour la souveraineté » est la même réponse que pour une absence.
/// C'est le **détail** qui porte la différence, et il commence en capitales pour qu'on ne puisse pas
/// le confondre avec un coût d'absence en parcourant la colonne.
fn line_hors_perimetre(
    governed_by: &'static str,
    capability: &'static str,
    why: &str,
) -> CapabilityReport {
    CapabilityReport {
        capability,
        status: Status::Degradee,
        detail: format!("SERVI HORS PÉRIMÈTRE — {why}"),
        governed_by,
    }
}

fn line_refusee(
    governed_by: &'static str,
    capability: &'static str,
    why: &str,
) -> CapabilityReport {
    CapabilityReport {
        capability,
        status: Status::Refusee,
        detail: format!("DEMANDÉE mais non servie — {why}"),
        governed_by,
    }
}

/// **P1.a — les variables que la frontière EXIGE et qu'un profil ne peut pas porter.**
///
/// `frontier_boot_guard` (dans `main.rs`) refuse de démarrer — `exit 78`, `EX_CONFIG` — si l'une
/// manque. Or aucune des trois ne peut vivre dans un `profiles/*.env` :
///
/// * `EFE_NANO_AUDIT_HMAC_MASTER` est un **secret** ;
/// * `DATABASE_URL` en contient un (le mot de passe) ;
/// * `EFE_OIDC_PUBLIC_KEY_PEM` n'est pas un secret mais un blob **multiligne**, qu'un fichier
///   `.env` ne sait pas représenter.
///
/// `profile-check.sh` §2 interdit d'ailleurs déjà qu'un profil les pose. Ce que le dépôt n'avait
/// pas, c'est la **contrepartie** : un profil qui déclare ce qu'il exige sans le porter. Sans elle,
/// `profiles/frontiere.env` annonçait « frontière SERVIE » dans une configuration où le binaire
/// fait `exit 78` — et rien ne pouvait l'attraper, parce que le garde vit dans `main.rs` sous
/// `#[cfg(feature = "frontier-llm")]`, hors de portée du chemin qui dérive un rapport d'un fichier.
///
/// La quatrième précondition du garde (`pii_ok`) n'est **pas** ici : elle porte sur une *feature
/// compilée*, pas sur une variable. Un fichier de profil ne peut rien en dire, et prétendre le
/// contraire serait le genre d'affirmation que ce module existe pour rendre impossible.
pub const INJECTIONS_FRONTIERE: &[&str] = &[
    "EFE_NANO_AUDIT_HMAC_MASTER",
    "EFE_OIDC_PUBLIC_KEY_PEM",
    "DATABASE_URL",
];

/// **P1.a — la ligne « frontière » d'un rapport dérivé, qui modélise le garde de démarrage.**
///
/// Avant P1, cette ligne se contentait du drapeau : `EFE_LLM_FRONTIER_ENABLED=true` donnait
/// « SERVIE », même dans une configuration où le binaire refuse de démarrer. Le mot « SERVIE » a
/// un sens unique dans tout le dépôt, et M2 a ajouté « fournie mais refusée » précisément pour ne
/// pas avoir à l'étirer : l'employer pour dire « *serait* servie si les secrets étaient là »
/// rouvrait l'écart déclaré/servi que M0 existe pour fermer — à l'endroit le pire, le seul chemin
/// **payant** et **non souverain** du produit.
///
/// Elle ne recopie aucun jugement : les trois préconditions sont déjà dans [`Resolved`], lues par
/// les mêmes fonctions pures que le chemin servi. Un fichier de profil, par construction, ne
/// portera jamais les injections — donc un profil qui active la frontière est **toujours** rendu
/// « REFUSÉE » tant qu'on le lit seul. C'est la vérité : un profil seul ne sert pas la frontière,
/// il la **demande**.
fn ligne_frontiere(r: &Resolved) -> CapabilityReport {
    if !r.frontier {
        return line("EFE_LLM_FRONTIER_ENABLED", "frontière", false);
    }
    let manquantes = r.injections_frontiere_manquantes();
    if !manquantes.is_empty() {
        return line_refusee(
            "EFE_LLM_FRONTIER_ENABLED",
            "frontière",
            &format!(
                "le garde de démarrage REFUSERA (exit 78) tant que {} manque(nt). Ces variables ne \
                 vivent jamais dans un profil — secret, ou blob multiligne — elles s'injectent au \
                 déploiement. Un profil qui active la frontière la DEMANDE, il ne la sert pas.",
                manquantes.join(", ")
            ),
        );
    }
    // Les injections y sont, mais aucun fournisseur : le binaire DÉMARRE (le garde ne les regarde
    // pas) et ne sert pourtant rien — `coordinator_from_env` rend `None`, `/v1/llm/*` répond 501.
    // Dire « SERVIE » ici serait le même écart déclaré/servi, un cran plus bas.
    if !r.frontier_provider {
        return line_refusee(
            "EFE_LLM_FRONTIER_ENABLED",
            "frontière",
            "aucune clé de fournisseur (ANTHROPIC_API_KEY / OPENAI_API_KEY / GOOGLE_AI_KEY) — le \
             binaire démarrera, et `/v1/llm/*` répondra 501. Le garde ne regarde pas les \
             fournisseurs : il protège la PREUVE, pas la disponibilité.",
        );
    }
    line("EFE_LLM_FRONTIER_ENABLED", "frontière", true)
}

/// L'état **résolu** des capacités — ce que le rapport consomme.
///
/// Exister séparément a un seul but, et il est décisif : permettre de calculer un rapport
/// **depuis un fichier de profil** sans muter l'environnement du process. Deux tests qui posent
/// des variables globales se courent l'un l'autre ; le dépôt évite déjà cela partout
/// (`bundle_export_from_env` est une fonction libre pour la même raison).
#[derive(Debug, Clone, Copy, Default)]
pub struct Resolved {
    /// **M5** : le pont réglementaire est-il demandé ? Il exige les DEUX interrupteurs — un
    /// dossier de sortie sans cadre ne peut rien dériver, et un cadre sans dossier n'a nulle part
    /// où écrire. Les compter séparément laisserait croire à un demi-câblage qui n'existe pas.
    pub reg_pont: bool,
    /// **M2** : la posture d'observation est-elle demandée ? Porté par `Resolved` — et non lu par
    /// `read::` au moment de rendre le rapport — parce que `from_resolved` doit rester résoluble
    /// depuis une TABLE quelconque : sans quoi un test de profil devrait muter l'environnement du
    /// process, et deux tests se voleraient leur réponse.
    pub capture_observation: bool,
    pub master: bool,
    pub anchor: bool,
    /// **M9** — cf. [`Observed::anchor_tierce`].
    pub anchor_tierce: bool,
    pub bundle: bool,
    pub attestation: bool,
    pub durable_store: bool,
    pub frontier: bool,
    /// **P1** — au moins une clé de fournisseur est-elle posée ? Sans elle, `coordinator_from_env`
    /// rend `None` et la frontière n'est pas servie, quelles que soient les injections.
    pub frontier_provider: bool,
    pub allowlist: bool,
    pub pii: bool,
    pub slm: bool,
    pub oidc: bool,
    pub router: bool,
}

impl Resolved {
    /// Depuis l'environnement du process — le chemin servi.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            reg_pont: read::reg_evidence_dir().is_some()
                && read::reg_framework().is_some_and(|f| crate::reg_pont::CADRES.contains(&f.trim())),
            capture_observation: read::capture_policy().as_deref() == Some("observation"),
            master: read::master_present(),
            anchor: read::anchor_log().is_some(),
            // TIERCE seulement si les DEUX sont posés : sans répertoire, le binaire refuse de
            // démarrer, donc annoncer « tierce » sur la seule URL décrirait un service qui n'existe
            // pas.
            anchor_tierce: read::tsa_url().is_some() && read::tsa_dir().is_some(),
            bundle: read::bundle_enabled(),
            attestation: read::bundle_signing_seed().is_some(),
            durable_store: read::database_url().is_some(),
            frontier: read::frontier_enabled(),
            frontier_provider: read::frontier_provider_present(),
            allowlist: read::tenant_allowlist().is_some(),
            pii: read::pii_default_policy().is_some(),
            slm: read::slm_reformulator_url().is_some(),
            oidc: read::oidc_public_key_pem().is_some(),
            router: read::router_endpoint().is_some(),
        }
    }

    /// Depuis une source arbitraire — un fichier de profil, un test. **Utilise les mêmes
    /// fonctions pures** (`flag_of`, `present_of`) que le chemin servi : aucune sémantique n'est
    /// dupliquée, donc aucune ne peut dériver.
    #[must_use]
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let p = |k: &str| read::present_of(get(k).as_deref());
        let f = |k: &str| read::flag_of(get(k).as_deref());
        Self {
            reg_pont: p("EFE_SWARM_REG_EVIDENCE_DIR")
                && get("EFE_SWARM_REG_FRAMEWORK")
                    .is_some_and(|f| crate::reg_pont::CADRES.contains(&f.trim())),
            capture_observation: get("EFE_SWARM_CAPTURE_POLICY").as_deref() == Some("observation"),
            master: p("EFE_NANO_AUDIT_HMAC_MASTER"),
            anchor: p("EFE_SWARM_ANCHOR_LOG"),
            anchor_tierce: p("EFE_SWARM_TSA_URL") && p("EFE_SWARM_TSA_DIR"),
            bundle: f("EFE_SWARM_BUNDLE_ENABLED"),
            attestation: p("EFE_SWARM_BUNDLE_SIGNING_SEED"),
            durable_store: p("DATABASE_URL"),
            frontier: f("EFE_LLM_FRONTIER_ENABLED"),
            frontier_provider: p("ANTHROPIC_API_KEY") || p("OPENAI_API_KEY") || p("GOOGLE_AI_KEY"),
            allowlist: p("EFE_LLM_TENANT_ALLOWLIST"),
            pii: p("EFE_PII_DEFAULT_POLICY"),
            slm: p("EFE_SLM_REFORMULATOR_URL"),
            oidc: p("EFE_OIDC_PUBLIC_KEY_PEM"),
            router: p("EFE_SWARM_ROUTER_ENDPOINT"),
        }
    }

    /// **P1.a** — celles des [`INJECTIONS_FRONTIERE`] qui manquent, dans l'ordre du tableau.
    ///
    /// Itère sur la **même** liste que celle qu'un profil déclare : deux sources se seraient
    /// séparées au premier ajout, et c'est exactement le défaut que M1 a passé deux modules à
    /// fermer. Un nom ajouté à la liste sans être câblé ci-dessous compte comme **manquant pour
    /// toujours** (`_ => false`) — le test devient rouge immédiatement plutôt que silencieux.
    #[must_use]
    pub fn injections_frontiere_manquantes(&self) -> Vec<&'static str> {
        INJECTIONS_FRONTIERE.iter().copied().filter(|n| !self.a_linjection(n)).collect()
    }

    /// Cette injection est-elle présente ? Fail-closed sur un nom inconnu — cf. ci-dessus.
    #[must_use]
    fn a_linjection(&self, nom: &str) -> bool {
        match nom {
            "EFE_NANO_AUDIT_HMAC_MASTER" => self.master,
            "EFE_OIDC_PUBLIC_KEY_PEM" => self.oidc,
            "DATABASE_URL" => self.durable_store,
            _ => false,
        }
    }
}

impl Report {
    /// Calcule le rapport **depuis les mêmes lectures que le constructeur d'état consomme** —
    /// jamais depuis une liste parallèle. C'est ce qui empêche le rapport d'affirmer plus que le
    /// code ne fait.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_resolved(&Resolved::from_env()).with_invalides(invalides_from_env())
    }

    /// Greffe les valeurs refusées sur un rapport déjà calculé.
    ///
    /// Séparé plutôt que fondu dans les constructeurs pour une raison précise : [`Resolved`] a déjà
    /// réduit chaque interrupteur à un booléen, et une valeur refusée n'est plus distinguable d'une
    /// absence une fois cette réduction faite. Il faut donc la lire à la source — et c'est aussi ce
    /// qui permet à un test de la calculer depuis un fichier.
    #[must_use]
    pub fn with_invalides(mut self, v: Vec<Invalide>) -> Self {
        self.invalides = v;
        self
    }

    /// Le rapport du **service** — construit depuis ce qui a été réellement câblé.
    ///
    /// C'est celui qu'imprime le binaire au démarrage, APRÈS l'assemblage de l'état. Le calculer
    /// avant, ou depuis l'environnement seul, annoncerait des capacités que le processus ne sert
    /// pas encore (ou pas du tout).
    #[must_use]
    pub fn from_observed(o: &Observed) -> Self {
        let mut capabilities = vec![
            line("EFE_NANO_AUDIT_HMAC_MASTER", "provenance", o.provenance_non_forgeable),
            line("EFE_SWARM_ANCHOR_LOG", "ancre", o.anchor),
            line("EFE_SWARM_TSA_URL", "ancre tierce", o.anchor_tierce),
            line("EFE_SWARM_BUNDLE_ENABLED", "export de bundle", o.bundle),
            line("EFE_SWARM_BUNDLE_SIGNING_SEED", "attestation", o.attestation),
            line("DATABASE_URL", "store durable", o.durable_store),
        ];
        // Le seul cas de REFUS servi : demandée, et pas là.
        capabilities.push(if o.frontier_intended && !o.frontier {
            line_refusee(
                "EFE_LLM_FRONTIER_ENABLED",
                "frontière",
                "aucun coordinator câblé (feature `frontier-llm` absente, ou clés provider manquantes). \
                 `/v1/llm/*` répondra 501 et `/readyz` refusera le trafic.",
            )
        } else {
            line("EFE_LLM_FRONTIER_ENABLED", "frontière", o.frontier)
        });
        capabilities.extend([
            line("EFE_LLM_TENANT_ALLOWLIST", "allowlist d'egress", o.allowlist),
            line("EFE_PII_DEFAULT_POLICY", "caviardage PII", o.pii),
            slm_line(o),
            line("EFE_OIDC_PUBLIC_KEY_PEM", "authentification", o.oidc),
            line("EFE_SWARM_ROUTER_ENDPOINT", "routeur de modèles", o.router),
            line("EFE_SWARM_CAPTURE_POLICY", "posture de capture", o.capture_observation),
            line("EFE_SWARM_REG_EVIDENCE_DIR", "pont réglementaire", o.reg_pont),
        ]);
        let mut r = Self::from_resolved(&Resolved::default());
        // On garde les lignes de RÉGLAGE (plafonds, admission) telles que calculées, et on
        // remplace les capacités par celles observées : les réglages ne dépendent pas du câblage.
        capabilities.extend(r.capabilities.drain(..).filter(|c| {
            matches!(c.capability, "plafond de dépense" | "admission" | "plafonds de requête")
        }));
        Report { mode: Mode::Servi, capabilities, features: r.features, invalides: Vec::new() }
    }

    /// Le cœur : les capacités, leur état, et **ce que leur absence coûte**.
    #[must_use]
    pub fn from_resolved(r: &Resolved) -> Self {
        let mut capabilities = vec![
            line("EFE_NANO_AUDIT_HMAC_MASTER", "provenance", r.master),
            line("EFE_SWARM_ANCHOR_LOG", "ancre", r.anchor),
            line("EFE_SWARM_TSA_URL", "ancre tierce", r.anchor_tierce),
            line("EFE_SWARM_BUNDLE_ENABLED", "export de bundle", r.bundle),
            line("EFE_SWARM_BUNDLE_SIGNING_SEED", "attestation", r.attestation),
            line("DATABASE_URL", "store durable", r.durable_store),
            ligne_frontiere(r),
            line("EFE_LLM_TENANT_ALLOWLIST", "allowlist d'egress", r.allowlist),
            line("EFE_PII_DEFAULT_POLICY", "caviardage PII", r.pii),
            line("EFE_SLM_REFORMULATOR_URL", "relais SLM", r.slm),
            line("EFE_OIDC_PUBLIC_KEY_PEM", "authentification", r.oidc),
            line("EFE_SWARM_ROUTER_ENDPOINT", "routeur de modèles", r.router),
        ];

        // Les réglages : toujours SERVIS (ils ont un défaut sûr), mais leur valeur effective est
        // dite — un plafond silencieux est un plafond qu'on découvre en production.
        capabilities.push(CapabilityReport {
            capability: "plafond de dépense",
            status: Status::Servie,
            detail: format!("{} c par requête", read::max_budget_cents()),
            governed_by: "EFE_LLM_MAX_BUDGET_CENTS",
        });
        let global = read::max_inflight_runs();
        capabilities.push(CapabilityReport {
            capability: "admission",
            status: Status::Servie,
            detail: format!("{global} runs simultanés, {} par tenant", read::max_inflight_per_tenant(global)),
            governed_by: "EFE_GATEWAY_MAX_INFLIGHT_RUNS",
        });
        // M2 : la posture de capture. `Degradee` sous `blocking` n'est PAS un reproche — c'est le
        // défaut sûr pour la preuve. Mais la capacité « observation non intrusive » y est bel et
        // bien éteinte, et son coût doit être lisible : le rapport nomme ce que le client perd.
        capabilities.push(line("EFE_SWARM_CAPTURE_POLICY", "posture de capture", r.capture_observation));
        capabilities.push(line("EFE_SWARM_REG_EVIDENCE_DIR", "pont réglementaire", r.reg_pont));
        capabilities.push(CapabilityReport {
            capability: "plafonds de requête",
            status: Status::Servie,
            detail: format!("largeur ≤ {}, deadline ≤ {} ms", read::max_width(), read::max_deadline_ms(50)),
            governed_by: "EFE_GATEWAY_MAX_WIDTH",
        });

        let mut features = Vec::new();
        if cfg!(feature = "frontier-llm") { features.push("frontier-llm"); }
        if cfg!(feature = "postgres") { features.push("postgres"); }
        if cfg!(feature = "oidc") { features.push("oidc"); }
        if cfg!(feature = "pii") { features.push("pii"); }
        if cfg!(feature = "tamper-testkit") { features.push("tamper-testkit"); }
        if cfg!(feature = "demo") { features.push("demo"); }

        Report { mode: Mode::Intention, capabilities, features, invalides: Vec::new() }
    }

    /// La forme **humaine**, imprimée au démarrage. Complète, là où les `println!` épars ne
    /// l'étaient pas.
    #[must_use]
    pub fn render_human(&self) -> String {
        let mut out = format!("┌─ profil {} ({})\n", self.mode.as_str(),
            if self.mode == Mode::Servi { "ce qui est réellement câblé" } else { "ce que la config demande, RIEN n'est câblé" });
        for c in &self.capabilities {
            out.push_str(&format!("│  {:<22} {:<9} {}\n", c.capability, c.status.as_str(), c.detail));
        }
        let feats = if self.features.is_empty() { "(aucune)".to_string() } else { self.features.join(", ") };
        out.push_str(&format!("└─ features compilées : {feats}\n"));
        for i in &self.invalides {
            out.push_str(&format!(
                "\n⚠  {} = '{}' — REFUSÉ (attendu : {}).\n   \
                 Vous avez posé cet interrupteur et il n'a EU AUCUN EFFET : {}.\n",
                i.name, i.fourni, i.attendu, i.consequence
            ));
        }
        out
    }

    /// La forme **machine**, servie par `--profile-report`. Jamais par une route : le rapport dit
    /// ce qui est *éteint*, ce qui est un renseignement pour un attaquant.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let caps: Vec<Value> = self
            .capabilities
            .iter()
            .map(|c| {
                json!({
                    "capability":   c.capability,
                    "status":       c.status.as_str(),
                    "detail":       c.detail,
                    "governed_by":  c.governed_by,
                })
            })
            .collect();
        let inv: Vec<Value> = self
            .invalides
            .iter()
            .map(|i| json!({ "switch": i.name, "fourni": i.fourni, "attendu": i.attendu }))
            .collect();
        json!({
            "mode":         self.mode.as_str(),
            "capabilities": caps,
            "features":     self.features,
            "invalides":    inv,
        })
    }

    /// Le raccourci du critère d'acceptation : `.provenance == "SERVIE"`.
    #[must_use]
    pub fn status_of(&self, capability: &str) -> Option<Status> {
        self.capabilities.iter().find(|c| c.capability == capability).map(|c| c.status)
    }
}

// ═════════════════════════════════════════════════════════════════════════════════════════════
//  T2 · Le rapport couvre le registre
// ═════════════════════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// **P1 — les trois issues du relais SLM.**
    ///
    /// Avant P1, la ligne était calculée par `line(VAR, "relais SLM", url.is_some())` : poser
    /// l'URL suffisait à faire imprimer « SERVIE ». Deux situations très différentes se cachaient
    /// derrière ce vert, et aucune des deux ne servait quoi que ce soit de souverain.
    #[test]
    fn le_relais_slm_a_trois_issues_et_une_seule_est_verte() {
        let statut = |o: &Observed| Report::from_observed(o).status_of("relais SLM").unwrap();
        let detail = |o: &Observed| {
            Report::from_observed(o)
                .capabilities
                .iter()
                .find(|c| c.capability == "relais SLM")
                .unwrap()
                .detail
                .clone()
        };

        // (1) URL posée, coordinator absent : rien ne la consomme. C'était le vert le plus faux —
        //     le build par défaut ne compile même pas le code qui lirait cette URL.
        let demande = Observed { slm_intended: true, slm: false, ..Observed::default() };
        assert_eq!(statut(&demande), Status::Refusee, "posée mais consommée par personne");
        assert!(
            detail(&demande).contains("coordinator"),
            "le rapport doit dire OÙ vit le relais, sinon l'opérateur cherchera la mauvaise variable"
        );

        // (2) Câblé, hôte distant : la capacité FONCTIONNE, et la promesse est morte.
        let hors = Observed { slm_intended: true, slm: true, slm_local: false, ..Observed::default() };
        assert_eq!(statut(&hors), Status::Degradee);
        assert!(
            detail(&hors).starts_with("SERVI HORS PÉRIMÈTRE"),
            "en capitales et en tête : impossible à confondre avec un coût d'absence"
        );

        // (3) Câblé, hôte local : le seul cas où « SERVIE » est vrai.
        let local = Observed { slm_intended: true, slm: true, slm_local: true, ..Observed::default() };
        assert_eq!(statut(&local), Status::Servie);

        // (4) Rien de posé : l'absence ordinaire, inchangée depuis M0.
        let absent = Observed::default();
        assert_eq!(statut(&absent), Status::Degradee);
        assert!(
            !detail(&absent).starts_with("SERVI HORS PÉRIMÈTRE"),
            "une absence n'est pas un hors-périmètre : les deux détails doivent rester distincts"
        );
    }

    /// Le hors-périmètre doit être **lisible** dans la forme humaine, pas seulement dans le JSON :
    /// c'est celle que l'opérateur voit défiler au démarrage.
    #[test]
    fn le_hors_perimetre_se_lit_dans_le_rapport_humain() {
        let h = Report::from_observed(&Observed {
            slm_intended: true,
            slm: true,
            slm_local: false,
            ..Observed::default()
        })
        .render_human();
        assert!(h.contains("SERVI HORS PÉRIMÈTRE"), "le motif doit apparaître tel quel");
        assert!(h.contains("ni décompté"), "et dire ce que ça coûte, pas seulement que c'est dégradé");
    }

    /// **La liste des drapeaux stricts ne doit pas dériver des appels réels.**
    ///
    /// [`FLAGS_STRICTS`] est une liste écrite à la main : si quelqu'un ajoute un `read::flag(…)`
    /// sans l'y inscrire, son footgun redevient silencieux — et le mécanisme entier serait un
    /// filet troué qu'on croirait plein. Ce test lit **le source de ce fichier même** et compare.
    ///
    /// Le balayage est fiable ici, et seulement ici : `flag` reçoit toujours un **littéral** au
    /// point d'appel (`flag("EFE_…")`). Les trois primitives qui touchent `std::env::var`, elles,
    /// reçoivent leur nom en paramètre — d'où l'assertion d'exécution dans `read::declare`, qui
    /// couvre ce qu'aucun balayage ne pourrait voir.
    #[test]
    fn les_flags_stricts_sont_exactement_ceux_lus_par_flag() {
        let src = include_str!("profile.rs");
        let mut lus: Vec<String> = Vec::new();
        for morceau in src.split("flag(\"").skip(1) {
            if let Some(nom) = morceau.split('"').next() {
                // Écarter `flag_of("…")`, qui n'est pas une lecture d'environnement, et tout ce qui
                // ne ressemble pas à un nom de variable.
                if nom.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                    && nom.len() > 3
                    && !lus.iter().any(|d| d == nom)
                {
                    lus.push(nom.to_string());
                }
            }
        }
        lus.sort();
        let mut declares: Vec<String> = FLAGS_STRICTS.iter().map(|s| (*s).to_string()).collect();
        declares.sort();
        assert_eq!(
            lus, declares,
            "\n`FLAGS_STRICTS` a dérivé des appels réels à `read::flag`.\n\
             Un drapeau lu mais non déclaré ici ne dira JAMAIS à l'opérateur que sa valeur a été \
             refusée.\n"
        );
    }

    /// Le cas concret que le dépôt savait diagnostiquer, puis avait perdu.
    #[test]
    fn une_valeur_footgun_est_nommee_et_sa_consequence_dite() {
        let vars = [("EFE_LLM_FRONTIER_ENABLED", "1")];
        let inv = invalides_from_lookup(|k| {
            vars.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_string())
        });
        assert_eq!(inv.len(), 1, "`1` doit être signalé — c'est le footgun documenté");
        assert_eq!(inv[0].name, "EFE_LLM_FRONTIER_ENABLED");
        assert_eq!(inv[0].fourni, "1");
        assert!(
            !inv[0].consequence.is_empty(),
            "dire « refusé » sans dire ce que ça coûte laisse l'opérateur juger à l'aveugle"
        );
    }

    /// L'absence et le vide ne sont **pas** des valeurs refusées. Les confondre noierait le signal :
    /// un rapport qui crie à chaque interrupteur non posé ne se lit plus.
    #[test]
    fn ni_labsence_ni_le_vide_ni_true_ne_sont_des_refus() {
        assert!(invalides_from_lookup(|_| None).is_empty(), "absence ≠ refus");
        assert!(invalides_from_lookup(|_| Some("   ".into())).is_empty(), "vide ≠ refus");
        // `true` n'est accepté que pour les DRAPEAUX. La première rédaction de ce test posait
        // `true` sur TOUTES les variables et exigeait zéro refus — prémisse vraie tant que seuls
        // des booléens existaient, devenue fausse avec le premier interrupteur ÉNUMÉRÉ (M2) :
        // `EFE_SWARM_CAPTURE_POLICY=true` doit être signalé, et l'est. On cible donc les drapeaux.
        let drapeaux_seuls =
            |k: &str| FLAGS_STRICTS.contains(&k).then(|| "true".to_string());
        assert!(invalides_from_lookup(drapeaux_seuls).is_empty(), "`true` est accepté d'un drapeau");
        for eteint in ["false", "FALSE", "0", "no", "off"] {
            let f = |k: &str| FLAGS_STRICTS.contains(&k).then(|| eteint.to_string());
            assert!(
                invalides_from_lookup(f).is_empty(),
                "`{eteint}` éteint DÉLIBÉRÉMENT : le résultat correspond à l'intention, donc silence"
            );
        }
        let tout_majuscule = |k: &str| FLAGS_STRICTS.contains(&k).then(|| "TRUE".to_string());
        assert_eq!(
            invalides_from_lookup(tout_majuscule).len(),
            FLAGS_STRICTS.len(),
            "`TRUE` est refusé partout — la casse compte, et c'est précisément le piège"
        );
    }

    /// **M2** — le mécanisme s'étend aux interrupteurs à plusieurs valeurs, et pour la même raison.
    ///
    /// `EFE_SWARM_CAPTURE_POLICY=observe` retombe silencieusement sur le défaut : l'opérateur croit
    /// avoir choisi la posture d'observation, la passerelle reste bloquante, et le premier incident
    /// de store lui coûtera les réponses de ses clients. Généraliser plutôt que d'ajouter un cas
    /// particulier, c'est ce qui évite que le prochain interrupteur énuméré rate le filet.
    #[test]
    fn une_valeur_hors_enumeration_est_signalee() {
        let inv = |v: &'static str| {
            invalides_from_lookup(move |k| (k == "EFE_SWARM_CAPTURE_POLICY").then(|| v.to_string()))
        };
        for accepte in ["blocking", "observation"] {
            assert!(inv(accepte).is_empty(), "`{accepte}` appartient à l'énumération");
        }
        for refuse in ["observe", "Observation", "true", "on", "OBSERVATION"] {
            let r = inv(refuse);
            assert_eq!(r.len(), 1, "`{refuse}` doit être signalé");
            assert_eq!(r[0].name, "EFE_SWARM_CAPTURE_POLICY");
            assert!(
                r[0].consequence.contains("BLOQUANTE"),
                "et le rapport doit dire ce que l'opérateur subira faute d'avoir été entendu"
            );
        }
        // Chaque nom déclaré au registre des énumérations doit y être vraiment déclaré, sinon la
        // ligne serait muette dans le rapport.
        for (name, valeurs) in ENUMS {
            assert!(switch(name).is_some(), "{name} : énuméré mais absent du registre");
            assert!(valeurs.len() >= 2, "{name} : une énumération à une valeur n'en est pas une");
        }
    }

    /// Un diagnostic que le rapport ne montre pas n'existe pas. Il doit nommer la variable, la
    /// valeur reçue, et le fait que le geste de l'opérateur est resté sans effet.
    #[test]
    fn le_rapport_humain_crie_la_valeur_refusee() {
        let r = Report::from_resolved(&Resolved::default()).with_invalides(vec![Invalide {
            name: "EFE_SWARM_BUNDLE_ENABLED",
            fourni: "yes".into(),
            attendu: "exactement `true`",
            consequence: "aucun export de bundle",
        }]);
        let h = r.render_human();
        assert!(h.contains("EFE_SWARM_BUNDLE_ENABLED"), "la variable doit être nommée");
        assert!(h.contains("'yes'"), "la valeur reçue doit être montrée telle quelle");
        assert!(h.contains("AUCUN EFFET"), "le rapport doit dire que le geste n'a rien fait");
        assert_eq!(r.to_json()["invalides"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn le_registre_na_pas_de_doublon() {
        let mut names: Vec<&str> = REGISTRY.iter().map(|s| s.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "un interrupteur est déclaré deux fois");
    }

    #[test]
    fn chaque_interrupteur_dit_ce_que_son_absence_coute() {
        for s in REGISTRY {
            assert!(!s.buys.trim().is_empty(), "{} : `buys` vide", s.name);
            assert!(
                !s.absence_costs.trim().is_empty() && s.absence_costs.trim() != "—",
                "{} : `absence_costs` vide — un coût non dit est un mensonge par omission",
                s.name
            );
        }
    }

    /// **T2** — toute capacité du rapport est gouvernée par un interrupteur **déclaré**.
    ///
    /// Les DEUX rapports, et c'est l'audit croisé des modules qui l'a exigé : cette version ne
    /// vérifiait que le rapport d'INTENTION (`from_env`). Or les lignes ajoutées depuis vivent
    /// toutes dans le rapport **SERVI** — `line_refusee` et `line_hors_perimetre` (M2), la ligne du
    /// pont réglementaire (M5). Une capacité servie gouvernée par un interrupteur absent du
    /// registre serait passée sans un bruit : l'opérateur lirait un nom de variable qu'il ne
    /// trouverait nulle part, et qu'aucun rapport ne saurait expliquer.
    ///
    /// La propriété tenait — les 12 citations du rapport servi sont toutes déclarées — mais elle
    /// n'était pas gardée. Une propriété vraie et non gardée est une propriété qui attend son
    /// accident.
    #[test]
    fn t2_le_rapport_couvre_le_registre() {
        let verifie = |r: &Report, quoi: &str| {
            assert!(!r.capabilities.is_empty(), "rapport {quoi} vide");
            for c in &r.capabilities {
                assert!(
                    switch(c.governed_by).is_some(),
                    "rapport {quoi} : la capacité « {} » est gouvernée par {} qui n'est PAS dans \
                     le registre",
                    c.capability,
                    c.governed_by
                );
            }
        };
        verifie(&Report::from_env(), "d'intention");

        // Le rapport SERVI, dans ses trois régimes : rien câblé, tout câblé, et les cas
        // particuliers (frontière demandée sans coordinator ⟹ REFUSÉE ; relais SLM hors périmètre)
        // qui produisent des lignes qu'aucun autre chemin ne fabrique.
        verifie(&Report::from_observed(&Observed::default()), "servi (rien câblé)");
        verifie(
            &Report::from_observed(&Observed {
                provenance_non_forgeable: true,
                anchor: true,
                anchor_tierce: false,
                bundle: true,
                attestation: true,
                durable_store: true,
                frontier: true,
                frontier_intended: true,
                allowlist: true,
                pii: true,
                slm: true,
                slm_intended: true,
                slm_local: true,
                oidc: true,
                router: true,
                capture_observation: true,
                reg_pont: true,
            }),
            "servi (tout câblé)",
        );
        verifie(
            &Report::from_observed(&Observed {
                // frontière DEMANDÉE sans coordinator ⟹ la ligne REFUSÉE de M2
                frontier_intended: true,
                // relais SLM câblé HORS périmètre ⟹ la ligne « SERVI HORS PÉRIMÈTRE » de P1
                slm_intended: true,
                slm: true,
                slm_local: false,
                ..Observed::default()
            }),
            "servi (refus + hors périmètre)",
        );
    }

    #[test]
    fn le_rapport_json_est_stable_et_complet() {
        let v = Report::from_env().to_json();
        assert!(v["capabilities"].is_array(), "capabilities absent");
        assert!(v["features"].is_array(), "features absent");
        let n = v["capabilities"].as_array().map(Vec::len).unwrap_or(0);
        assert!(n >= 11, "rapport trop court : {n} capacités");
    }

    /// La convention du dépôt : SEULE la chaîne `"true"` active. Un test qui fige cela évite
    /// qu'un « 1 » silencieusement inerte ne devienne un incident d'exploitation.
    #[test]
    fn seule_la_chaine_true_active() {
        // On ne mute pas l'environnement du process (course entre tests) : on vérifie le contrat
        // sur la valeur par défaut, qui est le cas réel d'un déploiement sans la variable.
        assert!(!read::bundle_enabled() || std::env::var("EFE_SWARM_BUNDLE_ENABLED").as_deref() == Ok("true"));
    }
}
