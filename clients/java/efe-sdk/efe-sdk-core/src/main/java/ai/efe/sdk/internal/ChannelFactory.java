/*
 * Sprint D26 (Φ-B) — gRPC ManagedChannel factory.
 *
 * Wraps NettyChannelBuilder + InProcessChannelBuilder routing :
 *   - "in-process:<name>"  → InProcessChannel (used by MockEfeServer in tests).
 *   - "host:port" or URL   → NettyChannelBuilder (production).
 *
 * Wires the canonical interceptor chain : auth → otel → prom → retry.
 */
package ai.efe.sdk.internal;

import ai.efe.sdk.auth.AuthConfig;
import ai.efe.sdk.auth.JwtAuth;
import ai.efe.sdk.auth.JwtCallCredentials;
import ai.efe.sdk.auth.MtlsAuth;
import ai.efe.sdk.auth.MtlsBuilder;
import ai.efe.sdk.auth.NoAuth;
import ai.efe.sdk.otel.OtelClientInterceptor;
import ai.efe.sdk.prometheus.PrometheusClientInterceptor;
import ai.efe.sdk.prometheus.SdkMetrics;
import ai.efe.sdk.retry.RetryClientInterceptor;

import io.grpc.CallCredentials;
import io.grpc.ClientInterceptor;
import io.grpc.ManagedChannel;
import io.grpc.inprocess.InProcessChannelBuilder;
import io.grpc.netty.shaded.io.grpc.netty.NettyChannelBuilder;
import io.grpc.netty.shaded.io.netty.handler.ssl.SslContext;

import java.util.ArrayList;
import java.util.List;

/**
 * Factory : builds the {@link ManagedChannel} + the (channel-level)
 * gRPC interceptor chain that {@link ai.efe.sdk.EfeClient} uses
 * internally.
 */
public final class ChannelFactory {

    public static final String IN_PROCESS_PREFIX = "in-process:";

    private ChannelFactory() {}

    /**
     * Materialised gRPC channel + JWT call credentials (if any).
     *
     * <p>JWT auth uses {@link CallCredentials} (per-RPC) instead of a
     * channel-level interceptor — that's the canonical gRPC Java
     * pattern + supports dynamic token refresh in future iterations.
     */
    public record ChannelHandle(ManagedChannel channel, CallCredentials callCredentials) {}

    /** Build the channel from the resolved options. */
    public static ChannelHandle create(ClientOptions opts) {
        ManagedChannel channel = buildChannel(opts);
        CallCredentials creds = (opts.auth() instanceof JwtAuth jwt)
            ? new JwtCallCredentials(jwt.token())
            : null;
        return new ChannelHandle(channel, creds);
    }

    private static ManagedChannel buildChannel(ClientOptions opts) {
        String endpoint = opts.endpoint();
        List<ClientInterceptor> interceptors = buildInterceptors(opts);

        if (endpoint.startsWith(IN_PROCESS_PREFIX)) {
            String name = endpoint.substring(IN_PROCESS_PREFIX.length());
            InProcessChannelBuilder builder = InProcessChannelBuilder.forName(name)
                .directExecutor()
                .userAgent(opts.userAgent())
                .maxInboundMessageSize(opts.maxReceivedMessageSize());
            interceptors.forEach(builder::intercept);
            return builder.build();
        }

        NettyChannelBuilder builder = NettyChannelBuilder.forTarget(endpoint)
            .userAgent(opts.userAgent())
            .maxInboundMessageSize(opts.maxReceivedMessageSize());

        switch (opts.auth()) {
            case JwtAuth ignored -> builder.useTransportSecurity();
            case MtlsAuth mtls -> {
                SslContext sslContext = MtlsBuilder.build(mtls);
                builder.sslContext(sslContext);
            }
            case NoAuth ignored -> builder.usePlaintext();
        }

        interceptors.forEach(builder::intercept);
        return builder.build();
    }

    /**
     * Canonical interceptor chain order (innermost → outermost) :
     * auth (CallCredentials, NOT in this list) → otel → prom → retry.
     *
     * <p>gRPC interceptor application order is REVERSE of registration
     * order : the LAST registered interceptor runs FIRST. So we
     * register retry first, then prom, then otel.
     */
    private static List<ClientInterceptor> buildInterceptors(ClientOptions opts) {
        List<ClientInterceptor> list = new ArrayList<>(3);
        // retry registered first → runs LAST (outermost) — retries each
        // attempt through the inner chain (otel + prom).
        list.add(new RetryClientInterceptor(opts.retry()));
        if (opts.prometheusRegistry() != null) {
            list.add(new PrometheusClientInterceptor(new SdkMetrics(opts.prometheusRegistry())));
        }
        if (opts.openTelemetry() != null) {
            list.add(new OtelClientInterceptor(opts.openTelemetry()));
        }
        return list;
    }
}
