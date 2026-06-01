/*
 * Sprint D26 (Φ-B) — Prometheus ClientInterceptor.
 *
 * Wraps each call with start/end observations on the SdkMetrics bag.
 */
package ai.efe.sdk.prometheus;

import ai.efe.sdk.exception.EfeException;
import ai.efe.sdk.internal.ErrorMapper;

import io.grpc.CallOptions;
import io.grpc.Channel;
import io.grpc.ClientCall;
import io.grpc.ClientInterceptor;
import io.grpc.ForwardingClientCall;
import io.grpc.ForwardingClientCallListener;
import io.grpc.Metadata;
import io.grpc.MethodDescriptor;
import io.grpc.Status;

/**
 * Observes start/end of every RPC + records to the 5 canonical
 * SdkMetrics collectors.
 *
 * @since 0.4.0
 */
public final class PrometheusClientInterceptor implements ClientInterceptor {

    private final SdkMetrics metrics;

    public PrometheusClientInterceptor(SdkMetrics metrics) {
        this.metrics = metrics;
    }

    @Override
    public <ReqT, RespT> ClientCall<ReqT, RespT> interceptCall(
            MethodDescriptor<ReqT, RespT> method,
            CallOptions callOptions,
            Channel next) {
        long startNanos = System.nanoTime();
        metrics.observeStart();

        return new ForwardingClientCall.SimpleForwardingClientCall<ReqT, RespT>(
            next.newCall(method, callOptions)) {
            @Override
            public void start(Listener<RespT> responseListener, Metadata headers) {
                super.start(new ForwardingClientCallListener.SimpleForwardingClientCallListener<RespT>(
                    responseListener) {
                    @Override
                    public void onClose(Status status, Metadata trailers) {
                        double seconds = (System.nanoTime() - startNanos) / 1_000_000_000.0;
                        if (status.isOk()) {
                            metrics.observeEnd("success", "", seconds);
                        } else {
                            EfeException sdkErr = ErrorMapper.fromGrpcStatus(status);
                            metrics.observeEnd("error", sdkErr.code(), seconds);
                        }
                        super.onClose(status, trailers);
                    }
                }, headers);
            }
        };
    }
}
