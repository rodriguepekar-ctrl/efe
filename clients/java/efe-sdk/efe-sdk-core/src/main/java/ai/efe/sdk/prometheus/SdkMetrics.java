/*
 * Sprint D26 (Φ-B) — Prometheus metrics.
 *
 * 5 canonical collectors with names that MATCH Python M8 + Go M9 +
 * TS M10 SDKs exactly. Bumping a name = breaking every Grafana
 * dashboard + Prom alert. Pin tested.
 */
package ai.efe.sdk.prometheus;

import io.prometheus.metrics.core.metrics.Counter;
import io.prometheus.metrics.core.metrics.Gauge;
import io.prometheus.metrics.core.metrics.Histogram;
import io.prometheus.metrics.model.registry.PrometheusRegistry;

import java.util.Objects;

/**
 * Bag of 5 Prometheus collectors registered against a
 * {@link PrometheusRegistry}.
 *
 * <p>Lifecycle : one instance per {@code EfeClient}. The registry is
 * supplied by the consumer.
 *
 * @since 0.4.0
 */
public final class SdkMetrics {

    /** Stable canonical metric names — match Go SDK + Python SDK + TS SDK. */
    public static final String CLASSIFY_TOTAL = "efe_sdk_classify_total";
    public static final String CLASSIFY_LATENCY = "efe_sdk_classify_latency_seconds";
    public static final String CLASSIFY_ERRORS_TOTAL = "efe_sdk_classify_errors_total";
    public static final String IN_FLIGHT = "efe_sdk_in_flight";
    public static final String RETRY_ATTEMPTS_TOTAL = "efe_sdk_retry_attempts_total";

    private final Counter classifyTotal;
    private final Histogram classifyLatency;
    private final Counter classifyErrors;
    private final Gauge inFlight;
    private final Counter retryAttempts;

    public SdkMetrics(PrometheusRegistry registry) {
        Objects.requireNonNull(registry, "registry");

        this.classifyTotal = Counter.builder()
            .name(CLASSIFY_TOTAL)
            .help("Total efe_sdk Classify calls, partitioned by outcome.")
            .labelNames("outcome")
            .register(registry);

        this.classifyLatency = Histogram.builder()
            .name(CLASSIFY_LATENCY)
            .help("Classify wall-clock latency in seconds, partitioned by outcome.")
            .labelNames("outcome")
            .classicUpperBounds(0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0)
            .register(registry);

        this.classifyErrors = Counter.builder()
            .name(CLASSIFY_ERRORS_TOTAL)
            .help("Total efe_sdk Classify errors, partitioned by sentinel code.")
            .labelNames("sentinel")
            .register(registry);

        this.inFlight = Gauge.builder()
            .name(IN_FLIGHT)
            .help("Currently in-flight Classify calls.")
            .register(registry);

        this.retryAttempts = Counter.builder()
            .name(RETRY_ATTEMPTS_TOTAL)
            .help("Total retry attempts, partitioned by attempt number.")
            .labelNames("attempt")
            .register(registry);
    }

    /** Mark an in-flight RPC start. */
    public void observeStart() {
        inFlight.inc();
    }

    /** Mark an in-flight RPC end + record latency + outcome. */
    public void observeEnd(String outcome, String sentinel, double seconds) {
        inFlight.dec();
        classifyTotal.labelValues(outcome).inc();
        classifyLatency.labelValues(outcome).observe(seconds);
        if ("error".equals(outcome) && sentinel != null && !sentinel.isEmpty()) {
            classifyErrors.labelValues(sentinel).inc();
        }
    }

    /** Count a retry attempt. */
    public void observeRetryAttempt(int attempt) {
        retryAttempts.labelValues(formatAttempt(attempt)).inc();
    }

    private static String formatAttempt(int n) {
        return n >= 1 && n <= 5 ? Integer.toString(n) : "many";
    }
}
