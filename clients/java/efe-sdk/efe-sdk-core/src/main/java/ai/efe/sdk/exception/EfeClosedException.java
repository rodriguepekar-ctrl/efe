/*
 * Sprint D26 (Φ-B) — Sentinel : local SDK error.
 * The client was already closed when the RPC was attempted.
 */
package ai.efe.sdk.exception;

public final class EfeClosedException extends EfeException {
    private static final long serialVersionUID = 1L;

    public EfeClosedException() { this("client is closed"); }
    public EfeClosedException(String message) {
        super("closed", "efe: " + message);
    }
    public EfeClosedException(String message, Throwable cause) {
        super("closed", "efe: " + message, cause);
    }
}
