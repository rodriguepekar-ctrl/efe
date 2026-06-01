/*
 * Sprint D26 (Φ-B) — JWT CallCredentials impl.
 *
 * gRPC `CallCredentials` injects metadata per RPC. We use it (vs a
 * static ClientInterceptor) because it integrates cleanly with the
 * gRPC stub `withCallCredentials(...)` helper + supports dynamic
 * token refresh in future iterations.
 */
package ai.efe.sdk.auth;

import io.grpc.CallCredentials;
import io.grpc.Metadata;
import io.grpc.Status;

import java.util.Objects;
import java.util.concurrent.Executor;

/**
 * Injects "Authorization: Bearer &lt;jwt&gt;" metadata on every RPC.
 *
 * <p>Captured at construction ; never logged ; redacted by
 * {@link #toString()}.
 *
 * @since 0.4.0
 */
public final class JwtCallCredentials extends CallCredentials {

    private static final Metadata.Key<String> AUTHORIZATION =
        Metadata.Key.of("authorization", Metadata.ASCII_STRING_MARSHALLER);

    private final String bearerHeader;

    public JwtCallCredentials(String token) {
        Objects.requireNonNull(token, "token");
        if (token.isBlank()) {
            throw new IllegalArgumentException("JWT token must be non-blank");
        }
        this.bearerHeader = "Bearer " + token;
    }

    @Override
    public void applyRequestMetadata(RequestInfo requestInfo, Executor appExecutor,
                                       MetadataApplier applier) {
        appExecutor.execute(() -> {
            try {
                Metadata headers = new Metadata();
                headers.put(AUTHORIZATION, bearerHeader);
                applier.apply(headers);
            } catch (Throwable t) {
                applier.fail(Status.UNAUTHENTICATED.withCause(t)
                    .withDescription("efe: failed to apply auth metadata"));
            }
        });
    }

    @Override
    public String toString() {
        return "JwtCallCredentials[<redacted>]";
    }
}
