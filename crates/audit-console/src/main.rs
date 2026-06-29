// SPDX-License-Identifier: UNLICENSED
//! **Le binaire de la console** — câble l'état et sert (bind **local** uniquement). Le backend détient la
//! clé tenant (souveraineté) ; il provisionne le registre S2, optionnellement **réhydrate** un log scellé
//! persisté (`EFE_CONSOLE_RECORDS`), enregistre une session, et lance le serveur. La console **ne capte
//! pas** de données — la capture (S1) est alimentée par le runtime ; ici on **sert** ce qui est scellé.
//!
//! Variables d'environnement :
//! - `EFE_CONSOLE_ADDR`    (défaut `127.0.0.1:8088`) — **doit** être local (souveraineté).
//! - `EFE_CONSOLE_TENANT`  (défaut `acme`)
//! - `EFE_CONSOLE_ROOT`    (défaut `demo-root-secret`) — le secret racine d'où dériver la clé tenant.
//! - `EFE_CONSOLE_TOKEN`   (défaut `demo-token`) — le jeton de session (un IdP réel le remplace).
//! - `EFE_CONSOLE_SUBJECT` (défaut `officer:demo`)
//! - `EFE_CONSOLE_RECORDS` (optionnel) — chemin d'un `records.sealed.jsonl` à réhydrater.

use audit_console::{Console, serve, Role};
use provenance_dag::{SealedDag, TenantKey};

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn main() -> std::io::Result<()> {
    let addr = env_or("EFE_CONSOLE_ADDR", "127.0.0.1:8088");
    let tenant = env_or("EFE_CONSOLE_TENANT", "acme");
    let root = env_or("EFE_CONSOLE_ROOT", "demo-root-secret");
    let token = env_or("EFE_CONSOLE_TOKEN", "demo-token");
    let subject = env_or("EFE_CONSOLE_SUBJECT", "officer:demo");

    // La clé est dérivée dans le backend (déploiement réel : un KMS), jamais exposée au navigateur.
    let key = TenantKey::derive(root.as_bytes(), &tenant);
    let mut console = Console::new();

    match std::env::var("EFE_CONSOLE_RECORDS") {
        Ok(path) => {
            let jsonl = std::fs::read_to_string(&path)?;
            let records = SealedDag::records_from_jsonl(&jsonl).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            console.rehydrate(&tenant, key, records);
            eprintln!("réhydraté le tenant « {tenant} » depuis {path}");
        }
        Err(_) => {
            console.provision(&tenant, key);
            eprintln!("tenant « {tenant} » provisionné (vide — alimenté par la capture S1 du runtime)");
        }
    }
    console.add_session(&token, &tenant, Role::ComplianceOfficer, &subject);

    eprintln!("console de conformité (S3) — http://{addr}");
    eprintln!("  tenant « {tenant} » · jeton « {token} » (en-tête Authorization: Bearer {token})");
    eprintln!("  souveraineté : bind LOCAL uniquement ; la clé tenant ne quitte pas le backend.");
    eprintln!("  « montré vérifié ≠ prouvé sans confiance » — pour une preuve opposable, exportez.");
    serve(&addr, console)
}
