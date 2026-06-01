/*
 * Sprint D26 (Φ-B) — Sentinel : gRPC code 13 (INTERNAL) / 2 (UNKNOWN).
 * Server crashed or in unknown state. Not retryable by default.
 */
package ai.efe.sdk.exception;

public final class EfeInternalException extends EfeException {
    private static final long serialVersionUID = 1L;

    public EfeInternalException() { this("server internal error"); }
    public EfeInternalException(String message) {
        super("internal", "efe: " + message);
    }
    public EfeInternalException(String message, Throwable cause) {
        super("internal", "efe: " + message, cause);
    }
}
