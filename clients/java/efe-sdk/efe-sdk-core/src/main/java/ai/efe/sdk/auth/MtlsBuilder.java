/*
 * Sprint D26 (Φ-B) — mTLS SslContext builder helper.
 *
 * Encapsulates the call into `GrpcSslContexts.forClient()` — the
 * canonical way to construct a Netty TLS context for gRPC client mTLS.
 */
package ai.efe.sdk.auth;

import io.grpc.netty.shaded.io.grpc.netty.GrpcSslContexts;
import io.grpc.netty.shaded.io.netty.handler.ssl.SslContext;

import ai.efe.sdk.exception.EfeInvalidArgumentException;

import java.io.IOException;

/**
 * Builds a Netty {@code SslContext} for client-side mTLS.
 *
 * <p>Enforces TLS 1.3 minimum (matches Lettre E D22 server-side
 * enforcement). All file reads happen at construction time — bad paths
 * surface immediately as {@link EfeInvalidArgumentException}.
 *
 * @since 0.4.0
 */
public final class MtlsBuilder {

    private MtlsBuilder() {}

    /**
     * Build the SslContext from the {@link MtlsAuth} cert/key/CA paths.
     *
     * @throws EfeInvalidArgumentException on any I/O / SSL error
     *         (file missing, malformed PEM, key/cert mismatch).
     */
    public static SslContext build(MtlsAuth auth) {
        try {
            return GrpcSslContexts.forClient()
                .keyManager(auth.certPath().toFile(), auth.keyPath().toFile())
                .trustManager(auth.caPath().toFile())
                .protocols("TLSv1.3")
                .build();
        } catch (IOException | RuntimeException e) {
            throw new EfeInvalidArgumentException(
                "mTLS: cannot build SslContext (" + e.getMessage() + ")", e);
        }
    }
}
