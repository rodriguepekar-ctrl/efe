// SPDX-License-Identifier: UNLICENSED
//! **R3.S7 — Serveur HTTP `/metrics` (Prometheus pull).**
//!
//! Serveur HTTP/1.1 minimal sur `tokio::net::TcpListener`, **zéro dépendance**
//! (même pattern que le mock `local_slm.rs`). Gated derrière la feature
//! `metrics` (qui n'active que `tokio/net`).
//!
//! Routes :
//! - `GET /metrics` → 200 `text/plain; version=0.0.4` + rendu Prometheus.
//! - `GET /healthz` → 200 (liveness).
//! - autre → 404.
//!
//! Le gauge d'état des circuits est lu **au scrape** (pull) via
//! [`snapshot_circuits`] : les circuits changent d'état de façon asynchrone, un
//! gauge doit refléter l'état courant.
//!
//! Bind par défaut `127.0.0.1:9090` (localhost — pas d'exposition réseau
//! directe ; le scraping sécurisé passe par un sidecar/reverse proxy).

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use efe_nano_app_coordinator::SlmCircuitState;

use crate::coordinator::CoordinatorClients;
use crate::metering::MetricsRegistry;

/// Adresse d'écoute par défaut (localhost).
pub const DEFAULT_METRICS_ADDR: &str = "127.0.0.1:9090";

/// Plafond de lecture de la request-line (anti slowloris trivial).
const MAX_REQUEST_BYTES: usize = 8 * 1024;

/// Snapshot scrape-time des états de circuits, trié (ordre stable).
#[must_use]
pub fn snapshot_circuits(clients: &CoordinatorClients) -> Vec<(String, SlmCircuitState)> {
    let mut v: Vec<(String, SlmCircuitState)> = clients
        .slm_circuits
        .iter()
        .map(|(robot, cb)| (robot.clone(), cb.state()))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

/// Lance le serveur `/metrics`. Boucle infinie (`accept`) ; l'app hôte la
/// `tokio::spawn`. Une connexion défaillante ne tue pas la boucle.
///
/// ## Exemple
/// ```ignore
/// let reg = Arc::new(MetricsRegistry::new());
/// let coord = Coordinator::builder().metrics(reg.clone()).build()?;
/// let addr = DEFAULT_METRICS_ADDR.parse().unwrap();
/// tokio::spawn(serve_metrics(addr, reg, coord.clients.clone()));
/// ```
pub async fn serve_metrics(
    addr: SocketAddr,
    registry: Arc<MetricsRegistry>,
    clients: Arc<CoordinatorClients>,
) -> std::io::Result<()> {
    // Bind puis délègue à la boucle canonique (testée via `serve_metrics_on`).
    // Aucune duplication de logique d'accept/handle.
    let listener = TcpListener::bind(addr).await?;
    serve_metrics_on(listener, registry, clients).await
}

/// **Boucle canonique** : sert sur un `TcpListener` déjà bindé. Utilisée par
/// [`serve_metrics`] (après bind) ET par les tests (bind `:0` pour récupérer le
/// port éphémère). Une connexion défaillante ne tue pas la boucle `accept`.
pub async fn serve_metrics_on(
    listener: TcpListener,
    registry: Arc<MetricsRegistry>,
    clients: Arc<CoordinatorClients>,
) -> std::io::Result<()> {
    loop {
        let (sock, _peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(target: "efe_coordinator::metrics", "accept error: {e}");
                continue;
            }
        };
        let reg = registry.clone();
        let cli = clients.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(sock, &reg, &cli).await {
                tracing::debug!(target: "efe_coordinator::metrics", "conn error: {e}");
            }
        });
    }
}

async fn handle_conn(
    mut sock: TcpStream,
    registry: &MetricsRegistry,
    clients: &CoordinatorClients,
) -> std::io::Result<()> {
    // Lit jusqu'à la fin des headers ("\r\n\r\n") ou le cap d'octets.
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() >= MAX_REQUEST_BYTES {
            break;
        }
    }

    let head = String::from_utf8_lossy(&buf);
    let request_line = head.lines().next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    let (status, content_type, body): (&str, &str, String) = match (method, path) {
        ("GET", "/metrics") => {
            let circuits = snapshot_circuits(clients);
            (
                "200 OK",
                "text/plain; version=0.0.4",
                registry.render_prometheus(&circuits),
            )
        }
        ("GET", "/healthz") => ("200 OK", "text/plain", "ok\n".to_string()),
        _ => ("404 Not Found", "text/plain", "not found\n".to_string()),
    };

    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    sock.write_all(resp.as_bytes()).await?;
    sock.flush().await?;
    Ok(())
}
