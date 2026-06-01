/*
 * Sprint D26 (Φ-B) — Sentinel : gRPC code 4 (DEADLINE_EXCEEDED).
 * Retry-eligible.
 */
package ai.efe.sdk.exception;

public final class EfeTimeoutException extends EfeException {
    private static final long serialVersionUID = 1L;

    public EfeTimeoutException() { this("deadline exceeded"); }
    public EfeTimeoutException(String message) {
        super("timeout", "efe: " + message);
    }
    public EfeTimeoutException(String message, Throwable cause) {
        super("timeout", "efe: " + message, cause);
    }
}
