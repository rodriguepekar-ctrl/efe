//! Sprint D17 — mockable shell-out abstraction.
//!
//! Every external CLI tool (pgBackRest, vault, rclone, helm,
//! kubectl) is invoked via the [`CommandRunner`] trait. Production
//! code uses [`TokioCommandRunner`] which spawns real processes.
//! Tests use [`MockCommandRunner`] which returns canned responses
//! keyed by `(program, args)`.
//!
//! This indirection is what lets the integration tests run
//! offline, without pgBackRest / Vault / S3 actually installed.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

/// Captured output of a child-process invocation.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// Process exit code. `0` is success on POSIX.
    pub status: i32,
    /// Standard output bytes.
    pub stdout: Vec<u8>,
    /// Standard error bytes.
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    /// `true` iff the process exited with status `0`.
    #[must_use]
    pub fn success(&self) -> bool {
        self.status == 0
    }

    /// Convenience access to stdout as a UTF-8 lossless string.
    #[must_use]
    pub fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    /// Convenience access to stderr as a UTF-8 lossless string.
    #[must_use]
    pub fn stderr_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }
}

/// Pluggable executor. `Arc<dyn CommandRunner>` is the canonical
/// injection shape across the crate (Snapshotter impls hold one).
///
/// `Debug` is a super-trait so structs holding an
/// `Arc<dyn CommandRunner>` can derive `Debug`.
#[async_trait]
pub trait CommandRunner: Send + Sync + std::fmt::Debug {
    /// Spawn `program` with `args`, augmenting the env with `env`.
    /// Awaits completion and returns the captured output.
    ///
    /// `env` entries shadow any inherited env var of the same key.
    async fn run(
        &self,
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> std::io::Result<CommandOutput>;
}

/// Production runner that uses `tokio::process::Command`.
///
/// **Audit fix § D17-AH9** — every spawn is wrapped in a
/// `tokio::time::timeout` of `process_timeout` (default 30 min,
/// covers a 100 GB pgBackRest base backup). On timeout the child
/// is `kill`-ed best-effort and the runner returns
/// `io::ErrorKind::TimedOut`. Pre-audit, a hung pgBackRest could
/// block the CronJob pod indefinitely (the K8s
/// `activeDeadlineSeconds` was the only fence — a brittle one).
#[derive(Debug, Clone)]
pub struct TokioCommandRunner {
    /// Maximum wall-clock duration for any single child process.
    pub process_timeout: std::time::Duration,
}

impl Default for TokioCommandRunner {
    fn default() -> Self {
        Self {
            process_timeout: std::time::Duration::from_secs(30 * 60),
        }
    }
}

impl TokioCommandRunner {
    /// Construct with a custom timeout.
    pub fn with_timeout(process_timeout: std::time::Duration) -> Self {
        Self { process_timeout }
    }
}

#[async_trait]
impl CommandRunner for TokioCommandRunner {
    async fn run(
        &self,
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> std::io::Result<CommandOutput> {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        // Spawn so we have a Child handle that we can kill on timeout.
        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let wait_for_output = async {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let (sout, serr, status) = tokio::join!(
                async {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    if let Some(mut r) = stdout {
                        let _ = r.read_to_end(&mut buf).await;
                    }
                    buf
                },
                async {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    if let Some(mut r) = stderr {
                        let _ = r.read_to_end(&mut buf).await;
                    }
                    buf
                },
                child.wait()
            );
            (sout, serr, status)
        };
        match tokio::time::timeout(self.process_timeout, wait_for_output).await {
            Ok((stdout, stderr, status)) => {
                let st = status?;
                Ok(CommandOutput {
                    status: st.code().unwrap_or(-1),
                    stdout,
                    stderr,
                })
            }
            Err(_elapsed) => {
                // Best-effort kill on timeout. Tokio's Child::kill
                // signals SIGKILL on Unix which is appropriate for
                // a hung child.
                let _ = child.start_kill();
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "subprocess {program:?} exceeded {} s timeout",
                        self.process_timeout.as_secs()
                    ),
                ))
            }
        }
    }
}

/// Test-only runner. Match key is `format!("{program} {args.join(\" \")}")`,
/// or any prefix thereof — first matching key wins. Records the
/// log of every invocation so tests can assert what was called.
#[derive(Debug, Default)]
pub struct MockCommandRunner {
    inner: Mutex<MockInner>,
}

#[derive(Debug, Default)]
struct MockInner {
    /// Map of "program args..." prefix → canned output.
    canned: HashMap<String, CommandOutput>,
    /// Log of every (program, args) invocation, in call order.
    log: Vec<MockInvocation>,
}

/// One recorded invocation against a [`MockCommandRunner`].
#[derive(Debug, Clone)]
pub struct MockInvocation {
    /// Program path passed to `run`.
    pub program: String,
    /// All argv entries.
    pub args: Vec<String>,
    /// Env-var pairs visible to this invocation.
    pub env: Vec<(String, String)>,
}

impl MockCommandRunner {
    /// Construct an empty mock. Pre-populate via [`Self::expect`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a canned response. The lookup matches by command
    /// prefix : `"pgbackrest"` matches every pgbackrest call,
    /// `"pgbackrest backup"` matches more specifically.
    pub fn expect(self, prefix: impl Into<String>, output: CommandOutput) -> Self {
        // unwrap_or_else here is in test-helper code path, but the
        // crate-wide deny(unwrap) lint applies.
        if let Ok(mut g) = self.inner.lock() {
            g.canned.insert(prefix.into(), output);
        }
        self
    }

    /// Convenience : expect a successful invocation returning
    /// `stdout` text.
    #[must_use]
    pub fn expect_ok(self, prefix: &str, stdout: &str) -> Self {
        self.expect(
            prefix,
            CommandOutput {
                status: 0,
                stdout: stdout.as_bytes().to_vec(),
                stderr: vec![],
            },
        )
    }

    /// Convenience : expect a failing invocation returning
    /// `stderr` text and the given exit code.
    #[must_use]
    pub fn expect_err(self, prefix: &str, status: i32, stderr: &str) -> Self {
        self.expect(
            prefix,
            CommandOutput {
                status,
                stdout: vec![],
                stderr: stderr.as_bytes().to_vec(),
            },
        )
    }

    /// Snapshot of the invocation log so far.
    pub fn invocations(&self) -> Vec<MockInvocation> {
        match self.inner.lock() {
            Ok(g) => g.log.clone(),
            Err(_) => vec![],
        }
    }

    /// Number of invocations recorded so far.
    pub fn invocation_count(&self) -> usize {
        match self.inner.lock() {
            Ok(g) => g.log.len(),
            Err(_) => 0,
        }
    }
}

#[async_trait]
impl CommandRunner for MockCommandRunner {
    async fn run(
        &self,
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> std::io::Result<CommandOutput> {
        let key = format!("{} {}", program, args.join(" "));
        let invocation = MockInvocation {
            program: program.to_string(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        };
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                return Err(std::io::Error::other("mock runner mutex poisoned"));
            }
        };
        guard.log.push(invocation);
        // Find the longest matching prefix among canned responses.
        let mut best: Option<(usize, CommandOutput)> = None;
        for (k, v) in &guard.canned {
            if key.starts_with(k) {
                let len = k.len();
                if best.as_ref().is_none_or(|(blen, _)| len > *blen) {
                    best = Some((len, v.clone()));
                }
            }
        }
        match best {
            Some((_, output)) => Ok(output),
            None => Err(std::io::Error::other(format!(
                "MockCommandRunner: no canned response for {key:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_runner_returns_canned_output() {
        let runner = MockCommandRunner::new()
            .expect_ok("pgbackrest backup", "{\"label\":\"x\"}");
        let out = runner
            .run("pgbackrest", &["backup", "--type=full"], &[])
            .await
            .unwrap();
        assert!(out.success());
        assert_eq!(out.stdout_string(), "{\"label\":\"x\"}");
    }

    #[tokio::test]
    async fn mock_runner_picks_longest_prefix() {
        let runner = MockCommandRunner::new()
            .expect_ok("pgbackrest", "generic")
            .expect_ok("pgbackrest backup", "specific");
        let out = runner
            .run("pgbackrest", &["backup", "--type=full"], &[])
            .await
            .unwrap();
        assert_eq!(out.stdout_string(), "specific");
    }

    #[tokio::test]
    async fn mock_runner_records_invocations() {
        let runner = MockCommandRunner::new()
            .expect_ok("vault", "ok")
            .expect_ok("rclone", "ok");
        let _ = runner.run("vault", &["operator", "raft"], &[]).await;
        let _ = runner.run("rclone", &["sync", "src", "dest"], &[]).await;
        let log = runner.invocations();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].program, "vault");
        assert_eq!(log[1].args, vec!["sync", "src", "dest"]);
    }

    #[tokio::test]
    async fn mock_runner_errors_on_no_match() {
        let runner = MockCommandRunner::new();
        let err = runner.run("unknown", &[], &[]).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
    }

    #[tokio::test]
    async fn mock_runner_propagates_failure_status() {
        let runner = MockCommandRunner::new().expect_err("rclone", 23, "boom");
        let out = runner.run("rclone", &[], &[]).await.unwrap();
        assert!(!out.success());
        assert_eq!(out.status, 23);
        assert_eq!(out.stderr_string(), "boom");
    }

    #[tokio::test]
    async fn mock_runner_invocation_count() {
        let runner = MockCommandRunner::new().expect_ok("foo", "");
        runner.run("foo", &[], &[]).await.unwrap();
        runner.run("foo", &["a"], &[]).await.unwrap();
        runner.run("foo", &["b"], &[]).await.unwrap();
        assert_eq!(runner.invocation_count(), 3);
    }

    #[tokio::test]
    async fn tokio_runner_runs_real_echo() {
        // Use `printf` because some sandbox shells lack `echo -n`.
        // This is the only test exercising the real spawn path.
        let runner = TokioCommandRunner::default();
        let out = runner.run("printf", &["%s", "hello"], &[]).await.unwrap();
        assert!(out.success());
        assert_eq!(out.stdout_string(), "hello");
    }
}
