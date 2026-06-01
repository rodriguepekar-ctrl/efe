/*
 * Sprint D26 (Φ-B) — RetryConfig record.
 */
package ai.efe.sdk.retry;

import java.time.Duration;
import java.util.Objects;

/**
 * Retry policy.
 *
 * @param maxAttempts 1-indexed maximum total attempts. 1 = no retry
 *                    (single try). 3 = recommended.
 * @param backoff Backoff impl.
 *
 * @since 0.4.0
 */
public record RetryConfig(int maxAttempts, RetryBackoff backoff) {

    public RetryConfig {
        if (maxAttempts < 1) {
            throw new IllegalArgumentException("maxAttempts must be >= 1, got " + maxAttempts);
        }
        Objects.requireNonNull(backoff, "backoff");
    }

    /**
     * Convenience factory : exponential backoff with full jitter.
     *
     * <p>Defaults : multiplier = 2.0.
     */
    public static RetryConfig exponential(int maxAttempts, Duration initial, Duration max) {
        return new RetryConfig(maxAttempts, new ExponentialBackoff(initial, max, 2.0));
    }

    /** No retry — single attempt. */
    public static RetryConfig disabled() {
        return new RetryConfig(1, attempt -> Duration.ZERO);
    }
}
