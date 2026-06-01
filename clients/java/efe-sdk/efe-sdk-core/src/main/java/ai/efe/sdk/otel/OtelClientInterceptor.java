/*
 * Sprint D26 (Φ-B) — OpenTelemetry interceptor.
 *
 * SOFT dep : opentelemetry-api is `provided` scope. If the consumer
 * passes an OpenTelemetry instance to the builder, the SDK wires this
 * interceptor ; otherwise it's not in the chain.
 */
package ai.efe.sdk.otel;

import ai.efe.sdk.Version;

import io.grpc.CallOptions;
import io.grpc.Channel;
import io.grpc.ClientCall;
import io.grpc.ClientInterceptor;
import io.grpc.ForwardingClientCall;
import io.grpc.ForwardingClientCallListener;
import io.grpc.Metadata;
import io.grpc.MethodDescriptor;
import io.grpc.Status;

import io.opentelemetry.api.OpenTelemetry;
import io.opentelemetry.api.trace.Span;
import io.opentelemetry.api.trace.SpanKind;
import io.opentelemetry.api.trace.StatusCode;
import io.opentelemetry.api.trace.Tracer;
import io.opentelemetry.context.Context;
import io.opentelemetry.context.Scope;

import java.util.Objects;

/**
 * Creates one client span per RPC named {@code efe/<MethodName>}.
 *
 * <p>Span attributes : {@code rpc.system=grpc},
 * {@code rpc.service=efe.v1.SwarmService}, {@code rpc.method=...}.
 *
 * <p>On error, the span is marked ERROR + the status description is
 * recorded.
 *
 * @since 0.4.0
 */
public final class OtelClientInterceptor implements ClientInterceptor {

    private final Tracer tracer;

    public OtelClientInterceptor(OpenTelemetry openTelemetry) {
        Objects.requireNonNull(openTelemetry, "openTelemetry");
        this.tracer = openTelemetry.getTracer("ai.efe.sdk", Version.VERSION);
    }

    @Override
    public <ReqT, RespT> ClientCall<ReqT, RespT> interceptCall(
            MethodDescriptor<ReqT, RespT> method,
            CallOptions callOptions,
            Channel next) {
        Span span = tracer.spanBuilder("efe/" + method.getBareMethodName())
            .setSpanKind(SpanKind.CLIENT)
            .setAttribute("rpc.system", "grpc")
            .setAttribute("rpc.service", method.getServiceName())
            .setAttribute("rpc.method", method.getBareMethodName())
            .startSpan();

        Context spanContext = Context.current().with(span);

        try (Scope ignored = spanContext.makeCurrent()) {
            return new ForwardingClientCall.SimpleForwardingClientCall<ReqT, RespT>(
                next.newCall(method, callOptions)) {
                @Override
                public void start(Listener<RespT> responseListener, Metadata headers) {
                    super.start(new ForwardingClientCallListener.SimpleForwardingClientCallListener<RespT>(
                        responseListener) {
                        @Override
                        public void onClose(Status status, Metadata trailers) {
                            if (!status.isOk()) {
                                span.setStatus(StatusCode.ERROR,
                                    status.getDescription() != null ? status.getDescription() : "");
                            }
                            span.end();
                            super.onClose(status, trailers);
                        }
                    }, headers);
                }
            };
        }
    }
}
