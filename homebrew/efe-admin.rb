# frozen_string_literal: true

# Homebrew formula for the EFE admin CLI (`efe-admin`).
#
# This file is a *reference template* — the canonical formula is rendered
# at release time by `scripts/release/render-homebrew.sh` and committed
# to the `efe-ai/homebrew-tap` tap repository. The URLs and target naming
# below match what the `release-v1-ga.yml` `cli-binaries` job actually
# emits :
#   - GitHub Release at the master `vX.Y.Z` tag (NOT `efe-admin/vX.Y.Z`).
#   - Filename pattern : `efe-admin-vX.Y.Z-<rust-target-triple>.tar.gz`
#     (see scripts/release/render-homebrew.sh for the exact pattern).
#   - Rust target triples : `x86_64-apple-darwin`, `aarch64-apple-darwin`,
#     `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`.
#
# Standard install (after tap is published) :
#   brew tap efe-ai/tap
#   brew install efe-admin
#
# Manual install (without tap, for testing) :
#   brew install --formula https://raw.githubusercontent.com/efe-ai/homebrew-tap/main/Formula/efe-admin.rb
#
# Verification (post-install) :
#   efe-admin --version       # → efe-admin 1.0.0 (sha: <git-sha>)
#   cosign verify-blob \
#     --certificate "$(brew --prefix efe-admin)/share/cosign.cert.pem" \
#     --signature   "$(brew --prefix efe-admin)/share/cosign.sig" \
#     --certificate-identity-regexp "^https://github\.com/efe-ai/efe/\.github/workflows/release-v1-ga\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+$" \
#     --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
#     "$(brew --prefix efe-admin)/bin/efe-admin"

class EfeAdmin < Formula
  desc "Admin CLI for the EFE (Ephemeral Fluid Swarm) classification platform"
  homepage "https://efe.example.com"
  version "1.0.0"
  license "Apache-2.0"

  # The release pipeline produces 4 macOS+Linux tarballs (CLI matrix :
  # darwin/linux × x86_64/aarch64). Filenames use Rust target triples,
  # matching the `cli-binaries` matrix in `.github/workflows/release-v1-ga.yml`.
  # SHA-256 digests are substituted by `render-homebrew.sh` at release time —
  # the placeholders below are intentional and MUST be replaced before tap publish.

  on_macos do
    on_intel do
      url "https://github.com/efe-ai/efe/releases/download/v1.0.0/efe-admin-v1.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_arm do
      url "https://github.com/efe-ai/efe/releases/download/v1.0.0/efe-admin-v1.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/efe-ai/efe/releases/download/v1.0.0/efe-admin-v1.0.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_arm do
      url "https://github.com/efe-ai/efe/releases/download/v1.0.0/efe-admin-v1.0.0-aarch64-unknown-linux-musl.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  # Soft-recommended (but not strict) — needed for `efe-admin verify` to do
  # offline Sigstore verification and SLSA provenance assertion.
  depends_on "cosign" => :recommended
  depends_on "slsa-verifier" => :optional

  def install
    bin.install "efe-admin"
    # Cosign signature + cert ship inside the tarball for offline verification.
    if File.exist?("cosign.sig")
      share.install "cosign.sig"
      share.install "cosign.cert.pem"
    end
    # Bash / zsh / fish completions, generated at build time.
    generate_completions_from_executable(bin/"efe-admin", "completions",
                                         shells: [:bash, :zsh, :fish])
    # Man page.
    man1.install "efe-admin.1" if File.exist?("efe-admin.1")
  end

  def caveats
    <<~EOS
      EFE Admin CLI v#{version} installed.

      First-time setup :
        efe-admin login --tenant <slug> --endpoint https://api.efe.example.com

      Verify the installed binary's Sigstore signature :
        cosign verify-blob \\
          --certificate "#{share}/cosign.cert.pem" \\
          --signature   "#{share}/cosign.sig" \\
          --certificate-identity-regexp "^https://github\\.com/efe-ai/efe/\\.github/workflows/release-v1-ga\\.yml@refs/tags/v[0-9]+\\.[0-9]+\\.[0-9]+$" \\
          --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \\
          "#{bin}/efe-admin"

      Documentation : https://docs.efe.example.com/cli
      Security feed : https://efe.example.com/security/feed.xml
    EOS
  end

  test do
    # Smoke test : --version prints semver + git sha.
    assert_match(/efe-admin \d+\.\d+\.\d+/, shell_output("#{bin}/efe-admin --version"))
    # Help summary lists all top-level commands.
    help = shell_output("#{bin}/efe-admin --help")
    assert_match "audit", help
    assert_match "compliance", help
    assert_match "verify", help
  end
end
