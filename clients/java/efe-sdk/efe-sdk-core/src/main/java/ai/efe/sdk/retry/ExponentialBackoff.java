/*
 * Sprint D26 (Φ-B) — Exponential backoff with full jitter (AWS canonical).
 *
 * Same algorithm as Go SDK ExponentialBackoff + TS SDK ExponentialBackoff.
 *
 * Pin tested by `test_d26_java_sdk_exponential_backoff`.
 */
package ai.efe.sdk.retry;

import java.time.Duration;
import java.util.Objects;
import java.util.random.RandomGenerator;

/**
 * Exponential backoff with full jitter.
 *
 * <pre>
 * delay = random(0, min(maxInterval, initialInterval * multiplier^(attempt-1)))
 * </pre>
 *
 * @since 0.4.0
 */
public final class ExponentialBackoff implements RetryBackoff {

    private final Duration initialInterval;
    private final Duration maxInterval;
    private final double multiplier;
    private final RandomGenerator random;

    public ExponentialBackoff(Duration initialInterval, Duration maxInterval, double multiplier) {
        this(initialInterval, maxInterval, multiplier, RandomGenerator.getDefault());
    }

    /** Test-only constructor — inject a deterministic random source. */
    ExponentialBackoff(Duration initialInterval, Duration maxInterval, double multiplier,
                        RandomGenerator random) {
        Objects.requireNonNull(initialInterval, "initialInterval");
        Objects.requireNonNull(maxInterval, "maxInterval");
        if (initialInterval.isNegative() || initialInterval.isZero()) {
            throw new IllegalArgumentException("initialInterval must be > 0");
        }
        if (maxInterval.isNegative() || maxInterval.isZero()) {
            throw new IllegalArgumentException("maxInterval must be > 0");
        }
        if (multiplier <= 1.0) {
            throw new IllegalArgumentException("multiplier must be > 1, got " + multiplier);
        }
        this.initialInterval = initialInterval;
        this.maxInterval = maxInterval;
        this.multiplier = multiplier;
        this.random = Objects.requireNonNull(random);
    }

    @Override
    public Duration next(int attempt) {
        if (attempt < 1) {
            throw new IllegalArgumentException("attempt must be >= 1, got " + attempt);
        }
        long baseMs = (long) Math.min(
            initialInterval.toMillis() * Math.pow(multiplier, attempt - 1.0),
            (double) maxInterval.toMillis());
        long delayMs = (long) (random.nextDouble() * baseMs);
        return Duration.ofMillis(delayMs);
    }

    public Duration initialInterval() { return initialInterval; }
    public Duration maxInterval() { return maxInterval; }
    public double multiplier() { return multiplier; }
}
