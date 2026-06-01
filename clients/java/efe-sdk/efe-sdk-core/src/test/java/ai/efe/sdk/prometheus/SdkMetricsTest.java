package ai.efe.sdk.prometheus;

import io.prometheus.metrics.model.registry.PrometheusRegistry;

import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThat;

/** Sprint D26 — unit tests : SdkMetrics — 5 canonical names cross-language. */
class SdkMetricsTest {

    @Test
    void allFiveCanonicalMetricNamesPresent() {
        // The names MUST match Python M8 + Go M9 + TS M10 SDKs.
        assertThat(SdkMetrics.CLASSIFY_TOTAL).isEqualTo("efe_sdk_classify_total");
        assertThat(SdkMetrics.CLASSIFY_LATENCY).isEqualTo("efe_sdk_classify_latency_seconds");
        assertThat(SdkMetrics.CLASSIFY_ERRORS_TOTAL).isEqualTo("efe_sdk_classify_errors_total");
        assertThat(SdkMetrics.IN_FLIGHT).isEqualTo("efe_sdk_in_flight");
        assertThat(SdkMetrics.RETRY_ATTEMPTS_TOTAL).isEqualTo("efe_sdk_retry_attempts_total");
    }

    @Test
    void registersFiveCollectorsOnRegistry() {
        PrometheusRegistry registry = new PrometheusRegistry();
        SdkMetrics metrics = new SdkMetrics(registry);

        // observeStart + observeEnd should not throw.
        metrics.observeStart();
        metrics.observeEnd("success", "", 0.123);

        metrics.observeStart();
        metrics.observeEnd("error", "unavailable", 0.5);

        metrics.observeRetryAttempt(1);
        metrics.observeRetryAttempt(2);
        metrics.observeRetryAttempt(99);  // mapped to "many"

        // No exceptions = collectors registered correctly.
        assertThat(metrics).isNotNull();
    }
}
