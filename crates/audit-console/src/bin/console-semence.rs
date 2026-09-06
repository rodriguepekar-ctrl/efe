// SPDX-License-Identifier: UNLICENSED
//! **`console-semence`** — alimenter la console, et savoir la CASSER.
//!
//! ## Pourquoi ce binaire existe
//!
//! `audit-console` sert une vraie interface web, elle est éprouvée par M7 et M8 — et elle était
//! **injoignable** : absente de `compose.yaml`, absente de l'image. La rendre lançable sur un
//! registre VIDE n'aurait rien réglé : on aurait remplacé « on ne peut pas l'ouvrir » par « on
//! l'ouvre et il n'y a rien », ce qui est le même défaut avec un habit neuf.
//!
//! Elle réhydrate son tenant depuis `EFE_CONSOLE_RECORDS` s'il existe, et se provisionne vide
//! sinon. Il fallait donc quelque chose qui écrive ce fichier.
//!
//! ## La clé — et pourquoi ce binaire vit DANS `audit-console`
//!
//! La console dérive sa clé tenant par `TenantKey::derive(EFE_CONSOLE_ROOT, EFE_CONSOLE_TENANT)`.
//! Un semeur qui emploierait une clé fixe — comme le fait l'exemple `emit_demo_case`, avec
//! `[0x2a; 32]` — produirait un fichier que la console **ne saurait pas re-prouver**, et l'échec
//! serait attribué à la chaîne alors qu'il viendrait des clés.
//!
//! D'où : ce binaire lit **les mêmes variables**, par **le même lecteur** (`profile::read`), et
//! appelle **la même dérivation**. Deux programmes qui doivent s'accorder sur une clé n'ont pas le
//! droit de la calculer chacun de leur côté.
//!
//! ## Le mode `--truquer` — c'est ce qui en fait un banc
//!
//! Sans lui, on ne verrait qu'un registre vert, et un registre vert ne prouve pas qu'on saurait
//! reconnaître un registre faux. `--truquer` retourne **un seul champ scellé** : le verdict de
//! criblage sanctions du fournisseur, de `FAIL` à `PASS` — l'altération qu'un fraudeur ferait, et
//! la seule qui change la conclusion du dossier. La re-preuve doit alors ÉCHOUER, et dire où.
//!
//! ```text
//! console-semence                 # un cas conforme, scellé, re-prouvé
//! console-semence --truquer       # le même, un verdict retourné — la re-preuve doit tomber
//! ```
//!
//! Codes : `0` écrit et re-prouvé comme attendu · `1` le contraire de ce qui était attendu ·
//! `2` usage ou écriture impossible.

use audit_console::profile::read;
use provenance_capture::{CaptureStore, HumanReview, LlmCall, MessageEvent, ToolCall};
use provenance_dag::{HumanAction, TenantKey};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let truquer = args.iter().any(|a| a == "--truquer");
    if let Some(inconnu) = args.iter().find(|a| *a != "--truquer") {
        eprintln!("argument inconnu : « {inconnu} »\n\nusage : console-semence [--truquer]");
        return std::process::ExitCode::from(2);
    }

    // LES MÊMES variables, par LE MÊME lecteur que le serveur. Cf. l'en-tête.
    let root = read::value("EFE_CONSOLE_ROOT");
    let tenant = read::value("EFE_CONSOLE_TENANT");
    let chemin = read::value("EFE_CONSOLE_RECORDS");
    if chemin.is_empty() {
        eprintln!("EFE_CONSOLE_RECORDS est vide — il n'y a pas d'endroit où écrire.");
        return std::process::ExitCode::from(2);
    }

    let cle = TenantKey::derive(root.as_bytes(), &tenant);
    let mut store = CaptureStore::new(cle);
    let cas = "SUP-2024-0917";

    // Un cas de diligence fournisseur complet, dans son ordre CAUSAL — c'est cet ordre que la
    // chaîne scelle, et le changer changerait la racine.
    //
    // ⚠ Le verdict de criblage est le champ qui décide de tout : `FAIL` ⟹ le fournisseur est
    // bloqué, `PASS` ⟹ il passe. C'est donc exactement celui qu'un fraudeur retournerait, et
    // c'est celui que `--truquer` retourne.
    let verdict = if truquer { "PASS" } else { "FAIL" };
    let conclusion = if truquer { "COMPLIANT" } else { "BLOQUÉ — criblage sanctions positif" };

    let echec = |quoi: &str| {
        eprintln!("écriture du cas impossible ({quoi}) — le store a refusé un record.");
    };
    if store
        .record_llm(&LlmCall {
            case_id: cas,
            step: "extract",
            agent_role: "supplier_extract",
            inputs: b"facture.pdf",
            model: "phi4-mini",
            version: "1",
            output: "{supplier: ACME GmbH, pays: DE}",
            evidence_steps: &[],
            timestamp: 1,
        })
        .is_err()
    {
        echec("extract");
        return std::process::ExitCode::from(2);
    }
    if store
        .record_evidence(&ToolCall {
            case_id: cas,
            step: "ev-sanctions",
            tool: "eu_sanctions_db",
            query: b"ACME GmbH",
            retrieved_ref: "eu-sanctions://2024-09",
            content: b"match: EU/2024/117",
            consumed_by_step: "sanction",
            timestamp: 2,
        })
        .is_err()
    {
        echec("ev-sanctions");
        return std::process::ExitCode::from(2);
    }
    if store
        .record_llm(&LlmCall {
            case_id: cas,
            step: "sanction",
            agent_role: "sanction_check",
            inputs: b"ACME GmbH; DE",
            model: "phi4-mini",
            version: "1",
            output: verdict,
            evidence_steps: &["ev-sanctions"],
            timestamp: 3,
        })
        .is_err()
    {
        echec("sanction");
        return std::process::ExitCode::from(2);
    }
    if store
        .record_message(&MessageEvent {
            case_id: cas,
            step: "msg-1",
            from_step: "extract",
            to_step: "sanction",
            artifact: b"supplier_record",
            artifact_type: "supplier_record",
            timestamp: 4,
        })
        .is_err()
    {
        echec("msg-1");
        return std::process::ExitCode::from(2);
    }
    if store
        .record_llm(&LlmCall {
            case_id: cas,
            step: "aggregate",
            agent_role: "verdict_aggregate",
            inputs: b"sanction=?",
            model: "phi4-mini",
            version: "1",
            output: conclusion,
            evidence_steps: &[],
            timestamp: 5,
        })
        .is_err()
    {
        echec("aggregate");
        return std::process::ExitCode::from(2);
    }
    if store
        .record_human(&HumanReview {
            case_id: cas,
            step: "review-1",
            human_id: "officer:hanna.svensson",
            action: HumanAction::Approve,
            target_step: "aggregate",
            justification: "revue conforme — criblage sanctions vérifié",
            timestamp: 6,
        })
        .is_err()
    {
        echec("review-1");
        return std::process::ExitCode::from(2);
    }

    let chemin = std::path::Path::new(&chemin);
    if let Err(e) = store.save_local(chemin) {
        eprintln!("écriture de « {} » impossible : {e}", chemin.display());
        return std::process::ExitCode::from(2);
    }

    // ── On RE-PROUVE avant de rendre la main ────────────────────────────────────────────────
    //
    //  Écrire un fichier et dire « c'est fait » sans l'avoir re-prouvé serait exactement le vert
    //  malhonnête que tout ce dépôt interdit. Et le sens du verdict s'inverse avec `--truquer` :
    //  dans ce mode, une re-preuve qui RÉUSSIT est un échec du banc.
    let rp = store.reprove();
    println!("cas       : {cas}");
    println!("tenant    : {tenant}");
    println!("records   : {}", chemin.display());
    println!("racine    : {}", store.manifest_root());
    println!("verdict   : criblage={verdict} · conclusion={conclusion}");
    println!("re-preuve : {}", if rp.ok { "OK" } else { "ÉCHEC" });

    if truquer {
        println!();
        println!("⚠ SEMENCE TRUQUÉE. Le verdict de criblage a été retourné de FAIL à PASS : le");
        println!("  dossier conclut désormais que le fournisseur est conforme alors que l'évidence");
        println!("  scellée dit l'inverse. C'est la fraude que ce produit existe pour attraper.");
        println!();
        println!("  Ce fichier est scellé de façon COHÉRENTE — il se re-prouve. Ce n'est pas une");
        println!("  altération APRÈS COUP : c'est un cas menteur scellé de bonne foi. La chaîne");
        println!("  atteste l'INTÉGRITÉ de ce qui a été enregistré, jamais la CORRECTION de la");
        println!("  décision. Pour voir une altération se faire attraper, il faut modifier le");
        println!("  fichier À LA MAIN après cette commande, puis rejouer verify-mini.");
    }

    if rp.ok {
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!("la re-preuve interne a ÉCHOUÉ sur un fichier qu'on vient d'écrire — ce n'est pas");
        eprintln!("une altération, c'est un défaut du semeur ou du store.");
        std::process::ExitCode::from(1)
    }
}
