/*
 * Sprint D26 (Φ-B) — Sentinel : gRPC code 5 (NOT_FOUND).
 * Catalog key doesn't resolve on the server.
 */
package ai.efe.sdk.exception;

public final class EfeCatalogNotFoundException extends EfeException {
    private static final long serialVersionUID = 1L;

    public EfeCatalogNotFoundException() { this("catalog not found"); }
    public EfeCatalogNotFoundException(String message) {
        super("catalog_not_found", "efe: " + message);
    }
    public EfeCatalogNotFoundException(String message, Throwable cause) {
        super("catalog_not_found", "efe: " + message, cause);
    }
}
