# Contributing to EFE

## Toolchain

EFE pins the Rust toolchain via `rust-toolchain.toml` at the repo root.
Currently : **rustc 1.88** (bumped from 1.85 in D93 — see
`docs/docs/adr/ADR-D93-001-rustc-1.88-bump.md`).

```bash
# First-time setup (one-shot install + components)
rustup install 1.88
rustup component add --toolchain 1.88 rustfmt clippy rust-src rust-analyzer

# Or just `cd` into the repo — the rust-toolchain.toml triggers an
# auto-install on the next cargo invocation.
cd path/to/efe
cargo --version       # rustc 1.88.0 (...)
```

If you have a stuck toolchain :
```bash
rustup self update
rustup update 1.88
```

The Go toolchain is pinned at `1.22` for `cmd/vsock-relayer`.

## Workflow

EFE follows trunk-based development with short-lived feature branches.

1. Pick an issue (or create one with clear acceptance criteria).
2. Create a feature branch: `feat/m<N>-<short-slug>`.
3. Open a draft PR as soon as you push — visibility > surprise.
4. Keep PRs under 400 LOC when possible.
5. Mark PR ready-for-review only when CI is green and you've self-reviewed.
6. Need 2 approvals (including CODEOWNER) before merge.
7. Squash-merge to main with a clean conventional commit.

## Branch naming

| Prefix | Usage |
|--------|-------|
| `feat/` | New feature |
| `fix/` | Bug fix |
| `refactor/` | Behavior-preserving refactor |
| `perf/` | Performance optimization |
| `chore/` | Maintenance (deps, CI) |
| `docs/` | Documentation |
| `test/` | Tests only |
| `hotfix/` | Urgent release fix |

## Commit convention

We enforce [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <short subject>

<optional body explaining why>

<optional footer: refs #123, BREAKING CHANGE, Co-authored-by>
```

Types: `feat`, `fix`, `refactor`, `perf`, `chore`, `docs`, `test`, `build`, `ci`.

Scopes: `m1`..`m12`, `api`, `kernels`, `training`, `infra`, etc.

Example:
```
feat(m5): add hierarchical fusion with 2-level clustering

Reduces attention complexity from O(N^2) to O(N sqrt N) for N > 4096.
Bench shows 3.2x speedup at N=10000 on H100.

Refs: ARCH-214
```

## Definition of Done

A PR is done when:

- [ ] Code does what the issue describes.
- [ ] Unit tests cover ≥85% of new branches.
- [ ] Integration tests exist for user-facing features.
- [ ] `rustdoc` / docstrings updated.
- [ ] `CHANGELOG.md` entry added.
- [ ] Benchmark (if hot path) shows no regression > 5%.
- [ ] 2 approvals, at least 1 CODEOWNER.
- [ ] Config/schema changes have migration script.
- [ ] CI green across all matrices.

## Code review etiquette

- **Responsiveness:** first reply within 4 business hours.
- **Priority tags in comments:**
  - `P1` — blocking, must be addressed
  - `P2` — suggestion, nice-to-have
  - `P3` — nit, taste
  - `Q`  — question, not a request
- **Constructive only:** explain the why, propose alternatives.
- **No bikeshedding** on style the formatter could catch.

## Module ownership

See `.github/CODEOWNERS` for full ownership matrix. Every module has a
primary owner who is an automatic reviewer.

## Questions

- Technical: Slack `#eng-help`
- Process: Slack `#team-efe`
- Security: `security@efe.ai` (private)
