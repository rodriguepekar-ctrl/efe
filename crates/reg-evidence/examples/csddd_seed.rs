// SPDX-License-Identifier: UNLICENSED
//! # `csddd_seed` — le **seed de démo CSDDD** : un dossier de diligence fournisseur, scellé puis dérivé.
//!
//! Ce que ce programme produit, en **une commande**, est un **bundle forensic autoportant** sur lequel les
//! **trois démos** de la présentation Kovant tournent — sans rien préparer d'autre :
//!
//! - **D1 — la re-preuve sans confiance** : `records.sealed.jsonl` se re-prouve avec `verify-mini`
//!   (vérifieur à **zéro dépendance** qu'un tiers sceptique re-compile) → `exit 0`.
//! - **D2 — l'altération détectée** : `records.TAMPERED.jsonl` (un seul champ scellé retourné —
//!   le verdict de criblage sanctions d'un fournisseur **bloqué**, passé de `FAIL` à `PASS`) → `exit 1`,
//!   pointant le record altéré.
//! - **D3 — le dossier réglementaire** : `reg/csddd.html` (+ `lksg`, + `eu_ai_act`) est la **VUE
//!   re-dérivable** — `artefact = derive(records_re-prouvés, mapping@version)`, une fonction déterministe
//!   que le destinataire **re-calcule** (`reg/REDERIVE.md`). **Outillé ≠ conforme** : le dossier *fournit
//!   et mappe* l'évidence, il ne *prononce* pas la conformité.
//!
//! Le fil rouge du bundle : **une seule capture** (le workflow de diligence, scellé au fil de l'exécution
//! via la capture S1 + le gate humain S2) **relue sous trois cadres** (CSDDD 2024/1760, LkSG, EU AI Act
//! 2024/1689). La **clé tenant** re-prouve mais **n'est jamais écrite** dans le bundle (souveraineté) :
//! ce programme l'**imprime** au terminal — c'est elle que le destinataire obtient « auprès du responsable
//! de traitement », pas dans les fichiers.
//!
//! ```text
//! cargo run -q -p reg-evidence --example csddd_seed            # -> ~/Downloads/csddd-demo-bundle
//! cargo run -q -p reg-evidence --example csddd_seed -- /chemin/de/sortie
//! ```

use audit_api::{AuditService, AuthContext, HumanActionInput, ProjectionConfig, Role};
use provenance_capture::{LlmCall, MessageEvent, ToolCall};
use provenance_dag::{HumanAction, SealedDag, TenantKey};
use reg_evidence::{derive, render_html, render_md, templates, CaseScope, Framework, MappingSet, RegulatoryArtifact};
use std::path::{Path, PathBuf};

/// Les octets bruts de la **clé tenant** du client. Elle sert de clé HMAC à S0 (elle **re-prouve**), mais
/// elle **n'entre jamais** dans un bundle — le destinataire l'obtient du responsable de traitement. Ici
/// elle est fixe (démo reproductible) et **imprimée** au terminal, pas écrite sur le disque.
const KEY_BYTES: [u8; 32] = [0x2A; 32];

fn key() -> TenantKey {
    TenantKey::from_bytes(KEY_BYTES)
}

/// L'officier de conformité qui exerce l'oversight (RBAC — le gate humain S2 valide le rôle et la cible).
fn officer() -> AuthContext {
    AuthContext::new("acme-conformite", Role::ComplianceOfficer, "officer:hanna-keller")
}

/// La clé en hex (64 caractères) — ce qu'on passe à `verify-mini --key-hex`.
fn key_hex() -> String {
    KEY_BYTES.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── 1. CAPTURE (S1/S2) — sceller un vrai dossier de diligence fournisseur au fil de l'exécution ──
    let jsonl = seal_supplier_diligence();

    // ── 2. RE-PARSER comme un tiers, puis DÉRIVER les dossiers (S6) sur le seul cas SUP-2024 ──
    // (SealedDag::records_from_jsonl : exactement le geste du destinataire — repartir des seuls records.)
    let recs = SealedDag::records_from_jsonl(&jsonl).expect("le JSONL exporté se re-parse");
    let diligence = templates::csddd_lksg_2026_06(); // un seul registre porte CSDDD *et* LkSG
    let eu = templates::eu_ai_act_2026_06();
    let scope = CaseScope::Case("SUP-2024".into());

    let csddd = derive(&recs, &key(), &diligence, Framework::Csddd, &scope)?;
    let lksg = derive(&recs, &key(), &diligence, Framework::Lksg, &scope)?;
    let euai = derive(&recs, &key(), &eu, Framework::EuAiAct, &scope)?;

    // La VUE est re-dérivable : re-faire derive(...) donne le MÊME artefact (zéro confiance neuve).
    assert_eq!(csddd, derive(&recs, &key(), &diligence, Framework::Csddd, &scope)?, "CSDDD re-dérivable");
    let root = csddd.manifest_root.clone();

    // ── 3. ÉCRIRE le bundle (records + records altérés + 3 dossiers + mappings + REDERIVE + README) ──
    let dir = out_dir();
    write_bundle(&dir, &jsonl, &[(&csddd, "csddd"), (&lksg, "lksg"), (&euai, "eu_ai_act")], &diligence, &eu, &root)?;

    // ── 4. IMPRIMER le mode d'emploi de la démo : clé, root, et les trois commandes prêtes à coller ──
    print_demo_guide(&dir, &root);
    Ok(())
}

/// Construit le workflow de diligence fournisseur et rend le **JSONL re-prouvable** de l'export S2.
///
/// Deux cas coexistent dans le tenant :
/// - **SUP-2024** (« Nordwind Textilhandel GmbH ») — le cas *riche* : intake → criblage sanctions (sur
///   évidence figée) → évaluation ESG (sur évidence figée) → agrégation → **double oversight** humain
///   (escalade puis approbation sous conditions). Il évidence CSDDD art. 8/10/11, LkSG §6/§10(1)/revue,
///   et EU AI Act art. 12/13/14/26.
/// - **SUP-2099** (« Ferro Components Ltd ») — un cas *bref* et **bloqué** (correspondance sanctions).
///   Il sert à (a) **prouver l'isolation par-cas** (le dossier SUP-2024 n'en fuit rien) et (b) fournir la
///   cible de l'altération D2 (son verdict `FAIL` que l'on tentera de maquiller en `PASS`).
fn seal_supplier_diligence() -> String {
    let mut svc = AuditService::new(key(), ProjectionConfig::default());
    let off = officer();

    // ═══ Cas SUP-2024 — Nordwind Textilhandel GmbH (fournisseur textile de rang 1) ═══

    // Acte — intake : extraction structurée du dossier fournisseur.
    svc.ingest_llm(&LlmCall {
        case_id: "SUP-2024",
        step: "intake",
        agent_role: "supplier_intake",
        inputs: "dossier: Nordwind Textilhandel GmbH, Hambourg; UBO x2; certificats ISO/OEKO-TEX".as_bytes(),
        model: "phi4-mini-local",
        version: "1",
        output: "supplier=Nordwind Textilhandel GmbH; pays=DE; rang=1; secteur=textile; UBO=2",
        evidence_steps: &[],
        timestamp: 1,
    })
    .expect("intake scellé");

    // Évidence — snapshot de la liste consolidée sanctions/PEP, figé au moment T (consommé par le criblage).
    svc.ingest_evidence(&ToolCall {
        case_id: "SUP-2024",
        step: "ev-sanctions",
        tool: "sanctions_pep_db",
        query: "screen: Nordwind Textilhandel GmbH + UBO vs listes consolidees".as_bytes(),
        retrieved_ref: "eu-consolidated://2026-06-28",
        content: "aucune correspondance - snapshot listes consolidees UE/OFAC/UK au 2026-06-28".as_bytes(),
        consumed_by_step: "sanction",
        timestamp: 2,
    })
    .expect("évidence sanctions scellée");

    // Acte — criblage sanctions/PEP (cite l'évidence) -> PASS.
    svc.ingest_llm(&LlmCall {
        case_id: "SUP-2024",
        step: "sanction",
        agent_role: "sanction_screen",
        inputs: "entite + UBO vs liste consolidee".as_bytes(),
        model: "phi4-mini-local",
        version: "1",
        output: "PASS",
        evidence_steps: &["ev-sanctions"],
        timestamp: 3,
    })
    .expect("criblage sanctions scellé");

    // Évidence — rapport d'audit social/environnemental du sous-tier, figé au moment T (consommé par l'ESG).
    svc.ingest_evidence(&ToolCall {
        case_id: "SUP-2024",
        step: "ev-esg",
        tool: "esg_risk_registry",
        query: "risque droits humains + environnement: textile, teinture, sous-tier".as_bytes(),
        retrieved_ref: "amfori-bsci://audit/2025-11",
        content: "risque ELEVE au sous-tier teinture (rejets aqueux); lacunes audit social tier-2".as_bytes(),
        consumed_by_step: "esg",
        timestamp: 4,
    })
    .expect("évidence ESG scellée");

    // Acte — évaluation ESG / droits humains (cite l'évidence) -> REVIEW (risque sous-tier).
    svc.ingest_llm(&LlmCall {
        case_id: "SUP-2024",
        step: "esg",
        agent_role: "esg_risk_assess",
        inputs: "profil fournisseur + rapport audit sous-tier".as_bytes(),
        model: "phi4-mini-local",
        version: "1",
        output: "REVIEW — risque environnemental élevé au sous-tier teinture; remédiation requise",
        evidence_steps: &["ev-esg"],
        timestamp: 5,
    })
    .expect("évaluation ESG scellée");

    // Acte — agrégation du verdict de diligence -> CONDITIONNEL (approbation sous plan d'atténuation).
    // (Scellée AVANT les arêtes : une arête ne peut référencer qu'un nœud DÉJÀ scellé — sinon DanglingRef.)
    svc.ingest_llm(&LlmCall {
        case_id: "SUP-2024",
        step: "aggregate",
        agent_role: "diligence_aggregate",
        inputs: "sanction=PASS; esg=REVIEW".as_bytes(),
        model: "phi4-mini-local",
        version: "1",
        output: "CONDITIONNEL — approuver sous plan d'atténuation (audit sous-tier + clause eau)",
        evidence_steps: &[],
        timestamp: 6,
    })
    .expect("agrégation scellée");

    // Arête — le supplier_record circule de l'intake au criblage (communication typée).
    svc.ingest_message(&MessageEvent {
        case_id: "SUP-2024",
        step: "e1",
        from_step: "intake",
        to_step: "sanction",
        artifact: "supplier_record".as_bytes(),
        artifact_type: "supplier_record",
        timestamp: 7,
    })
    .expect("arête e1 scellée");

    // Arête — le risk_profile circule de l'ESG à l'agrégation (les deux nœuds existent déjà).
    svc.ingest_message(&MessageEvent {
        case_id: "SUP-2024",
        step: "e2",
        from_step: "esg",
        to_step: "aggregate",
        artifact: "risk_profile".as_bytes(),
        artifact_type: "risk_profile",
        timestamp: 8,
    })
    .expect("arête e2 scellée");

    // Oversight humain 1 — ESCALADE : le risque ESG sous-tier est remonté au comité de conformité.
    svc.record_human_action(
        "SUP-2024",
        &HumanActionInput {
            target_step: "aggregate".into(),
            action: HumanAction::Escalate,
            justification: "Risque environnemental sous-tier signalé; dossier remonté au comité de conformité (CSDDD art. 10 / LkSG §6).".into(),
        },
        &off,
        9,
    )
    .expect("escalade scellée");

    // Oversight humain 2 — APPROBATION sous conditions, après validation du plan d'atténuation.
    svc.record_human_action(
        "SUP-2024",
        &HumanActionInput {
            target_step: "aggregate".into(),
            action: HumanAction::Approve,
            justification: "Plan d'atténuation validé (audit sous-tier à T+90j; clause contractuelle sur les rejets aqueux); fournisseur approuvé sous conditions.".into(),
        },
        &off,
        10,
    )
    .expect("approbation scellée");

    // ═══ Cas SUP-2099 — Ferro Components Ltd (BLOQUÉ) — isolation par-cas + cible de l'altération D2 ═══

    svc.ingest_evidence(&ToolCall {
        case_id: "SUP-2099",
        step: "ev-sanctions",
        tool: "sanctions_pep_db",
        query: "screen: Ferro Components Ltd + UBO vs listes consolidees".as_bytes(),
        retrieved_ref: "eu-consolidated://2026-06-28",
        content: "CORRESPONDANCE UBO sur liste consolidee - a confirmer".as_bytes(),
        consumed_by_step: "sanction",
        timestamp: 11,
    })
    .expect("évidence sanctions SUP-2099 scellée");

    // Le verdict FAIL — c'est LUI que l'altération D2 tentera de maquiller en PASS (cacher le hit sanctions).
    svc.ingest_llm(&LlmCall {
        case_id: "SUP-2099",
        step: "sanction",
        agent_role: "sanction_screen",
        inputs: "entite + UBO vs liste consolidee".as_bytes(),
        model: "phi4-mini-local",
        version: "1",
        output: "FAIL — correspondance UBO confirmée sur liste consolidée",
        evidence_steps: &["ev-sanctions"],
        timestamp: 12,
    })
    .expect("criblage SUP-2099 scellé");

    svc.record_human_action(
        "SUP-2099",
        &HumanActionInput {
            target_step: "sanction".into(),
            action: HumanAction::Escalate,
            justification: "Correspondance sanctions confirmée; remonté au service juridique, relation non engagée (LkSG §6).".into(),
        },
        &off,
        13,
    )
    .expect("escalade SUP-2099 scellée");

    // ── EXPORT S2 — le JSONL re-prouvable (la charge scellée complète du tenant) ──
    let exp = svc.export(None, &off, 100).expect("export S2");
    assert!(exp.integrity_ok, "l'export S2 doit être re-prouvé (intègre)");
    exp.records_jsonl
}

/// Écrit le bundle complet sous `dir`. Tout est **écrit à la main** (plutôt que `write_reg_subtree`) pour
/// nommer sans collision les mappings des deux registres (diligence & EU AI Act partagent la version
/// `2026-06`), et pour ajouter les rendus HTML de démo + le README.
fn write_bundle(
    dir: &Path,
    jsonl: &str,
    artifacts: &[(&RegulatoryArtifact, &str)],
    diligence: &MappingSet,
    eu: &MappingSet,
    root: &str,
) -> std::io::Result<()> {
    // Les records scellés (D1) + une copie altérée (D2 : FAIL -> PASS sur le fournisseur bloqué).
    let tampered = jsonl.replacen("FAIL", "PASS", 1);
    assert_ne!(tampered, jsonl, "l'altération doit modifier au moins un octet scellé (FAIL introuvable ?)");
    write(dir, "records.sealed.jsonl", jsonl)?;
    write(dir, "records.TAMPERED.jsonl", &tampered)?;

    // Le vérifieur zéro-dépendance VOYAGE avec le bundle : le destinataire le re-compile HORS-LIGNE depuis
    // ce qu'il a reçu, sans rien croire d'autre que les maths + sa propre compilation (le geste D1).
    write_embedded_verifier(dir)?;

    // Les trois dossiers : la VUE canonique (.md, comparée octet-à-octet) + un rendu HTML de démo.
    for (art, slug) in artifacts {
        write(dir, &format!("reg/{slug}.md"), &render_md(art))?;
        write(dir, &format!("reg/{slug}.html"), &render_html(art))?;
    }

    // Les mappings versionnés qui VOYAGENT (sans eux, le tiers ne peut pas re-dériver) — nommés par registre.
    write(dir, "reg/mapping.diligence.2026-06.json", &(serde_json::to_string_pretty(diligence).expect("mapping sérialisable") + "\n"))?;
    write(dir, "reg/mapping.eu_ai_act.2026-06.json", &(serde_json::to_string_pretty(eu).expect("mapping sérialisable") + "\n"))?;

    // Le mode d'emploi de la re-dérivation (trust-light) + le README de la démo.
    write(dir, "reg/REDERIVE.md", &rederive_md(root, &diligence.set_hash(), &eu.set_hash()))?;
    write(dir, "README.md", &readme_md(dir, root))?;
    Ok(())
}

/// Écrit `dir/rel` en créant les dossiers parents au besoin.
fn write(dir: &Path, rel: &str, content: &str) -> std::io::Result<()> {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)
}

/// Embarque le vérifieur **zéro-dépendance** dans `verify/verify-mini/`, avec un `Cargo.toml` **autonome**
/// (valeurs concrètes, sans héritage de workspace) — pour que le destinataire le **re-compile hors-ligne**
/// depuis le bundle. Les sources sont **baked à la compilation** (`include_str!`) : la copie embarquée est
/// donc **octet-à-octet identique** au `verify-mini` du dépôt Kovant (aucune dérive possible).
fn write_embedded_verifier(dir: &Path) -> std::io::Result<()> {
    // Les 7 fichiers du vérifieur (arbre de dépendances littéralement vide : SHA-256/HMAC + parseur JSONL
    // + encodage canonique, tout std-only) — chemin résolu depuis la crate reg-evidence à la compilation.
    const SRC: &[(&str, &str)] = &[
        ("hmac.rs", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../verify-mini/src/hmac.rs"))),
        ("json.rs", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../verify-mini/src/json.rs"))),
        ("lib.rs", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../verify-mini/src/lib.rs"))),
        ("main.rs", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../verify-mini/src/main.rs"))),
        ("record.rs", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../verify-mini/src/record.rs"))),
        ("sha256.rs", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../verify-mini/src/sha256.rs"))),
        ("verify.rs", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../verify-mini/src/verify.rs"))),
    ];
    // Un manifeste AUTONOME (pas de `{ workspace = true }`) : buildable seul, hors de tout workspace.
    const CARGO_TOML: &str = "# Verifieur forensic zero-dependance — copie AUTONOME embarquee dans le bundle\n\
# (sans heritage de workspace), pour la re-compilation hors-ligne par le destinataire.\n\
# Octet-a-octet identique au verify-mini du depot Kovant. `cargo tree` = verify-mini SEUL.\n\
[package]\n\
name = \"verify-mini\"\n\
version = \"0.0.0\"\n\
edition = \"2021\"\n\
publish = false\n\
\n\
[[bin]]\n\
name = \"verify-mini\"\n\
path = \"src/main.rs\"\n\
\n\
[lib]\n\
path = \"src/lib.rs\"\n\
\n\
[dependencies]\n";

    for (name, content) in SRC {
        write(dir, &format!("verify/verify-mini/src/{name}"), content)?;
    }
    write(dir, "verify/verify-mini/Cargo.toml", CARGO_TOML)?;
    Ok(())
}

/// Le dossier de sortie : `argv[1]` s'il est fourni, sinon `~/Downloads/csddd-demo-bundle`.
fn out_dir() -> PathBuf {
    if let Some(arg) = std::env::args().nth(1) {
        return PathBuf::from(arg);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Downloads").join("csddd-demo-bundle")
}

/// Le `reg/REDERIVE.md` : le geste trust-light, cadré honnêtement (outillé ≠ conforme).
fn rederive_md(root: &str, diligence_hash: &str, eu_hash: &str) -> String {
    format!(
        "# Re-dériver ces dossiers VOUS-MÊME (trust-light — zéro confiance neuve)\n\
\n\
Ces dossiers réglementaires (`reg/*.md`) ne sont **pas** des assertions qu'il faut nous croire : chacun est\n\
une **fonction déterministe** de deux entrées que vous détenez déjà — les **records re-prouvés** et la\n\
**lecture (mapping) versionnée**. Vous les **re-calculez**.\n\
\n\
1. **Re-prouvez** `records.sealed.jsonl` avec la **clé tenant** (obtenue auprès du responsable de\n\
   traitement — elle n'est **pas** dans ce bundle, par souveraineté) via le vérifieur à zéro dépendance :\n\
   `verify-mini records.sealed.jsonl --key-hex <64hex> --root {root}` → doit rendre `exit 0`.\n\
2. **Re-dérivez** : appliquez `reg/mapping.diligence.2026-06.json` (CSDDD & LkSG) ou\n\
   `reg/mapping.eu_ai_act.2026-06.json` (EU AI Act) aux records re-prouvés, en sélectionnant pour chaque\n\
   exigence les nœuds que son sélecteur désigne.\n\
3. **Comparez** : le résultat doit **égaler** le `reg/<cadre>.md` correspondant, octet pour octet.\n\
\n\
- **manifest-root** attendu : `{root}`\n\
- **mapping `set_hash`** — diligence (CSDDD/LkSG) : `{diligence_hash}` · EU AI Act : `{eu_hash}`\n\
\n\
## Ce que ces dossiers SONT, et ce qu'ils NE SONT PAS\n\
\n\
- Ils **fournissent et mappent** l'évidence que l'exigence réclame ; chaque ligne cite un `node_id` scellé\n\
  que vous **recoupez** directement dans les records (ne faites jamais confiance au texte — recoupez-le).\n\
- Ils **ne prononcent pas** la conformité : **outillé ≠ conforme**. La conformité est une conclusion\n\
  juridique (classification du système, conseil du client, obligations organisationnelles que le DAG ne\n\
  produit pas — voir les exigences **HORS-PÉRIMÈTRE-DAG**, qui sont **déclarées, jamais tues**).\n\
- La **lecture** (le mapping) est une **revendication versionnée** de Kovant, datée — pas une vérité. Elle\n\
  voyage ici pour que vous puissiez la **lire, la contester, et la re-jouer**.\n"
    )
}

/// Le `README.md` du bundle — le runbook des trois démos, avec les commandes prêtes à coller.
fn readme_md(dir: &Path, root: &str) -> String {
    let hex = key_hex();
    let d = dir.display();
    format!(
        "# Bundle de démo — diligence fournisseur (CSDDD / LkSG / EU AI Act)\n\
\n\
Un **seul** workflow de diligence fournisseur, scellé au fil de l'exécution (capture S1 + gate humain S2),\n\
**relu sous trois cadres**. Ce bundle est **autoportant** : les trois démos ci-dessous tournent dessus\n\
sans rien préparer d'autre. La **clé tenant n'est pas ici** (souveraineté) — passez-la en `--key-hex`.\n\
\n\
## Contenu\n\
- `records.sealed.jsonl` — les records **scellés & re-prouvables** (le journal du tenant : 2 cas).\n\
- `records.TAMPERED.jsonl` — une copie où **un seul champ scellé** a été retourné (le verdict sanctions du\n\
  fournisseur **bloqué** SUP-2099, maquillé de `FAIL` en `PASS`).\n\
- `verify/verify-mini/` — le **vérifieur à zéro dépendance**, source complète + `Cargo.toml` autonome :\n\
  re-compilez-le **hors-ligne** et re-prouvez **sans nous**.\n\
- `reg/csddd.{{md,html}}`, `reg/lksg.{{md,html}}`, `reg/eu_ai_act.{{md,html}}` — les **dossiers dérivés**\n\
  (VUE `.md` canonique + rendu `.html` de lecture), sur le cas **SUP-2024**.\n\
- `reg/mapping.*.json` — les **lectures versionnées** qui voyagent (pour re-dériver).\n\
- `reg/REDERIVE.md` — comment **re-dériver vous-même**, octet pour octet (trust-light).\n\
\n\
## D1 — la re-preuve sans confiance, HORS-LIGNE (→ exit 0)\n\
Coupez le Wi-Fi. **Re-compilez** le vérifieur depuis ce que vous avez reçu, puis re-prouvez :\n\
```\n\
cargo build --release --offline --manifest-path \"{d}/verify/verify-mini/Cargo.toml\" --target-dir /tmp/vm\n\
/tmp/vm/release/verify-mini \"{d}/records.sealed.jsonl\" --key-hex {hex} --root {root}\n\
```\n\
Arbre de dépendances **vide** → compile en secondes, **sans réseau**. Le `--root` ancre au manifest-root\n\
publié. _(Alternative rapide depuis le dépôt : `cargo run -q -p verify-mini -- <records> --key-hex … --root …`.)_\n\
\n\
## D2 — l'altération détectée (→ exit 1)\n\
```\n\
cargo run -q -p verify-mini -- \"{d}/records.TAMPERED.jsonl\" --key-hex {hex}\n\
```\n\
Un octet scellé a changé : le vérifieur **rejette** le bundle et **pointe le record** altéré. Aucun champ\n\
scellé ne peut être modifié après coup sans que la re-preuve casse.\n\
\n\
## D3 — le dossier réglementaire (la VUE re-dérivable)\n\
```\n\
open \"{d}/reg/csddd.html\"      # ou lksg.html / eu_ai_act.html\n\
```\n\
Le dossier **fournit et mappe** l'évidence (chaque ligne cite un `node_id` scellé), **déclare ses trous**\n\
(HORS-PÉRIMÈTRE-DAG), et n'affirme **jamais** la conformité — **outillé ≠ conforme**. Il est\n\
**re-dérivable** (`reg/REDERIVE.md`) : le destinataire le **re-calcule** au lieu de nous croire.\n\
\n\
- **manifest-root** : `{root}`\n\
- **clé tenant (démo)** : `--key-hex {hex}` — en production, elle vient du responsable de traitement, pas d'un README.\n"
    )
}

/// Imprime le mode d'emploi au terminal (clé + root + trois commandes), avec les chemins résolus.
fn print_demo_guide(dir: &Path, root: &str) {
    let hex = key_hex();
    let d = dir.display();
    println!("\n═══════════════════════════════════════════════════════════════════════════");
    println!("  SEED CSDDD — bundle de démo écrit dans :\n    {d}");
    println!("───────────────────────────────────────────────────────────────────────────");
    println!("  clé tenant (hex) : {hex}");
    println!("  manifest-root    : {root}");
    println!("───────────────────────────────────────────────────────────────────────────");
    println!("  D1  re-preuve HORS-LIGNE (exit 0) — re-compiler le vérifieur depuis le bundle :");
    println!("      cargo build --release --offline --manifest-path \"{d}/verify/verify-mini/Cargo.toml\" --target-dir /tmp/vm");
    println!("      /tmp/vm/release/verify-mini \"{d}/records.sealed.jsonl\" --key-hex {hex} --root {root}");
    println!("      (alternative rapide : cargo run -q -p verify-mini -- \"{d}/records.sealed.jsonl\" --key-hex {hex} --root {root})");
    println!("  D2  altération (exit 1) :");
    println!("      /tmp/vm/release/verify-mini \"{d}/records.TAMPERED.jsonl\" --key-hex {hex}");
    println!("  D3  dossier CSDDD :");
    println!("      open \"{d}/reg/csddd.html\"");
    println!("═══════════════════════════════════════════════════════════════════════════\n");
}
