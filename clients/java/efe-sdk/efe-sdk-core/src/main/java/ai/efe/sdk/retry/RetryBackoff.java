/*
 * Sprint D26 (Φ-B) — RetryBackoff interface (universal — used by
 * RetryClientInterceptor + classifyStream worker pool).
 */
package ai.efe.sdk.retry;

import java.time.Duration;

/**
 * Computes the delay before the (attempt+1)-th try.
 *
 * <p>Implementations MUST be deterministic given the same attempt
 * number — except for jitter, which intentionally introduces randomness
 * to avoid thundering herd.
 *
 * @since 0.4.0
 */
@FunctionalInterface
public interface RetryBackoff {

    /**
     * Returns the delay before retry attempt {@code attempt} (1-indexed,
     * so attempt=1 = first retry, AFTER the initial try).
     *
     * @param attempt 1-indexed attempt number, &ge; 1.
     * @return delay duration (&ge; Duration.ZERO).
     */
    Duration next(int attempt);
}
