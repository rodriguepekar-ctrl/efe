/*
 * Sprint D26 (Φ-B) — Sentinel : gRPC code 7 (PERMISSION_DENIED).
 * Token valid but lacks the required scope.
 */
package ai.efe.sdk.exception;

public final class EfePermissionDeniedException extends EfeException {
    private static final long serialVersionUID = 1L;

    public EfePermissionDeniedException() { this("permission denied"); }
    public EfePermissionDeniedException(String message) {
        super("permission_denied", "efe: " + message);
    }
    public EfePermissionDeniedException(String message, Throwable cause) {
        super("permission_denied", "efe: " + message, cause);
    }
}
