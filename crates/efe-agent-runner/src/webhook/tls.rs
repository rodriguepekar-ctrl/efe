//! TLS material reload helper.
//!
//! cert-manager writes `tls.crt` + `tls.key` into a mounted Secret. We
//! watch the directory via `inotify` and reload the rustls server config
//! on change. New connections use the new cert ; in-flight requests
//! finish on the old one.

use rustls::ServerConfig;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct TlsMaterial {
    pub cert_path: std::path::PathBuf,
    pub key_path: std::path::PathBuf,
}

pub fn load(
    material: &TlsMaterial,
) -> Result<Arc<ServerConfig>, Box<dyn std::error::Error + Send + Sync>> {
    // Ensure a process-wide CryptoProvider is installed (idempotent).
    static INIT: once_cell::sync::OnceCell<()> = once_cell::sync::OnceCell::new();
    INIT.get_or_init(|| {
        // Safe to ignore the result : `install_default` errors only if
        // a provider is already installed, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });

    let cert_pem = std::fs::read(&material.cert_path)?;
    let key_pem = std::fs::read(&material.key_path)?;

    let mut cert_reader = std::io::Cursor::new(cert_pem);
    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut cert_reader).collect::<Result<Vec<_>, _>>()?;
    if certs.is_empty() {
        return Err("no certs in cert-manager Secret".into());
    }

    let mut key_reader = std::io::Cursor::new(key_pem);
    let key = match rustls_pemfile::private_key(&mut key_reader)? {
        Some(k) => k,
        None => return Err("no private key in cert-manager Secret".into()),
    };

    let cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    Ok(Arc::new(cfg))
}

/// Spawn a watcher on the cert dir. Sends a fresh `Arc<ServerConfig>`
/// over the channel each time the cert is rewritten.
pub fn spawn_watcher(material: TlsMaterial) -> watch::Receiver<Arc<ServerConfig>> {
    let initial = load(&material).expect("initial TLS load failed");
    let (tx, rx) = watch::channel(initial);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        let mut last_mtime: Option<std::time::SystemTime> = None;
        loop {
            interval.tick().await;
            let mtime = std::fs::metadata(&material.cert_path)
                .and_then(|m| m.modified())
                .ok();
            if mtime != last_mtime {
                if let Some(t) = mtime { last_mtime = Some(t); }
                match load(&material) {
                    Ok(cfg) => {
                        tracing::info!(
                            cert = %material.cert_path.display(),
                            "TLS material reloaded"
                        );
                        let _ = tx.send(cfg);
                    }
                    Err(e) => {
                        tracing::error!(?e, "failed to reload TLS material");
                    }
                }
            }
        }
    });
    rx
}
