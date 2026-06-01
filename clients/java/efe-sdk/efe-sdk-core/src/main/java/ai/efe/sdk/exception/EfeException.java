/*
 * Sprint D26 (Φ-B) — Base exception + isRetryable + errorCode helpers.
 *
 * Cross-language pinned : 10 sentinels match Python M8 + Go M9 + TS M10.
 */
package ai.efe.sdk.exception;

/**
 * Base unchecked exception for all SDK-generated errors.
 *
 * <p>Use {@code instanceof EfeException} to detect any SDK error
 * generically ; use {@code instanceof EfeXxxException} to discriminate
 * the specific sentinel.
 *
 * <p>The {@link #code()} field is a stable string short-label (used
 * for metrics labels / log fields / IDE autocomplete). The class
 * name follows the Java convention {@code EfeXxxException}.
 *
 * @since 0.4.0
 */
public class EfeException extends RuntimeException {

    private static final long serialVersionUID = 1L;

    private final String code;

    public EfeException(String code, String message) {
        super(message);
        this.code = code;
    }

    public EfeException(String code, String message, Throwable cause) {
        super(message, cause);
        this.code = code;
    }

    /** Stable short code (used for metrics labels). */
    public String code() {
        return code;
    }

    /**
     * Returns true iff {@code err} belongs to the retry-eligible subset.
     *
     * <p>Retry-eligible :
     * <ul>
     *   <li>{@link EfeUnavailableException}</li>
     *   <li>{@link EfeTimeoutException}</li>
     *   <li>{@link EfeRateLimitedException}</li>
     * </ul>
     *
     * <p>Non-retryable : everything else (auth failure, config error,
     * request shape error, internal server bug, local close).
     */
    public static boolean isRetryable(Throwable err) {
        return err instanceof EfeUnavailableException
            || err instanceof EfeTimeoutException
            || err instanceof EfeRateLimitedException;
    }

    /**
     * Returns the canonical short code used in logs / metrics labels.
     *
     * <p>Stable mapping :
     * <ul>
     *   <li>EfeUnauthenticatedException  → "unauthenticated"</li>
     *   <li>EfePermissionDeniedException → "permission_denied"</li>
     *   <li>EfeCatalogNotFoundException  → "catalog_not_found"</li>
     *   <li>EfeCatalogInvalidException   → "catalog_invalid"</li>
     *   <li>EfeTimeoutException          → "timeout"</li>
     *   <li>EfeUnavailableException      → "unavailable"</li>
     *   <li>EfeRateLimitedException      → "rate_limited"</li>
     *   <li>EfeInvalidArgumentException  → "invalid_argument"</li>
     *   <li>EfeInternalException         → "internal"</li>
     *   <li>EfeClosedException           → "closed"</li>
     *   <li>other                        → "unknown"</li>
     *   <li>null                         → ""</li>
     * </ul>
     */
    public static String errorCode(Throwable err) {
        if (err == null) return "";
        if (err instanceof EfeException ee) return ee.code;
        return "unknown";
    }
}
