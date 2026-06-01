package ai.efe.sdk.retry;

import org.junit.jupiter.api.Test;

import java.time.Duration;
import java.util.random.RandomGenerator;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/** Sprint D26 — unit tests : ExponentialBackoff. */
class ExponentialBackoffTest {

    /** Deterministic random : returns 1.0 always (max jitter). */
    private static final RandomGenerator MAX_RNG = new RandomGenerator() {
        @Override public long nextLong() { return 0; }
        @Override public double nextDouble() { return 1.0; }
    };

    @Test
    void firstAttemptDelayEqualsInitialIntervalAtMaxJitter() {
        ExponentialBackoff b = new ExponentialBackoff(
            Duration.ofMillis(100), Duration.ofSeconds(30), 2.0, MAX_RNG);
        assertThat(b.next(1)).isEqualTo(Duration.ofMillis(100));
    }

    @Test
    void multiplicativeGrowthAtMaxJitter() {
        ExponentialBackoff b = new ExponentialBackoff(
            Duration.ofMillis(100), Duration.ofSeconds(10), 2.0, MAX_RNG);
        assertThat(b.next(1)).isEqualTo(Duration.ofMillis(100));
        assertThat(b.next(2)).isEqualTo(Duration.ofMillis(200));
        assertThat(b.next(3)).isEqualTo(Duration.ofMillis(400));
        assertThat(b.next(4)).isEqualTo(Duration.ofMillis(800));
    }

    @Test
    void capsAtMaxInterval() {
        ExponentialBackoff b = new ExponentialBackoff(
            Duration.ofSeconds(1), Duration.ofSeconds(2), 10.0, MAX_RNG);
        for (int i = 1; i <= 10; i++) {
            assertThat(b.next(i)).isLessThanOrEqualTo(Duration.ofSeconds(2));
        }
    }

    @Test
    void neverReturnsNegative() {
        ExponentialBackoff b = new ExponentialBackoff(
            Duration.ofMillis(100), Duration.ofSeconds(30), 2.0);
        for (int attempt = 1; attempt <= 100; attempt++) {
            assertThat(b.next(attempt)).isNotNegative();
        }
    }

    @Test
    void rejectsAttemptZero() {
        ExponentialBackoff b = new ExponentialBackoff(
            Duration.ofMillis(100), Duration.ofSeconds(30), 2.0);
        assertThatThrownBy(() -> b.next(0)).isInstanceOf(IllegalArgumentException.class);
    }

    @Test
    void rejectsBadConstructorArgs() {
        assertThatThrownBy(() -> new ExponentialBackoff(
                Duration.ZERO, Duration.ofSeconds(1), 2.0))
            .isInstanceOf(IllegalArgumentException.class);
        assertThatThrownBy(() -> new ExponentialBackoff(
                Duration.ofMillis(1), Duration.ZERO, 2.0))
            .isInstanceOf(IllegalArgumentException.class);
        assertThatThrownBy(() -> new ExponentialBackoff(
                Duration.ofMillis(1), Duration.ofSeconds(1), 1.0))
            .isInstanceOf(IllegalArgumentException.class);
    }

    @Test
    void retryConfigExponentialFactorySensibleDefaults() {
        RetryConfig cfg = RetryConfig.exponential(3, Duration.ofMillis(100), Duration.ofSeconds(30));
        assertThat(cfg.maxAttempts()).isEqualTo(3);
        assertThat(cfg.backoff()).isInstanceOf(ExponentialBackoff.class);
    }
}
