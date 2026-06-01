/*
 * Sprint D26 (Φ-B) — Sentinel : gRPC code 9 (FAILED_PRECONDITION).
 * Catalog corrupted, version-mismatched, or otherwise unusable.
 */
package ai.efe.sdk.exception;

public final class EfeCatalogInvalidException extends EfeException {
    private static final long serialVersionUID = 1L;

    public EfeCatalogInvalidException() { this("catalog invalid"); }
    public EfeCatalogInvalidException(String message) {
        super("catalog_invalid", "efe: " + message);
    }
    public EfeCatalogInvalidException(String message, Throwable cause) {
        super("catalog_invalid", "efe: " + message, cause);
    }
}
