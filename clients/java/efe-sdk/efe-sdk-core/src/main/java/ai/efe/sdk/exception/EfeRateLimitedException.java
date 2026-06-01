/*
 * Sprint D26 (Φ-B) — Sentinel : gRPC code 8 (RESOURCE_EXHAUSTED).
 * Tenant quota exceeded. Retry-eligible after backoff.
 */
package ai.efe.sdk.exception;

public final class EfeRateLimitedException extends EfeException {
    private static final long serialVersionUID = 1L;

    public EfeRateLimitedException() { this("rate limited"); }
    public EfeRateLimitedException(String message) {
        super("rate_limited", "efe: " + message);
    }
    public EfeRateLimitedException(String message, Throwable cause) {
        super("rate_limited", "efe: " + message, cause);
    }
}
