/*
 * Sprint D26 (Φ-B) — No-auth singleton.
 */
package ai.efe.sdk.auth;

/**
 * Singleton no-auth config (DEV ONLY — production servers reject).
 *
 * <p>Use {@link #INSTANCE}.
 *
 * @since 0.4.0
 */
public final class NoAuth implements AuthConfig {

    /** Canonical singleton instance. */
    public static final NoAuth INSTANCE = new NoAuth();

    private NoAuth() {}

    @Override
    public String toString() {
        return "NoAuth";
    }
}
