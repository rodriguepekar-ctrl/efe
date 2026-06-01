/*
 * Sprint D26 (Φ-B) — JWT auth config record.
 */
package ai.efe.sdk.auth;

import java.util.Objects;

/**
 * Bearer JWT auth — the token is sent as `Authorization: Bearer <token>`
 * in the gRPC metadata of every request.
 *
 * <p>The SDK does NOT validate the token client-side — that's the
 * server's job. The token is held in this record + injected via
 * {@link JwtCallCredentials} at call time. Never logged ;
 * {@link #toString()} returns a redacted placeholder.
 *
 * @param token the JWT bearer token (non-null, non-blank).
 *
 * @since 0.4.0
 */
public record JwtAuth(String token) implements AuthConfig {

    public JwtAuth {
        Objects.requireNonNull(token, "token");
        if (token.isBlank()) {
            throw new IllegalArgumentException("JWT token must be non-blank");
        }
    }

    /** Convenience factory : `JwtAuth.of("eyJhbGc...")`. */
    public static JwtAuth of(String token) {
        return new JwtAuth(token);
    }

    /** Redacted — never includes the actual token. */
    @Override
    public String toString() {
        return "JwtAuth[<redacted>]";
    }
}
