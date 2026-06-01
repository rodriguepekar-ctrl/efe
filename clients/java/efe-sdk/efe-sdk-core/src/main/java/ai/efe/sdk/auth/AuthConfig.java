/*
 * Sprint D26 (Φ-B) — Sealed interface for auth strategies.
 *
 * Java 17 sealed types : exhaustive pattern matching at compile time
 * via `switch (auth) { case JwtAuth jwt -> ...; case MtlsAuth mtls -> ...;
 * case NoAuth n -> ...; }`. No default branch needed.
 *
 * Pin tested by `test_d26_java_sdk_auth_config_sealed`.
 */
package ai.efe.sdk.auth;

/**
 * Sealed interface : exactly 3 implementations.
 *
 *   - {@link JwtAuth} : Bearer token in `authorization` header (universal).
 *   - {@link MtlsAuth} : client certificate at TLS handshake.
 *   - {@link NoAuth}  : no auth (DEV ONLY — server typically rejects).
 *
 * @since 0.4.0
 */
public sealed interface AuthConfig permits JwtAuth, MtlsAuth, NoAuth {}
