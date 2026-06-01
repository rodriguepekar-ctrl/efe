/*
 * Sprint D26 (Φ-B) — Resolved client options (internal record).
 *
 * Built by EfeClient.Builder ; consumed by ChannelFactory.
 */
package ai.efe.sdk.internal;

import ai.efe.sdk.auth.AuthConfig;
import ai.efe.sdk.retry.RetryConfig;

import io.opentelemetry.api.OpenTelemetry;
import io.prometheus.metrics.model.registry.PrometheusRegistry;

import org.jspecify.annotations.Nullable;

import java.time.Duration;

/**
 * Internal — resolved options after Builder defaults applied.
 * NOT part of the public surface.
 */
public record ClientOptions(
    String endpoint,
    AuthConfig auth,
    RetryConfig retry,
    Duration deadline,
    String userAgent,
    int maxReceivedMessageSize,
    int streamConcurrency,
    @Nullable OpenTelemetry openTelemetry,
    @Nullable PrometheusRegistry prometheusRegistry
) {}
