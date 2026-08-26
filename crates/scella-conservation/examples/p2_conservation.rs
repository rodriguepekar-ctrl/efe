// SPDX-License-Identifier: UNLICENSED
//! # `p2_conservation` — le critère de P2, exécuté de bout en bout, en une commande.
//!
//! Scelle une chaîne, l'archive, **détruit le store**, restaure depuis la seule archive, et laisse
//! sur le disque de quoi faire re-prouver le résultat par `verify-mini` — le vérifieur à
//! dépendances nulles qu'un tiers recompile. Ce programme n'affirme donc rien qu'un autre binaire
//! ne puisse confirmer sans lui.
//!
//! Il dépose aussi une copie **altérée** : sans elle, un contrôle vert ne prouverait que le vert.
//!
//! ```text
//! cargo run -q -p scella-conservation --example p2_conservation -- /chemin/de/travail
//! ```
//!
//! La clé est fixe (démo reproductible) et **imprimée au terminal**, jamais écrite dans l'archive :
//! c'est exactement le chemin par lequel un destinataire l'obtient — auprès du responsable de
//! traitement, pas dans les fichiers qu'on lui remet.

use std::path::PathBuf;

use provenance_dag::{Node, SealedDag, TenantKey};
use scella_conservation::archive::{sauvegarder, Prise};
use scella_conservation::{restaurer, TrousseauMaitre};

/// Les octets de la clé de démonstration. Fixes pour que le contrôle soit reproductible.
const CLE: [u8; 32] = [0x2A; 32];

fn acte(id: &str, role: &str, sortie: &str, t: u64) -> Node {
    Node::AgentAct {
        node_id: id.into(),
        agent_role: role.into(),
        inputs_hash: "h".into(),
        model: "phi4".into(),
        version: "1".into(),
        output: sortie.into(),
        evidence_refs: vec![],
        timestamp: t,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("scella-p2-demo"));
    let store = base.join("store");
    let arch = base.join("archive");
    std::fs::create_dir_all(&store)?;

    let cle = TenantKey::from_bytes(CLE);
    let hex: String = CLE.iter().map(|b| format!("{b:02x}")).collect();

    // ── 1. Un dossier de diligence, scellé au fil de l'exécution ───────────────────────
    let mut dag = SealedDag::new();
    dag.append(&cle, acte("SUP-2024::intake", "supplier_intake", "supplier=Nordwind; DE; rang 1", 1))?;
    dag.append(&cle, acte("SUP-2024::sanction", "sanction_screen", "PASS", 2))?;
    dag.append(&cle, acte("SUP-2024::esg", "esg_risk_assess", "REVIEW", 3))?;
    dag.append(&cle, acte("SUP-2024::agregat", "diligence_aggregate", "CONDITIONNEL", 4))?;
    let racine = dag.manifest_root();
    std::fs::write(store.join("records.jsonl"), format!("{}\n", dag.to_jsonl()))?;
    println!("1. scellé          : {} records, racine {racine}", dag.records().len());

    // ── 2. La sauvegarde ──────────────────────────────────────────────────────────────
    let mut trousseau = TrousseauMaitre::new();
    trousseau.poser(1, cle.clone(), 1_756_166_400)?;
    let prise = Prise {
        tenant: "acme-corp".into(),
        cree_a: 1_756_166_400,
        generation: 1,
        empreinte_trousseau: None,
        ancre: None,
    };
    let s = sauvegarder(dag.records(), &cle, &prise, &arch)?;
    println!("2. archivé         : {}", s.dossier.display());

    // ── 3. LE STORE EST DÉTRUIT ───────────────────────────────────────────────────────
    std::fs::remove_dir_all(&store)?;
    println!("3. store DÉTRUIT   : {} n'existe plus", store.display());

    // ── 4. La restauration, depuis la seule archive ───────────────────────────────────
    let r = restaurer(&arch, &cle);
    if !r.issue.est_restauree() {
        eprintln!("ÉCHEC : {}", r.issue);
        std::process::exit(1);
    }
    println!("4. restauré        : {}", r.issue);

    // ── 5. De quoi faire confirmer par un AUTRE binaire ───────────────────────────────
    // Une copie altérée d'un seul champ scellé : le verdict de criblage, retourné de PASS à FAIL.
    // C'est la falsification la plus tentante, et celle qu'un contrôle doit démasquer.
    let brut = std::fs::read_to_string(arch.join("records.jsonl"))?;
    std::fs::write(base.join("records.ALTERE.jsonl"), brut.replacen("\"PASS\"", "\"FAIL\"", 1))?;

    println!("5. clé (hex)       : {hex}");
    println!("   racine attendue : {racine}");
    println!("   à re-prouver    : {}", arch.join("records.jsonl").display());
    println!("   altéré (témoin) : {}", base.join("records.ALTERE.jsonl").display());
    Ok(())
}
