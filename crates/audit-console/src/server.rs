// SPDX-License-Identifier: UNLICENSED
//! **Le serveur** — bind **egress-gardé** (souveraineté) puis une boucle `TcpListener` minuscule qui lit
//! une requête, appelle [`crate::handler::handle`], et écrit la réponse. C'est la part la **moins**
//! trust-critique (de l'E/S) ; toute la confiance est dans S2, et `handle` est pur (testé en air-gap). Le
//! framework HTTP est « permis ici » (cf. l'architecture §10) — on choisit néanmoins le **fait main** pour
//! garder l'arbre de dépendances **vide**, cohérent avec le reste de P1.

use crate::app::Console;
use crate::canal::Canal;
use crate::provisioning::{balayer, Deploiement};
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

/// Sert la console sur `addr` (ex. `127.0.0.1:8088`). Boucle bloquante (un thread par connexion ;
/// l'état est partagé sous `Mutex`). Ne rend `Ok` que si le listener s'arrête proprement.
///
/// ## M8 — le bind non-local, et ce qu'il coûte
///
/// `I-S3-SOUVERAIN` exigeait un bind **local**, sans exception : la console ne pouvait pas servir un
/// auditeur distant, donc « TLS » ne pouvait même pas se poser. M8 lève l'interdit — **contre une
/// déclaration explicite** du terminateur de canal placé devant le service (`terminateur`).
///
/// Ce n'est pas une case à cocher. La déclaration est **portante** : elle qualifie chaque connexion
/// en [`Canal::A2ProtegeDeclare`], ce qui est le seul barreau, hors boucle locale, qui franchisse le
/// plancher d'un **scellement**. Sans elle, un pair distant est [`Canal::A3NonProtege`] : il peut
/// lire, il ne peut rien sceller.
///
/// `origines` — facultatif — porte les adresses **d'où le terminateur se connecte**. Quand elles
/// sont déclarées, chaque pair y est confronté : une connexion qui contourne le terminateur retombe
/// en [`Canal::A3NonProtege`] et perd le droit de sceller. Le témoin est `peer_addr`, remplie par le
/// noyau — le client ne peut pas la fabriquer, contrairement à un en-tête.
///
/// **Ce processus ne termine PAS TLS**, et il faut le dire. Le dépôt possède déjà une pile TLS
/// complète (TLS 1.3, mTLS, rotation à chaud) dans `efe-meta-swarm-server`, hors du produit ; en
/// réécrire une ici triplerait l'arbre de dépendances d'un crate feuille et serait moins sûr qu'un
/// terminateur mûr. Ce que M8 refuse, c'est de servir à distance **en silence** ; ce qu'il n'a
/// jamais prétendu, c'est de chiffrer lui-même.
/// Ce qui fait EXPIRER les cas : la déclaration à appliquer, et à quel rythme.
///
/// Sans ce fil, une politique de conservation déclarée ne s'exécuterait **jamais** — un horizon
/// écrit dans un fichier, et rien qui ne l'atteigne. C'est le trou que l'audit M6→M8 a trouvé.
pub struct Balayeur {
    pub intervalle_s: u64,
    pub deploiement: Deploiement,
}

pub fn serve(
    addr: &str,
    console: Console,
    terminateur: Option<String>,
    origines: Vec<std::net::IpAddr>,
    balayeur: Option<Balayeur>,
) -> std::io::Result<()> {
    let declare = terminateur.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    if declare.is_none() {
        // I-S3-SOUVERAIN : sans déclaration, le bind DOIT rester local.
        assert_local(addr).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "{e} — pour servir hors de la boucle locale, DÉCLAREZ le terminateur de canal \
                     placé devant ce service. Servir à distance sans le dire ferait traverser les \
                     jetons de session en clair sans qu'une ligne ne proteste."
                ),
            )
        })?;
    }
    let listener = TcpListener::bind(addr)?;
    let console = Arc::new(Mutex::new(console));
    // Partagées, pas clonées par connexion : la liste est minuscule mais elle est lue à CHAQUE
    // requête, et un `Vec` recopié par connexion serait une allocation par requête pour rien.
    let origines: Arc<[std::net::IpAddr]> = origines.into();
    let inflight = Arc::new(AtomicUsize::new(0));

    // ── M6, câblé : le fil qui fait réellement expirer ──
    //
    // ⚠ Il tient le verrou pendant la réécriture du trousseau, `fsync` compris : toutes les
    // requêtes se sérialisent derrière, quelques millisecondes, une fois par intervalle. C'est
    // assumé — libérer le verrou entre le calcul des expirés et leur broyage ouvrirait une course
    // pour ne gagner qu'une latence qu'une console de conformité ne remarquera pas.
    if let Some(b) = balayeur {
        let console = Arc::clone(&console);
        std::thread::spawn(move || loop {
            // Pas de `.max(1)` ici. Il y en avait un, et il était la SECONDE substitution
            // silencieuse de cet interrupteur : `EFE_CONSOLE_BALAYAGE_S=0` devenait un balayage
            // toutes les secondes — l'inverse de ce que `0` suggère — sans que rien ne le dise.
            // L'invariant « ≥ 1 » appartient désormais à `profile::NUMERIQUES`, qui le DÉCLARE, et à
            // `profile::read::entier`, qui l'applique une fois. Deux gardes pour une même propriété,
            // c'est un garde dont on ne remarque jamais la panne.
            std::thread::sleep(Duration::from_secs(b.intervalle_s));
            let maintenant =
                SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let rapports = {
                let mut g = console.lock().unwrap_or_else(|p| p.into_inner());
                balayer(&mut g, &b.deploiement, maintenant)
            };
            journaliser_balayage(&rapports);
        });
    }
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
                let declare = declare.clone();
                let origines = Arc::clone(&origines);
                std::thread::spawn(move || {
                    let _ = handle_conn(stream, &console, declare.as_deref(), &origines);
                    inflight.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(_) => continue, // une connexion ratée ne tue pas la boucle
        }
    }
    Ok(())
}

/// Journalise un balayage. **Un échec est crié**, un succès reste bref : un journal qui répète
/// « rien à faire » à chaque heure ne se lit plus, et c'est celui-là qui avalera la ligne qui compte.
pub fn journaliser_balayage(rapports: &[crate::provisioning::RapportBalayage]) {
    for r in rapports {
        match &r.echec {
            None => eprintln!(
                "conservation « {} » : {} cas broyé(s), destruction DURABLE (trousseau réécrit)",
                r.tenant,
                r.broyes.len()
            ),
            Some(e) => eprintln!(
                "⚠ conservation « {} » : {} cas ONT EXPIRÉ et n'ont PAS été broyés — {e}\n\
                 \x20  Rien n'a été détruit, ni sur le disque ni en mémoire : broyer en mémoire \n\
                 \x20  pendant que les clés restent sur le disque déclarerait détruit ce qui est \n\
                 \x20  intact, et le redémarrage suivant les ressusciterait.",
                r.tenant,
                r.expires.len()
            ),
        }
    }
}

/// Traite UNE connexion : lit (avec timeout + bornes), dispatche, écrit, ferme.
fn handle_conn(
    stream: TcpStream,
    console: &Arc<Mutex<Console>>,
    terminateur: Option<&str>,
    origines: &[std::net::IpAddr],
) -> std::io::Result<()> {
    // Anti-slowloris minimal : un client muet est coupé.
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;

    let response = match HttpRequest::read_from(&stream) {
        Ok(req) => {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            // Le verrou n'est tenu QUE pendant le dispatch (handle est rapide et pur).
            // M8 — le barreau vient de la CONNEXION, jamais de la requête. Un client qui pourrait
            // annoncer son propre niveau de confiance rendrait l'échelle décorative.
            // `peer_addr` vient du noyau, pas du client : c'est pour ça qu'il peut arbitrer
            // le contournement du terminateur, là où un en-tête ne le pourrait pas.
            let canal = Canal::observer(stream.peer_addr().ok(), terminateur, origines);
            let mut guard = console.lock().unwrap_or_else(|p| p.into_inner());
            handle(&mut guard, &req, now, &canal)
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
        let err = serve("203.0.113.5:8088", c, None, Vec::new(), None).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "endpoint public refusé (souveraineté)");
        assert!(err.to_string().contains("souveraineté"));
    }

    /// **M8** — le refus dit désormais *quel geste* le lève, et pourquoi le taire serait pire.
    #[test]
    fn le_refus_de_bind_indique_la_declaration_qui_le_leve() {
        let mut c = Console::new();
        c.provision("acme", TenantKey::from_bytes([1u8; 32]));
        let msg = serve("203.0.113.5:8088", c, None, Vec::new(), None).unwrap_err().to_string();
        assert!(msg.contains("DÉCLAREZ le terminateur"), "le geste : {msg}");
        assert!(msg.contains("en clair"), "et le danger que le silence ferait courir : {msg}");
    }

    /// Une déclaration BLANCHE n'est pas une déclaration : elle ne lève pas la garde.
    #[test]
    fn une_declaration_blanche_ne_leve_pas_la_garde() {
        for d in [Some(String::new()), Some("   ".to_string())] {
            let mut c = Console::new();
            c.provision("acme", TenantKey::from_bytes([1u8; 32]));
            let e = serve("203.0.113.5:8088", c, d, Vec::new(), None).unwrap_err();
            assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
        }
    }

    #[test]
    fn local_endpoints_pass_the_guard() {
        // On ne lance pas la boucle (bloquante) ; on vérifie juste que la garde laisse passer le loopback.
        assert!(audit_api::is_local_endpoint("127.0.0.1:8088"));
        assert!(audit_api::is_local_endpoint("localhost:8088"));
        assert!(!audit_api::is_local_endpoint("10.0.0.5:8088"));
    }
}
