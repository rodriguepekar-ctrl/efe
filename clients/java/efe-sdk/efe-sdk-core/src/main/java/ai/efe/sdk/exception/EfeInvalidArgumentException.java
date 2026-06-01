/*
 * Sprint D26 (Φ-B) — Sentinel : gRPC code 3 (INVALID_ARGUMENT).
 * Request shape is bad. Not retryable.
 */
package ai.efe.sdk.exception;

public final class EfeInvalidArgumentException extends EfeException {
    private static final long serialVersionUID = 1L;

    public EfeInvalidArgumentException() { this("invalid argument"); }
    public EfeInvalidArgumentException(String message) {
        super("invalid_argument", "efe: " + message);
    }
    public EfeInvalidArgumentException(String message, Throwable cause) {
        super("invalid_argument", "efe: " + message, cause);
    }
}
