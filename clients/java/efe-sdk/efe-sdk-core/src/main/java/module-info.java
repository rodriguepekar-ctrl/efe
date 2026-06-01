/*
 * Sprint D26 (Φ-B) — JPMS module declaration for ai.efe.sdk.
 *
 * Pin tested by `test_d26_java_sdk_module_info_present`.
 *
 * Public surface :
 *   - ai.efe.sdk           : EfeClient, Verdict, StreamItem, Version
 *   - ai.efe.sdk.auth      : AuthConfig + JwtAuth/MtlsAuth/NoAuth + JwtCallCredentials/MtlsBuilder
 *   - ai.efe.sdk.exception : EfeException + 10 sentinels
 *   - ai.efe.sdk.retry     : RetryConfig, RetryBackoff, ExponentialBackoff, RetryClientInterceptor
 *   - ai.efe.sdk.otel      : OtelClientInterceptor (soft dep)
 *   - ai.efe.sdk.prometheus: SdkMetrics, PrometheusClientInterceptor (soft dep)
 *
 * Internal — explicitly NOT exported :
 *   - ai.efe.sdk.internal  : ChannelFactory, ClientOptions, ErrorMapper, StreamImpl
 */

/**
 * EFE SDK Java — official Java/JVM SDK for the EFE inference platform.
 *
 * @since 0.4.0
 */
module ai.efe.sdk {

    // Public surface
    exports ai.efe.sdk;
    exports ai.efe.sdk.auth;
    exports ai.efe.sdk.exception;
    exports ai.efe.sdk.retry;
    exports ai.efe.sdk.otel;
    exports ai.efe.sdk.prometheus;

    // Internal — INTENTIONALLY not exported
    // exports ai.efe.sdk.internal;

    // Required dependencies (transitively visible to consumers).
    requires transitive io.grpc.api;
    requires io.grpc.stub;
    requires io.grpc.protobuf;
    requires io.grpc.netty.shaded;
    requires io.grpc.services;
    requires com.google.common;
    requires com.google.protobuf;
    requires org.slf4j;

    // Compile-time / soft dep annotations.
    requires static org.jspecify;

    // Soft / peer-equivalent deps (provided scope in POM).
    requires static io.opentelemetry.api;
    requires static io.opentelemetry.context;
    requires static io.prometheus.metrics.core;
    requires static io.prometheus.metrics.model;
}
