/*
 * Sprint D26 (Φ-B) — mTLS auth config record.
 */
package ai.efe.sdk.auth;

import java.nio.file.Path;
import java.util.Objects;

/**
 * Mutual TLS auth — paths to PEM-encoded cert / key / CA files.
 *
 * <p>Paths are read at {@code EfeClient.builder().build()} time via
 * {@link MtlsBuilder#build(MtlsAuth)} which produces a Netty
 * {@code SslContext} with TLS 1.3 minimum (matches Lettre E D22
 * server-side enforcement).
 *
 * @param certPath PEM-encoded client certificate.
 * @param keyPath  PEM-encoded private key.
 * @param caPath   PEM-encoded CA bundle.
 *
 * @since 0.4.0
 */
public record MtlsAuth(Path certPath, Path keyPath, Path caPath) implements AuthConfig {

    public MtlsAuth {
        Objects.requireNonNull(certPath, "certPath");
        Objects.requireNonNull(keyPath, "keyPath");
        Objects.requireNonNull(caPath, "caPath");
    }

    /** Convenience factory. */
    public static MtlsAuth fromFiles(Path cert, Path key, Path ca) {
        return new MtlsAuth(cert, key, ca);
    }
}
