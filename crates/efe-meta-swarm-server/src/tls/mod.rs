//! TLS / mTLS support for the EFE meta-swarm controller and participants.
//!
//! Implements the design frozen in
//! [`docs/EFE_A1_PHASE2_PR16_ARCHITECTURE.md`][doc] :
//!
//!   * Pure-Rust crypto stack — `rustls` + `tokio-rustls` + `tonic`'s
//!     optional `tls` feature.  No `OpenSSL` FFI in this crate.
//!   * `TLS 1.3`-only by default ; `TLS 1.2` is opt-in via
//!     `EFE_META_SWARM_TLS_MIN_PROTOCOL=1.2`.
//!   * Cipher suite list is the `rustls` `DEFAULT_TLS13_CIPHER_SUITES`
//!     (frozen at chart version — AD30).
//!   * Mutual TLS opt-in via `EFE_META_SWARM_TLS_MUTUAL=true`.  When
//!     enabled, both server and client verify peer cert SANs strictly
//!     against the configured allowlist.
//!   * Hot reload : an `inotify`-backed watcher (with a 5-second
//!     polling fallback on macOS / non-Linux) observes the cert mount
//!     and triggers an atomic [`arc_swap::ArcSwap`] swap of the
//!     active config when cert-manager rotates the underlying
//!     `Secret`.
//!   * Connection drain : in-flight RPCs continue on the old session
//!     keys ; new connections post-rotation pick up the new cert.
//!     30-second grace period (AD10).
//!   * Strict SAN verifier rejects IP-only and wildcard SANs (AD11 /
//!     § 10).
//!
//! ## Module layout
//!
//!   * [`config`]   — strongly-typed [`TlsServerConfig`] +
//!     [`TlsClientConfig`] structs ; rustls config builders.
//!   * [`reader`]   — PEM cert / key / CA bundle parsers.
//!   * [`watcher`]  — file watcher (notify + polling fallback).
//!   * [`reload`]   — [`TlsReloader`] : atomic swap coordinator.
//!   * [`metrics`]  — 4 Prometheus series (handshake duration /
//!     handshake total / cert expiry / reload total).
//!   * [`error`]    — [`TlsError`] enum.
//!
//! ## Lifecycle
//!
//! ```text
//!     startup
//!        ├─ read /etc/efe-meta-swarm/tls/{tls.crt, tls.key, ca.crt}
//!        ├─ build initial TlsServerConfig + TlsClientConfig
//!        ├─ register Prometheus metrics
//!        ├─ spawn cert-expiry sampling loop (60 s cadence)
//!        ├─ spawn inotify watcher (or polling fallback)
//!        └─ wire tonic Server::tls_config + Channel::tls_config
//!
//!     cert change (Day 23 + every ~ 23 days)
//!        ├─ watcher emits CertChangeEvent
//!        ├─ debounce 500 ms (Kubernetes Secrets fire multiple events)
//!        ├─ TlsReloader::handle_cert_change
//!        │     ├─ re-read cert + key + ca
//!        │     ├─ rebuild rustls config
//!        │     └─ ArcSwap atomic store (lock-free for readers)
//!        ├─ tls_reload_total++
//!        └─ OTel span tls.reload recorded
//! ```
//!
//! [doc]: ../../../docs/EFE_A1_PHASE2_PR16_ARCHITECTURE.md

pub mod audit;
pub mod config;
pub mod error;
pub mod metrics;
pub mod reader;
pub mod reload;
pub mod watcher;

pub use audit::{
    default_audit_emitter, AuditRole, MemoryAuditEmitter, NoopAuditEmitter, SharedAuditEmitter,
    TlsAuditEmitter, TlsAuditEvent,
};
pub use config::{
    build_rustls_client_config, build_rustls_server_config, ProtocolVersion, StrictSanVerifier,
    TlsClientConfig, TlsServerConfig,
};
pub use error::TlsError;
pub use metrics::{ReloadOutcome, TlsMetrics};
pub use reader::{load_ca_bundle, load_cert_chain, load_private_key, parse_cert_expiry};
pub use reload::{ReloadResult, TlsReloadEvent, TlsReloader};
pub use watcher::{CertChangeEvent, CertWatcher};

/// Environment-variable name that requests TLS at runtime.  When
/// `true`, the binaries refuse to start without a valid cert+key
/// pair (no plain-text fallback — AD25).
pub const ENV_TLS_REQUIRED: &str = "EFE_META_SWARM_TLS_REQUIRED";

/// Environment variable pointing at the leaf+intermediate cert chain
/// PEM file.  Provisioned by cert-manager at
/// `/etc/efe-meta-swarm/tls/tls.crt` by default.
pub const ENV_TLS_CERT: &str = "EFE_META_SWARM_TLS_CERT";

/// Environment variable pointing at the matching private key PEM
/// file.  Defaults to `/etc/efe-meta-swarm/tls/tls.key`.
pub const ENV_TLS_KEY: &str = "EFE_META_SWARM_TLS_KEY";

/// Environment variable pointing at the CA bundle PEM file used to
/// verify peer certs.  Defaults to `/etc/efe-meta-swarm/tls/ca.crt`.
pub const ENV_TLS_CA: &str = "EFE_META_SWARM_TLS_CA";

/// Environment variable enabling mutual TLS (client cert
/// verification).  Defaults to `false` (server-side TLS only).
pub const ENV_TLS_MUTUAL: &str = "EFE_META_SWARM_TLS_MUTUAL";

/// Environment variable selecting the minimum protocol version.
/// Accepts `"1.2"` or `"1.3"`.  Defaults to `"1.3"`.
pub const ENV_TLS_MIN_PROTOCOL: &str = "EFE_META_SWARM_TLS_MIN_PROTOCOL";

/// Environment variable carrying a comma-separated list of allowed
/// peer SANs.  Used by [`StrictSanVerifier`] when mutual TLS is on.
pub const ENV_TLS_ALLOWED_PEER_SANS: &str = "EFE_META_SWARM_TLS_ALLOWED_PEER_SANS";

/// Environment variable carrying the expected SAN of the **server**
/// the controller dials.  Required when `EFE_META_SWARM_TLS_REQUIRED=true`
/// on the client side (controller-as-client to participant-as-server).
pub const ENV_TLS_PEER_SAN: &str = "EFE_META_SWARM_TLS_PEER_SAN";

/// Default debounce window for the cert-file watcher.  Kubernetes
/// Secret mounts fire multiple inotify events on a single rotation
/// (RENAME + CREATE + WRITE within ~ 100 ms).  A 500 ms window
/// collapses them to a single reload (§ 18.2).
pub const WATCHER_DEBOUNCE_MS: u64 = 500;

/// Default cadence at which the cert-expiry sampling loop refreshes
/// the `efe_meta_swarm_tls_cert_expiry_seconds` gauge.
pub const CERT_EXPIRY_SAMPLE_INTERVAL_S: u64 = 60;

/// Default 30-second drain grace period quoted in AD10.
pub const DEFAULT_DRAIN_GRACE_S: u64 = 30;

/// AD18 / AC25 / T44 — `true` lorsque l'endpoint désigne le **loopback** (IPv4 `127.0.0.0/8`,
/// IPv6 `::1`, ou `localhost`). Une connexion loopback contourne TLS quel que soit `tls.enabled`.
///
/// Le raisonnement (architecture § 30 T44) : le surcoût intra-pod n'est pas négligeable et le
/// modèle de menace de `127.0.0.1` est trivialement confiant — quiconque peut composer le loopback
/// d'un Pod possède déjà la portée de confiance de la charge elle-même. La production Kubernetes
/// ne route pas par le loopback ; ceci ne se déclenche qu'en développement et dans les tests
/// mono-processus.
///
/// # M1.b — pourquoi ceci est un RÉ-EXPORT et non une fonction
///
/// Ce nom désignait la **cinquième** copie d'un analyseur d'autorité du dépôt, et la dernière à
/// décider quelque chose d'aussi lourd que « TLS peut-il être retiré ». Elle était **fausse**, et
/// pas d'une manière théorique : `rsplit_once(':')` coupait au **dernier** deux-points, donc
///
/// ```text
/// http://127.0.0.1:8080@evil.com/
///        └──────── « hôte » ────┘ selon l'ancienne lecture : 127.0.0.1
///                              └── selon `tonic::transport::Endpoint` : evil.com
/// ```
///
/// `IpAddr::from_str` recevait un `127.0.0.1` parfaitement propre, répondait « loopback », et
/// `RemoteParticipant::connect_with_tls` **retirait TLS** — du gRPC en clair vers `evil.com`,
/// annoncé au journal en `debug!`. La dette de M1.a affirmait que ces deux analyseurs étaient
/// « sûrs aujourd'hui par composition » : pour celui-ci, la composition invoquée était exactement
/// ce qui le faisait échouer. Cinq formes mesurées contre le vrai client, cinq contournements.
///
/// Un *wrapper* aurait laissé un corps où la logique peut repousser. Un **ré-export n'a pas de
/// corps** : il n'existe plus qu'un seul analyseur dans le dépôt, et
/// `scella-authority/tests/un_seul_analyseur.rs` (exemptions **vides** depuis M1.b) interdit qu'un
/// second réapparaisse.
///
/// ## Ce qui change, observable
///
/// | forme | avant | après | pourquoi |
/// |---|---|---|---|
/// | `http://127.0.0.1:8080@evil.com` | loopback ⟹ **TLS retiré** | non locale ⟹ TLS gardé | le `userinfo` se coupe au PREMIER `@` |
/// | `::1` (IPv6 **nue**) | loopback | non locale ⟹ TLS gardé | la RFC exige les crochets ; `tonic` et `url` la refusent |
/// | `2130706433`, `0177.0.0.1`, `127.1` | non locale | non locale (**refus** explicite) | écritures qu'aucun résolveur ne lit pareil |
/// | `[::0001]` | non locale | non locale (**refus** explicite) | même adresse, autre texte : faux pour l'identité |
/// | `127.0.0.1`, `[::1]:p`, `localhost`, `127.1.2.3` | loopback | loopback | inchangé |
///
/// Les trois familles nouvellement refusées vont toutes dans le sens **fail-closed** : TLS reste
/// allumé là où il pouvait être coupé. Aucune n'apparaît dans une charte ni dans un fichier de
/// configuration du dépôt — seulement dans les tests des analyseurs supprimés.
pub use scella_authority::is_local_endpoint as endpoint_is_loopback;

#[cfg(test)]
mod loopback_tests {
    use super::endpoint_is_loopback;

    // ─── Les tests HISTORIQUES, conservés, pointés sur la nouvelle implémentation ──────────
    //
    // Non réécrits : c'est ce qui en fait une preuve de non-régression. Deux seulement ont dû
    // changer, et chacun porte sa raison.

    #[test]
    fn loopback_ipv4_variants_detected() {
        assert!(endpoint_is_loopback("http://127.0.0.1:50080"));
        assert!(endpoint_is_loopback("https://127.0.0.1:50080"));
        assert!(endpoint_is_loopback("grpc://127.0.0.1"));
        assert!(endpoint_is_loopback("127.0.0.1:50080"));
        assert!(endpoint_is_loopback("http://127.1.2.3:50080")); // 127/8 is loopback
    }

    #[test]
    fn loopback_ipv6_variants_detected() {
        assert!(endpoint_is_loopback("http://[::1]:50080"));
        assert!(endpoint_is_loopback("https://[::1]:50080"));
        assert!(endpoint_is_loopback("[::1]:50080"));
    }

    /// ⭐ **CHANGEMENT ASSUMÉ (M1.b).** L'ancien test exigeait `endpoint_is_loopback("::1")`.
    ///
    /// Une IPv6 **nue** n'est pas une autorité valide : la RFC 3986 exige les crochets, et
    /// `tonic::transport::Endpoint` comme `url` la refusent. L'ancienne lecture rendait pire
    /// qu'un refus — sur `fe80::1:8080` elle rendait l'hôte **`fe80`**, une chaîne qui ne
    /// désigne rien et qui aurait été journalisée puis comparée à une épingle TLS.
    ///
    /// Le sens du changement est **fail-closed** : ce qui était « local, TLS retiré » devient
    /// « illisible, TLS gardé ». Cette forme n'apparaît dans aucune charte ni configuration du
    /// dépôt : la seule chose qu'elle cassait était l'assertion ci-dessus.
    #[test]
    fn ipv6_nue_nest_plus_locale_et_cest_un_durcissement() {
        assert!(!endpoint_is_loopback("::1"), "IPv6 nue : illisible, donc non locale");
        assert!(!endpoint_is_loopback("::1:8080"));
        assert!(!endpoint_is_loopback("fe80::1:8080"));
        // La forme LÉGITIME, elle, reste reconnue — c'est ce qui distingue un durcissement
        // d'une régression.
        assert!(endpoint_is_loopback("[::1]:8080"));
        assert!(endpoint_is_loopback("http://[::1]"));
    }

    #[test]
    fn localhost_treated_as_loopback() {
        assert!(endpoint_is_loopback("http://localhost:50080"));
        assert!(endpoint_is_loopback("LOCALHOST:50080"));
    }

    #[test]
    fn non_loopback_addresses_are_not_loopback() {
        assert!(!endpoint_is_loopback("http://controller.efe.svc:50080"));
        assert!(!endpoint_is_loopback("http://10.0.0.1:50080"));
        assert!(!endpoint_is_loopback("http://192.168.1.1:50080"));
        assert!(!endpoint_is_loopback("http://[2001:db8::1]:50080"));
    }

    #[test]
    fn malformed_endpoints_are_not_loopback() {
        // Fail-safe : dans le doute, TLS reste allumé.
        assert!(!endpoint_is_loopback(""));
        assert!(!endpoint_is_loopback("not a url"));
    }

    #[test]
    fn endpoint_with_path_still_detected() {
        assert!(endpoint_is_loopback("http://127.0.0.1:50080/grpc"));
        assert!(endpoint_is_loopback("http://[::1]:50080/grpc"));
    }

    // ─── Ce que M1.b ferme ────────────────────────────────────────────────────────────────

    /// ⭐⭐ **Le défaut, nommé et mort.** Chacune de ces cinq formes faisait dire « loopback »
    /// à l'ancien analyseur alors que `tonic::transport::Endpoint` part vers `evil.com` —
    /// mesuré, pas supposé (cf. `tests/m1b_pas_de_degradation_silencieuse.rs`, qui le vérifie
    /// contre le vrai client plutôt que contre une liste écrite à la main).
    #[test]
    fn le_userinfo_a_port_ne_coupe_plus_tls() {
        for charge in [
            "http://127.0.0.1:8080@evil.com/",
            "http://127.0.0.1:8080@evil.com",
            "https://127.0.0.1:443@evil.com/grpc",
            "http://localhost:8080@evil.com/",
            "http://[::1]:8080@evil.com/",
        ] {
            assert!(
                !endpoint_is_loopback(charge),
                "{charge} : l'hôte contacté est evil.com — le déclarer local RETIRE TLS"
            );
        }
    }

    /// Le loopback de façade : un **nom** qui commence par `127.` résout où son propriétaire veut.
    #[test]
    fn le_loopback_de_facade_reste_refuse() {
        assert!(!endpoint_is_loopback("http://127.evil.com:50080"));
        assert!(!endpoint_is_loopback("http://localhost.evil.com:50080"));
        assert!(!endpoint_is_loopback("http://2130706433:50080"));
        assert!(!endpoint_is_loopback("http://0177.0.0.1:50080"));
        assert!(!endpoint_is_loopback("http://127.1:50080"));
    }

    /// **Le corpus de conformité de M1**, passé par le nom que ce crate expose.
    ///
    /// Tant que la ligne ci-dessus est un `pub use`, ce test est une tautologie — et c'est
    /// précisément son emploi : il cesse de l'être à la seconde où quelqu'un redonne un **corps**
    /// à `endpoint_is_loopback`. C'est le fil qui casse si la copie revient sous forme de wrapper,
    /// là où le garde `"://"` ne verrait qu'un appel de fonction.
    #[test]
    fn le_corpus_de_localite_passe_par_le_nom_de_ce_crate() {
        assert!(
            scella_authority::CORPUS_LOCALITE.len() >= 20,
            "corpus suspicieusement court ({}) — un test vert sur rien",
            scella_authority::CORPUS_LOCALITE.len()
        );
        for (entree, attendu) in scella_authority::CORPUS_LOCALITE {
            assert_eq!(
                endpoint_is_loopback(entree),
                *attendu,
                "{entree} : le nom exposé par ce crate a divergé de l'analyseur unique"
            );
        }
    }
}
