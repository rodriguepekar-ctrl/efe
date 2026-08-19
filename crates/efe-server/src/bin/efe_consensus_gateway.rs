// SPDX-License-Identifier: UNLICENSED
//! # `efe-consensus-gateway` — le binaire qui SERT `/v1/consensus`.
//!
//! ```text
//! EFE_META_SWARM_ENABLED=true \
//! EFE_META_SWARM_CONTROLLER_ENDPOINT=http://127.0.0.1:50080 \
//! EFE_CONSENSUS_GATEWAY_ADDR=127.0.0.1:8090 \
//!   cargo run -p efe-server --bin efe-consensus-gateway
//! ```
//!
//! ## Pourquoi ce binaire existe
//!
//! `efe-server` **fournit** dix-sept routeurs `v1::*` et n'en assemble aucun : chaque déploiement
//! monte ce dont il a besoin. C'est une convention défendable, mais elle a un effet de bord que
//! l'audit de cette couche a nommé — une capacité livrée, testée, et que *personne n'exécute*.
//! `/v1/consensus` était dans ce cas. Ce binaire le corrige de la manière la plus simple qui
//! reste honnête : il ne monte QUE cette route, avec l'état dont elle a besoin, et rien d'autre.
//!
//! Il ne se greffe pas sur `efe-server`'s `main.rs` : celui-là est la passerelle REST **agent**,
//! avec son propre état Postgres et sa propre raison d'être. Y ajouter le consensus aurait mêlé
//! deux préoccupations pour économiser un fichier.
//!
//! ## Fail-closed au boot
//!
//! Sans `EFE_META_SWARM_ENABLED=true`, le sous-système est éteint et la route répondrait `503` à
//! chaque appel. Plutôt que de démarrer un serveur qui ne peut rien faire, on **refuse de
//! démarrer** en nommant la variable : une passerelle qui écoute mais ne sert jamais est plus
//! difficile à diagnostiquer qu'une qui ne démarre pas.

use std::net::SocketAddr;
use std::sync::Arc;

use efe_server::config::ServerConfig;
use efe_server::services::meta_swarm::{MetaSwarmClient, MetaSwarmMetrics};

/// Adresse d'écoute — même convention que les autres passerelles du dépôt.
const ADDR_ENV: &str = "EFE_CONSENSUS_GATEWAY_ADDR";
const DEFAULT_ADDR: &str = "127.0.0.1:8090";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    )
    .init();

    let config = ServerConfig::from_env()?;

    if !config.meta_swarm.enabled {
        return Err(format!(
            "sous-système méta-essaim DÉSACTIVÉ — /v1/consensus répondrait 503 à chaque appel.\n\
             Cette passerelle refuse de démarrer plutôt que d'écouter sans rien pouvoir servir : \
             une passerelle muette est plus difficile à diagnostiquer qu'une absente.\n\
             Remède : EFE_META_SWARM_ENABLED=true (+ EFE_META_SWARM_CONTROLLER_ENDPOINT si le \
             contrôleur n'est pas sur {}).",
            config.meta_swarm.controller_endpoint,
        )
        .into());
    }

    // La connexion est établie AU BOOT, pas à la première requête : un contrôleur injoignable
    // doit se voir au démarrage, quand un opérateur regarde, et non au premier appel client.
    let metrics = MetaSwarmMetrics::register_into_default_registry()
        .map_err(|e| format!("enregistrement des métriques méta-essaim : {e}"))?;
    let client = MetaSwarmClient::connect(&config.meta_swarm, metrics)
        .await
        .map_err(|e| format!("connexion au contrôleur méta-essaim : {e}"))?;
    tracing::info!(endpoint = %client.endpoint(), "contrôleur méta-essaim joint");

    let router = efe_server::v1::consensus::router(Some(Arc::new(client)));

    let addr: SocketAddr = std::env::var(ADDR_ENV)
        .unwrap_or_else(|_| DEFAULT_ADDR.to_string())
        .parse()
        .map_err(|e| format!("{ADDR_ENV} illisible : {e}"))?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "passerelle de consensus en écoute sur POST /v1/consensus");
    println!("EFE · passerelle de consensus — POST http://{addr}/v1/consensus");
    println!("  corps : {{\"meta_dim\": 8, \"include_receipt\": true}}");
    println!("  le reçu rendu se re-prouve avec : verify_meta_mini <steps> <manifest> --key-hex <clé tenant>");

    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("arrêt demandé — drainage des connexions");
        })
        .await?;
    Ok(())
}
