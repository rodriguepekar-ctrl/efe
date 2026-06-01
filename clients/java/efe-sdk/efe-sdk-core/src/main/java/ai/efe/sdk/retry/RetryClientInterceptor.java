/*
 * Sprint D26 (Φ-B) — Retry ClientInterceptor.
 *
 * Wraps each unary call with an exponential-backoff retry on
 * retry-eligible errors (UNAVAILABLE, DEADLINE_EXCEEDED, RESOURCE_EXHAUSTED).
 *
 * Order in the gRPC interceptor chain : LAST (outermost). Retries
 * are counted as separate observations + each retry runs through
 * auth + otel independently (fresh span per retry).
 */
package ai.efe.sdk.retry;

import io.grpc.CallOptions;
import io.grpc.Channel;
import io.grpc.ClientCall;
import io.grpc.ClientInterceptor;
import io.grpc.ForwardingClientCall;
import io.grpc.ForwardingClientCallListener;
import io.grpc.Metadata;
import io.grpc.MethodDescriptor;
import io.grpc.Status;

import java.util.Set;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Retry interceptor that retries up to {@code config.maxAttempts}
 * times on retryable gRPC status codes.
 *
 * @since 0.4.0
 */
public final class RetryClientInterceptor implements ClientInterceptor {

    private static final Set<Status.Code> RETRYABLE_CODES = Set.of(
        Status.Code.UNAVAILABLE,
        Status.Code.DEADLINE_EXCEEDED,
        Status.Code.RESOURCE_EXHAUSTED
    );

    private final RetryConfig config;
    private final ScheduledExecutorService scheduler;

    public RetryClientInterceptor(RetryConfig config) {
        this.config = config;
        this.scheduler = Executors.newSingleThreadScheduledExecutor(r -> {
            Thread t = new Thread(r, "efe-sdk-retry-scheduler");
            t.setDaemon(true);
            return t;
        });
    }

    @Override
    public <ReqT, RespT> ClientCall<ReqT, RespT> interceptCall(
            MethodDescriptor<ReqT, RespT> method,
            CallOptions callOptions,
            Channel next) {
        if (config.maxAttempts() == 1) {
            // No retry — pass through.
            return next.newCall(method, callOptions);
        }
        return new RetryingClientCall<>(method, callOptions, next, config, scheduler);
    }

    /**
     * Wraps the user's listener + retries on UNAVAILABLE/DEADLINE_EXCEEDED/RESOURCE_EXHAUSTED.
     *
     * <p>Implementation note : we capture the original request via
     * {@code sendMessage} + replay it on each retry. Suitable for unary
     * calls only (D26 scope). For streaming, retry semantics get more
     * complex — deferred D27+.
     */
    private static final class RetryingClientCall<ReqT, RespT>
            extends ForwardingClientCall<ReqT, RespT> {

        private final MethodDescriptor<ReqT, RespT> method;
        private final CallOptions callOptions;
        private final Channel channel;
        private final RetryConfig config;
        private final ScheduledExecutorService scheduler;

        private final AtomicReference<ClientCall<ReqT, RespT>> currentCall = new AtomicReference<>();
        private ClientCall.Listener<RespT> userListener;
        private Metadata userHeaders;
        private ReqT pendingMessage;
        private int attempt = 0;

        RetryingClientCall(MethodDescriptor<ReqT, RespT> method, CallOptions callOptions,
                            Channel channel, RetryConfig config,
                            ScheduledExecutorService scheduler) {
            this.method = method;
            this.callOptions = callOptions;
            this.channel = channel;
            this.config = config;
            this.scheduler = scheduler;
        }

        @Override
        protected ClientCall<ReqT, RespT> delegate() {
            ClientCall<ReqT, RespT> c = currentCall.get();
            if (c == null) {
                throw new IllegalStateException("delegate accessed before start");
            }
            return c;
        }

        @Override
        public void start(Listener<RespT> responseListener, Metadata headers) {
            this.userListener = responseListener;
            this.userHeaders = headers;
            startNewCall();
        }

        @Override
        public void sendMessage(ReqT message) {
            this.pendingMessage = message;
            currentCall.get().sendMessage(message);
        }

        private void startNewCall() {
            attempt += 1;
            ClientCall<ReqT, RespT> newCall = channel.newCall(method, callOptions);
            currentCall.set(newCall);
            newCall.start(new RetryListener(), userHeaders);
            // request 1 is implied by the unary contract on user side.
            newCall.request(1);
            if (pendingMessage != null) {
                newCall.sendMessage(pendingMessage);
                newCall.halfClose();
            }
        }

        private final class RetryListener extends ForwardingClientCallListener.SimpleForwardingClientCallListener<RespT> {

            RetryListener() {
                super(userListener);
            }

            @Override
            public void onClose(Status status, Metadata trailers) {
                if (status.isOk()) {
                    super.onClose(status, trailers);
                    return;
                }
                if (attempt >= config.maxAttempts() || !RETRYABLE_CODES.contains(status.getCode())) {
                    super.onClose(status, trailers);
                    return;
                }
                // Schedule retry.
                long delayMs = config.backoff().next(attempt).toMillis();
                scheduler.schedule(() -> {
                    try {
                        startNewCall();
                    } catch (Throwable t) {
                        userListener.onClose(Status.INTERNAL
                            .withDescription("retry scheduling failed")
                            .withCause(t), new Metadata());
                    }
                }, delayMs, TimeUnit.MILLISECONDS);
            }
        }
    }
}
