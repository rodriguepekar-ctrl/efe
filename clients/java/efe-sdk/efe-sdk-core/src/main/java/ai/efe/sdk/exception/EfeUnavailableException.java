/*
 * Sprint D26 (Φ-B) — Sentinel : gRPC code 14 (UNAVAILABLE).
 * Server reachable but transient bad state. Retry-eligible.
 */
package ai.efe.sdk.exception;

public final class EfeUnavailableException extends EfeException {
    private static final long serialVersionUID = 1L;

    public EfeUnavailableException() { this("server unavailable"); }
    public EfeUnavailableException(String message) {
        super("unavailable", "efe: " + message);
    }
    public EfeUnavailableException(String message, Throwable cause) {
        super("unavailable", "efe: " + message, cause);
    }
}
