/*
 * Sprint D26 (Φ-B) — Sentinel : gRPC code 16 (UNAUTHENTICATED).
 * Token missing, malformed, or rejected by the server.
 */
package ai.efe.sdk.exception;

public final class EfeUnauthenticatedException extends EfeException {
    private static final long serialVersionUID = 1L;

    public EfeUnauthenticatedException() { this("unauthenticated"); }
    public EfeUnauthenticatedException(String message) {
        super("unauthenticated", "efe: " + message);
    }
    public EfeUnauthenticatedException(String message, Throwable cause) {
        super("unauthenticated", "efe: " + message, cause);
    }
}
