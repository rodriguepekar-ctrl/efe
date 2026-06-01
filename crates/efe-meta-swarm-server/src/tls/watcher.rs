//! Cert-file watcher.
//!
//! On Linux this is a thin wrapper around `notify`'s `inotify`
//! backend.  On non-Linux platforms (macOS in dev, BSDs in
//! customer-managed environments) `notify`'s `FSEvents` / `kqueue`
//! backends do the equivalent.  The polling fallback ([`PollWatcher`])
//! is provided for the rare case where the host kernel doesn't expose
//! any file-change API at all (user-mode K8s, container-in-container
//! demos).
//!
//! ## Debouncing
//!
//! Kubernetes Secret mounts produce a burst of inotify events on a
//! single rotation — typically a RENAME (atomically swapping the
//! `..data` symlink) followed by 1-2 WRITE events on the
//! materialised files.  All of those land within ~ 100 ms.  We
//! debounce to a single emission per [`super::WATCHER_DEBOUNCE_MS`]
//! window per file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use super::{TlsError, WATCHER_DEBOUNCE_MS};

/// A debounced cert-file change event surfaced by the watcher.
#[derive(Debug, Clone)]
pub struct CertChangeEvent {
    /// The cert file that changed.
    pub path: PathBuf,
    /// The raw `notify` event kind (informational).
    pub kind: String,
    /// Wall-clock time the event was observed.
    pub at: SystemTime,
}

/// Watches one or more cert paths and emits debounced
/// [`CertChangeEvent`]s.
///
/// The watcher owns the `notify::Watcher` handle and a debounce
/// table.  Drop the [`CertWatcher`] to stop watching.
pub struct CertWatcher {
    _inner: Box<dyn Watcher + Send + Sync>,
    paths: Vec<PathBuf>,
}

impl std::fmt::Debug for CertWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertWatcher")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

impl CertWatcher {
    /// Start watching every path in `paths`.  Returns the watcher
    /// (keep it alive for the duration of the watch) and a receiver
    /// that produces debounced events.
    ///
    /// The receiver channel is bounded at 64.  Bursts beyond that
    /// are dropped with a `tracing::warn!` — a single rotation should
    /// never produce > 4 events post-debounce.
    pub fn watch(paths: Vec<PathBuf>) -> Result<(Self, mpsc::Receiver<CertChangeEvent>), TlsError> {
        if paths.is_empty() {
            return Err(TlsError::Watcher("no paths to watch".into()));
        }

        let (raw_tx, raw_rx) = mpsc::channel::<RawEvent>(64);
        let (out_tx, out_rx) = mpsc::channel::<CertChangeEvent>(64);

        // Spawn the debounce task.
        tokio::spawn(debounce_loop(raw_rx, out_tx));

        // Build the platform-native watcher.  `notify`'s recommended
        // backend is `inotify` on Linux, `FSEvents` on macOS,
        // `ReadDirectoryChangesW` on Windows, `kqueue` on the BSDs.
        let raw_tx_clone = raw_tx.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    if is_interesting_event(&event.kind) {
                        for p in event.paths {
                            let raw = RawEvent {
                                path: p,
                                kind: format!("{:?}", event.kind),
                                at: Instant::now(),
                            };
                            // Use blocking_send because notify's
                            // callback is sync.
                            if let Err(e) = raw_tx_clone.blocking_send(raw) {
                                warn!(error = %e, "cert-watcher tx drop");
                            }
                        }
                    }
                }
            },
            notify::Config::default()
                .with_poll_interval(Duration::from_secs(5))
                .with_compare_contents(false),
        )
        .map_err(|e| TlsError::Watcher(e.to_string()))?;

        // Watch each path's PARENT directory (Kubernetes Secret
        // mounts use a `..data` symlink swap on rotation ; watching
        // the leaf file directly misses the change in some kernels).
        let mut watched = std::collections::HashSet::new();
        for path in &paths {
            let parent = path.parent().ok_or_else(|| {
                TlsError::Watcher(format!("path {} has no parent", path.display()))
            })?;
            if watched.insert(parent.to_path_buf()) {
                watcher
                    .watch(parent, RecursiveMode::NonRecursive)
                    .map_err(|e| TlsError::Watcher(e.to_string()))?;
            }
        }

        Ok((
            Self {
                _inner: Box::new(watcher),
                paths,
            },
            out_rx,
        ))
    }

    /// Return the list of watched paths (for introspection / logs).
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
}

/// Internal raw-event type emitted by the `notify` callback before
/// the debounce stage.
#[derive(Debug)]
struct RawEvent {
    path: PathBuf,
    kind: String,
    at: Instant,
}

fn is_interesting_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) | EventKind::Other
    )
}

/// Collapse bursts of inotify events to a single debounced emission
/// per file per [`WATCHER_DEBOUNCE_MS`] window.
async fn debounce_loop(mut rx: mpsc::Receiver<RawEvent>, out: mpsc::Sender<CertChangeEvent>) {
    let mut last_seen: HashMap<PathBuf, Instant> = HashMap::new();
    let window = Duration::from_millis(WATCHER_DEBOUNCE_MS);

    while let Some(raw) = rx.recv().await {
        let last = last_seen.get(&raw.path).copied();
        let now = raw.at;
        let recent = last.is_some_and(|t| now.duration_since(t) < window);
        if recent {
            debug!(path = %raw.path.display(), "cert-watcher debounce drop");
            continue;
        }
        last_seen.insert(raw.path.clone(), now);
        let event = CertChangeEvent {
            path: raw.path,
            kind: raw.kind,
            at: SystemTime::now(),
        };
        if let Err(e) = out.send(event).await {
            warn!(error = %e, "cert-watcher debounce tx drop");
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Polling fallback.
// ─────────────────────────────────────────────────────────────────────

/// Pure-polling alternative to [`CertWatcher`] for environments where
/// no kernel file-change API is available.
///
/// Always available, regardless of OS or kernel features.  Less
/// efficient than [`CertWatcher`] (one `stat()` per file per
/// `interval`) but trivially portable.
#[derive(Debug)]
pub struct PollWatcher {
    handle: Arc<Mutex<PollWatcherInner>>,
}

#[derive(Debug)]
struct PollWatcherInner {
    paths: Vec<PathBuf>,
    interval: Duration,
}

impl PollWatcher {
    /// Start polling.  Returns the watcher (keep alive) and a
    /// receiver of debounced events.
    pub fn watch(
        paths: Vec<PathBuf>,
        interval: Duration,
    ) -> Result<(Self, mpsc::Receiver<CertChangeEvent>), TlsError> {
        if paths.is_empty() {
            return Err(TlsError::Watcher("no paths to poll".into()));
        }
        let (tx, rx) = mpsc::channel::<CertChangeEvent>(64);
        let paths_clone = paths.clone();
        let interval_clone = interval;
        tokio::spawn(async move {
            poll_loop(paths_clone, interval_clone, tx).await;
        });
        Ok((
            Self {
                handle: Arc::new(Mutex::new(PollWatcherInner { paths, interval })),
            },
            rx,
        ))
    }

    /// Return a snapshot of the (paths, interval) tuple.
    pub async fn snapshot(&self) -> (Vec<PathBuf>, Duration) {
        let g = self.handle.lock().await;
        (g.paths.clone(), g.interval)
    }
}

async fn poll_loop(paths: Vec<PathBuf>, interval: Duration, out: mpsc::Sender<CertChangeEvent>) {
    let mut last: HashMap<PathBuf, SystemTime> = HashMap::new();
    loop {
        for p in &paths {
            if let Ok(meta) = std::fs::metadata(p) {
                if let Ok(mtime) = meta.modified() {
                    match last.get(p) {
                        Some(prev) if *prev == mtime => continue,
                        _ => {
                            last.insert(p.clone(), mtime);
                            let _ = out
                                .send(CertChangeEvent {
                                    path: p.clone(),
                                    kind: "poll-mtime-change".into(),
                                    at: SystemTime::now(),
                                })
                                .await;
                        }
                    }
                }
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// Test helper : check that a path exists and is regular.
#[allow(dead_code)]
pub(crate) fn path_is_regular(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn poll_watcher_emits_on_mtime_change() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("tls.crt");
        std::fs::write(&path, b"initial").expect("write");

        let (watcher, mut rx) =
            PollWatcher::watch(vec![path.clone()], Duration::from_millis(100)).expect("watch");

        // First emission : the initial mtime read.
        let ev = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("event");
        assert_eq!(ev.path, path);

        // Modify the file ; expect a second emission.
        // Sleep first so that the second mtime is *strictly* later than
        // the first on filesystems with second-grained mtime (e.g. ext4
        // under certain mount options or HFS+ on macOS CI runners).
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open");
        f.write_all(b"rotated").expect("write");
        f.sync_all().expect("sync");
        drop(f);

        let ev = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timeout")
            .expect("event");
        assert_eq!(ev.path, path);

        let (paths, interval) = watcher.snapshot().await;
        assert_eq!(paths, vec![path]);
        assert_eq!(interval, Duration::from_millis(100));
    }

    #[tokio::test]
    async fn cert_watcher_rejects_empty_path_list() {
        let err = CertWatcher::watch(vec![]).unwrap_err();
        assert!(matches!(err, TlsError::Watcher(_)));
    }

    #[tokio::test]
    async fn poll_watcher_rejects_empty_path_list() {
        let err = PollWatcher::watch(vec![], Duration::from_secs(1)).unwrap_err();
        assert!(matches!(err, TlsError::Watcher(_)));
    }

    #[test]
    fn is_interesting_event_covers_modify_and_create() {
        use notify::event::{CreateKind, ModifyKind, RemoveKind};
        assert!(is_interesting_event(&EventKind::Modify(ModifyKind::Any)));
        assert!(is_interesting_event(&EventKind::Create(CreateKind::File)));
        assert!(is_interesting_event(&EventKind::Remove(RemoveKind::File)));
        assert!(is_interesting_event(&EventKind::Other));
        assert!(!is_interesting_event(&EventKind::Access(
            notify::event::AccessKind::Read
        )));
    }

    #[tokio::test]
    async fn debounce_collapses_burst() {
        let (in_tx, in_rx) = mpsc::channel::<RawEvent>(8);
        let (out_tx, mut out_rx) = mpsc::channel::<CertChangeEvent>(8);
        tokio::spawn(debounce_loop(in_rx, out_tx));

        let path = PathBuf::from("/tmp/burst.crt");
        let now = Instant::now();
        // 5 events within 50 ms ; expect 1 emission.
        for _ in 0..5 {
            in_tx
                .send(RawEvent {
                    path: path.clone(),
                    kind: "burst".into(),
                    at: now,
                })
                .await
                .expect("tx");
        }

        let ev = tokio::time::timeout(Duration::from_millis(300), out_rx.recv())
            .await
            .expect("timeout")
            .expect("event");
        assert_eq!(ev.path, path);

        // No second emission within 300 ms.
        let result = tokio::time::timeout(Duration::from_millis(300), out_rx.recv()).await;
        assert!(result.is_err(), "expected timeout, got {result:?}");
    }
}
