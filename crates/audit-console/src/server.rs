// SPDX-License-Identifier: UNLICENSED
//! **Le serveur** — bind **egress-gardé** (souveraineté) puis une boucle `TcpListener` minuscule qui lit
//! une requête, appelle [`crate::handler::handle`], et écrit la réponse. C'est la part la **moins**
//! trust-critique (de l'E/S) ; toute la confiance est dans S2, et `handle` est pur (testé en air-gap). Le
//! framework HTTP est « permis ici » (cf. l'architecture §10) — on choisit néanmoins le **fait main** pour
//! garder l'arbre de dépendances **vide**, cohérent avec le reste de P1.

use crate::app::Console;
use crate::handler::handle;
use crate::http::{HttpRequest, HttpResponse};
use audit_api::assert_local;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Plafond de connexions concurrentes — un thread par connexion, mais borné (anti-exhaustion). Au-delà,
/// on refuse net (503) plutôt que d'empiler des threads/tampons sans fin.
const MAX_INFLIGHT: usize = 256;

/// Sert la console sur `addr` (ex. `127.0.0.1:8088`). **Refuse** un endpoint non-local (souveraineté :
/// rien ne sort de la frontière du tenant). Boucle bloquante (un thread par connexion ; l'état est
/// partagé sous `Mutex`). Ne rend `Ok` que si le listener s'arrête proprement.
pub fn serve(addr: &str, console: Console) -> std::io::Result<()> {
    // I-S3-SOUVERAIN : le bind DOIT être local.
    assert_local(addr).map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e.to_string()))?;
    let listener = TcpListener::bind(addr)?;
    let console = Arc::new(Mutex::new(console));
    let inflight = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                // Borne de concurrence : au-delà, on refuse net (503) sans spawn (anti-exhaustion).
                if inflight.load(Ordering::Relaxed) >= MAX_INFLIGHT {
                    let _ = (&stream).write_all(&HttpResponse::html(503, "<!doctype html><h1>503</h1><p>trop de connexions — réessayez</p>").to_bytes());
                    continue;
                }
                inflight.fetch_add(1, Ordering::Relaxed);
                let console = Arc::clone(&console);
                let inflight = Arc::clone(&inflight);
                std::thread::spawn(move || {
                    let _ = handle_conn(stream, &console);
                    inflight.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(_) => continue, // une connexion ratée ne tue pas la boucle
        }
    }
    Ok(())
}

/// Traite UNE connexion : lit (avec timeout + bornes), dispatche, écrit, ferme.
fn handle_conn(stream: TcpStream, console: &Arc<Mutex<Console>>) -> std::io::Result<()> {
    // Anti-slowloris minimal : un client muet est coupé.
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;

    let response = match HttpRequest::read_from(&stream) {
        Ok(req) => {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            // Le verrou n'est tenu QUE pendant le dispatch (handle est rapide et pur).
            let mut guard = console.lock().unwrap_or_else(|p| p.into_inner());
            handle(&mut guard, &req, now)
        }
        Err(e) => HttpResponse::html(400, format!("<!doctype html><h1>400</h1><p>requête invalide : {}</p>", html_escape(&e.to_string()))),
    };

    let mut stream = stream;
    stream.write_all(&response.to_bytes())?;
    stream.flush()?;
    Ok(())
}

/// Échappement minimal pour le message d'erreur de bas niveau (avant que `render` ne soit en jeu).
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use provenance_dag::TenantKey;

    #[test]
    fn refuses_a_non_local_bind() {
        // I-S3-SOUVERAIN : un bind non-local est refusé AVANT toute écoute (rien ne sort de la frontière).
        let mut c = Console::new();
        c.provision("acme", TenantKey::from_bytes([1u8; 32]));
        let err = serve("203.0.113.5:8088", c).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "endpoint public refusé (souveraineté)");
        assert!(err.to_string().contains("souveraineté"));
    }

    #[test]
    fn local_endpoints_pass_the_guard() {
        // On ne lance pas la boucle (bloquante) ; on vérifie juste que la garde laisse passer le loopback.
        assert!(audit_api::is_local_endpoint("127.0.0.1:8088"));
        assert!(audit_api::is_local_endpoint("localhost:8088"));
        assert!(!audit_api::is_local_endpoint("10.0.0.5:8088"));
    }
}
