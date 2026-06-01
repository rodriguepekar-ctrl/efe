/*
 * Sprint D26 (Φ-B) — `EfeClient` : the public Java entry point.
 *
 * Architecture :
 *   1. Builder validates options + builds the gRPC ManagedChannel via
 *      ChannelFactory. Lazy gRPC dial — no I/O at builder.build() time.
 *   2. The interceptor chain auth → otel → prom → retry is applied
 *      automatically by ChannelFactory.
 *   3. classify() uses BlockingStub ; classifyAsync() uses FutureStub
 *      → CompletableFuture conversion.
 *   4. classifyStream() spawns a worker pool via StreamImpl.
 *
 * Concurrency : EfeClient instances are SAFE for concurrent use.
 *
 * Lifecycle :
 *   - try (var client = EfeClient.builder()...build()) { ... }
 *   - close() shuts down the channel + interrupts in-flight RPCs.
 *
 * Cf. docs/EFE_JAVA_SDK_EXTENSION_ARCHITECTURE.md §10.
 */
package ai.efe.sdk;

import ai.efe.sdk.auth.AuthConfig;
import ai.efe.sdk.auth.NoAuth;
import ai.efe.sdk.exception.*;
import ai.efe.sdk.internal.ChannelFactory;
import ai.efe.sdk.internal.ChannelFactory.ChannelHandle;
import ai.efe.sdk.internal.ClientOptions;
import ai.efe.sdk.internal.ErrorMapper;
import ai.efe.sdk.internal.StreamImpl;
import ai.efe.sdk.retry.RetryConfig;

import com.google.common.util.concurrent.FutureCallback;
import com.google.common.util.concurrent.Futures;
import com.google.common.util.concurrent.ListenableFuture;
import com.google.common.util.concurrent.MoreExecutors;

import efe.v1.Efe.ExecuteRequest;
import efe.v1.Efe.ExecuteResponse;
import efe.v1.SwarmServiceGrpc;
import efe.v1.SwarmServiceGrpc.SwarmServiceBlockingStub;
import efe.v1.SwarmServiceGrpc.SwarmServiceFutureStub;
import efe.v1.Types;

import io.grpc.ManagedChannel;
import io.grpc.StatusRuntimeException;
import io.grpc.health.v1.HealthCheckRequest;
import io.grpc.health.v1.HealthCheckResponse;
import io.grpc.health.v1.HealthGrpc;

import io.opentelemetry.api.OpenTelemetry;
import io.prometheus.metrics.model.registry.PrometheusRegistry;

import org.jspecify.annotations.NonNull;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.time.Duration;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import java.util.stream.Stream;

/**
 * EfeClient — main entry point of the EFE Java SDK.
 *
 * <p>Construct via the builder pattern :
 *
 * <pre>{@code
 * try (var client = EfeClient.builder()
 *         .endpoint("staging.efe.ai:50051")
 *         .auth(JwtAuth.of(System.getenv("EFE_JWT_TOKEN")))
 *         .deadline(Duration.ofSeconds(10))
 *         .build()) {
 *     Verdict verdict = client.classify("I want a refund");
 *     if (verdict.isTrusted()) {
 *         // ...
 *     }
 * }
 * }</pre>
 *
 * <p>Lifecycle :
 * <ul>
 *   <li>{@code builder().build()} → ready (lazy gRPC dial — no I/O yet).
 *   <li>{@code classify}, {@code classifyAsync}, {@code classifyStream},
 *       {@code healthcheck} — concurrent OK.
 *   <li>{@code close()} → idempotent. Subsequent calls throw
 *       {@link EfeClosedException}.
 * </ul>
 *
 * <p>Concurrency : instances are SAFE for concurrent use. Internally
 * uses a single {@code ManagedChannel} that multiplexes HTTP/2 streams.
 *
 * @since 0.4.0
 */
public final class EfeClient implements AutoCloseable {

    private static final Logger LOG = LoggerFactory.getLogger(EfeClient.class);

    private final ManagedChannel channel;
    private final SwarmServiceBlockingStub blockingStub;
    private final SwarmServiceFutureStub futureStub;
    private final HealthGrpc.HealthBlockingStub healthBlockingStub;
    private final ClientOptions opts;
    private volatile boolean closed = false;

    private EfeClient(Builder builder) {
        this.opts = builder.toOptions();
        ChannelHandle handle = ChannelFactory.create(opts);
        this.channel = handle.channel();

        SwarmServiceBlockingStub bs = SwarmServiceGrpc.newBlockingStub(channel);
        SwarmServiceFutureStub fs = SwarmServiceGrpc.newFutureStub(channel);
        HealthGrpc.HealthBlockingStub hs = HealthGrpc.newBlockingStub(channel);

        // Apply JWT credentials at the stub level (per-RPC).
        if (handle.callCredentials() != null) {
            bs = bs.withCallCredentials(handle.callCredentials());
            fs = fs.withCallCredentials(handle.callCredentials());
            hs = hs.withCallCredentials(handle.callCredentials());
        }
        this.blockingStub = bs;
        this.futureStub = fs;
        this.healthBlockingStub = hs;

        LOG.info("efe-sdk-java: client constructed (endpoint={}, deadline={}ms, retry_max={})",
            opts.endpoint(), opts.deadline().toMillis(), opts.retry().maxAttempts());
    }

    /** Returns a new {@link Builder}. */
    public static Builder builder() {
        return new Builder();
    }

    /**
     * Send one classification request synchronously.
     *
     * @param text input text (non-null, non-blank).
     * @return verdict.
     * @throws EfeInvalidArgumentException if text is null or blank.
     * @throws EfeUnauthenticatedException on auth failure.
     * @throws EfeTimeoutException on deadline exceeded.
     * @throws EfeUnavailableException on transient unavailability.
     * @throws EfeRateLimitedException on quota exceeded.
     * @throws EfeClosedException if {@link #close()} was already called.
     */
    public Verdict classify(@NonNull String text) {
        if (closed) throw new EfeClosedException("cannot issue RPC after close()");
        Objects.requireNonNull(text, "text");
        if (text.isBlank()) {
            throw new EfeInvalidArgumentException("text must be non-blank");
        }
        long startNanos = System.nanoTime();
        try {
            ExecuteRequest req = ExecuteRequest.newBuilder().setRawPrompt(text).build();
            ExecuteResponse resp = blockingStub
                .withDeadlineAfter(opts.deadline().toMillis(), TimeUnit.MILLISECONDS)
                .execute(req);
            long elapsedMs = (System.nanoTime() - startNanos) / 1_000_000L;
            return verdictFromExecuteResponse(resp, elapsedMs);
        } catch (StatusRuntimeException e) {
            throw ErrorMapper.fromGrpcStatus(e);
        }
    }

    /**
     * Send one classification request asynchronously.
     *
     * @return CompletableFuture that completes with the Verdict, or
     *         exceptionally with one of the {@link EfeException} subclasses.
     */
    public CompletableFuture<Verdict> classifyAsync(@NonNull String text) {
        if (closed) {
            return CompletableFuture.failedFuture(new EfeClosedException("client is closed"));
        }
        if (text == null || text.isBlank()) {
            return CompletableFuture.failedFuture(
                new EfeInvalidArgumentException("text must be non-blank"));
        }
        long startNanos = System.nanoTime();
        ExecuteRequest req = ExecuteRequest.newBuilder().setRawPrompt(text).build();
        ListenableFuture<ExecuteResponse> grpcFuture = futureStub
            .withDeadlineAfter(opts.deadline().toMillis(), TimeUnit.MILLISECONDS)
            .execute(req);
        CompletableFuture<Verdict> result = new CompletableFuture<>();
        Futures.addCallback(grpcFuture, new FutureCallback<ExecuteResponse>() {
            @Override
            public void onSuccess(ExecuteResponse resp) {
                long elapsedMs = (System.nanoTime() - startNanos) / 1_000_000L;
                result.complete(verdictFromExecuteResponse(resp, elapsedMs));
            }
            @Override
            public void onFailure(Throwable t) {
                if (t instanceof StatusRuntimeException sre) {
                    result.completeExceptionally(ErrorMapper.fromGrpcStatus(sre));
                } else {
                    result.completeExceptionally(t);
                }
            }
        }, MoreExecutors.directExecutor());
        return result;
    }

    /**
     * Concurrent batch classification with input-order index preservation.
     *
     * @param texts input source (non-null).
     * @return Stream of {@link StreamItem} — each item's {@code index}
     *         reflects the position in the input iterable.
     */
    public Stream<StreamItem> classifyStream(@NonNull Iterable<String> texts) {
        if (closed) throw new EfeClosedException("cannot issue RPC after close()");
        Objects.requireNonNull(texts, "texts");
        return StreamImpl.run(this, texts, opts.streamConcurrency());
    }

    /**
     * Probe the server's gRPC standard Health endpoint.
     *
     * @throws EfeUnavailableException on NOT_SERVING / unreachable.
     * @throws EfeTimeoutException on deadline exceeded.
     */
    public void healthcheck() {
        if (closed) throw new EfeClosedException("cannot issue RPC after close()");
        try {
            HealthCheckResponse resp = healthBlockingStub
                .withDeadlineAfter(opts.deadline().toMillis(), TimeUnit.MILLISECONDS)
                .check(HealthCheckRequest.newBuilder().setService("").build());
            if (resp.getStatus() != HealthCheckResponse.ServingStatus.SERVING) {
                throw new EfeUnavailableException(
                    "server reports status=" + resp.getStatus());
            }
        } catch (StatusRuntimeException e) {
            throw ErrorMapper.fromGrpcStatus(e);
        }
    }

    /**
     * Release the underlying gRPC connection. Idempotent — subsequent
     * calls are no-ops, but {@code classify}/{@code healthcheck}/
     * {@code classifyAsync}/{@code classifyStream} throw {@link EfeClosedException}.
     */
    @Override
    public void close() {
        if (closed) return;
        closed = true;
        channel.shutdown();
        try {
            if (!channel.awaitTermination(5, TimeUnit.SECONDS)) {
                channel.shutdownNow();
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            channel.shutdownNow();
        }
    }

    /** Returns true after {@link #close()} has been called. */
    public boolean isClosed() {
        return closed;
    }

    /** Redacted summary, safe for logs (NEVER includes JWT / mTLS material). */
    @Override
    public String toString() {
        return "EfeClient[endpoint=" + opts.endpoint()
            + " closed=" + closed
            + " deadline=" + opts.deadline().toMillis() + "ms]";
    }

    /**
     * Project a wire-shape {@link ExecuteResponse} into the public {@link Verdict}.
     */
    private static Verdict verdictFromExecuteResponse(ExecuteResponse resp, long elapsedMs) {
        boolean reject = false;
        String zone;
        switch (resp.getVerdict()) {
            case VERDICT_DIVERGENCE, VERDICT_MODE_COLLAPSE -> {
                reject = true;
                zone = "reject";
            }
            case VERDICT_CONVERGED -> zone = Verdict.deriveZone(resp.getConfidence());
            case VERDICT_CONTINUE -> zone = "warn";
            default -> zone = "warn";
        }
        return new Verdict(
            resp.hasOutput() ? resp.getOutput().getText() : "",
            resp.getConfidence(),
            reject,
            resp.getRoundsExecuted(),
            resp.getAlignmentDelta(),
            "",  // catalog — server-side metadata (D26 baseline empty).
            zone,
            resp.getSwarmId(),
            resp.getRequestId(),
            elapsedMs
        );
    }

    // ─── Builder ───
    public static final class Builder {
        private @Nullable String endpoint;
        private AuthConfig auth = NoAuth.INSTANCE;
        private RetryConfig retry = RetryConfig.exponential(3,
            Duration.ofMillis(100), Duration.ofSeconds(30));
        private Duration deadline = Duration.ofSeconds(10);
        private String userAgent = "efe-sdk-java/" + Version.VERSION;
        private int maxReceivedMessageSize = 16 * 1024 * 1024;
        private int streamConcurrency = 8;
        private @Nullable OpenTelemetry openTelemetry;
        private @Nullable PrometheusRegistry prometheusRegistry;

        private Builder() {}

        public Builder endpoint(String endpoint) {
            this.endpoint = Objects.requireNonNull(endpoint, "endpoint");
            return this;
        }

        public Builder auth(AuthConfig auth) {
            this.auth = Objects.requireNonNull(auth, "auth");
            return this;
        }

        public Builder retry(RetryConfig retry) {
            this.retry = Objects.requireNonNull(retry, "retry");
            return this;
        }

        public Builder deadline(Duration deadline) {
            Objects.requireNonNull(deadline, "deadline");
            if (deadline.isNegative()) {
                throw new EfeInvalidArgumentException("deadline must be ≥ 0");
            }
            this.deadline = deadline;
            return this;
        }

        public Builder userAgent(String ua) {
            Objects.requireNonNull(ua, "userAgent");
            if (ua.isBlank()) {
                throw new EfeInvalidArgumentException("userAgent must be non-blank");
            }
            this.userAgent = ua;
            return this;
        }

        public Builder maxReceivedMessageSize(int bytes) {
            if (bytes <= 0) {
                throw new EfeInvalidArgumentException("maxReceivedMessageSize must be > 0");
            }
            this.maxReceivedMessageSize = bytes;
            return this;
        }

        public Builder streamConcurrency(int n) {
            if (n < 1) {
                throw new EfeInvalidArgumentException("streamConcurrency must be ≥ 1");
            }
            this.streamConcurrency = n;
            return this;
        }

        public Builder otel(OpenTelemetry otel) {
            this.openTelemetry = Objects.requireNonNull(otel, "otel");
            return this;
        }

        public Builder prometheus(PrometheusRegistry registry) {
            this.prometheusRegistry = Objects.requireNonNull(registry, "registry");
            return this;
        }

        /** Validate + materialise the {@link EfeClient}. */
        public EfeClient build() {
            if (endpoint == null || endpoint.isBlank()) {
                throw new EfeInvalidArgumentException("endpoint is required");
            }
            return new EfeClient(this);
        }

        ClientOptions toOptions() {
            return new ClientOptions(
                endpoint,
                auth,
                retry,
                deadline,
                userAgent,
                maxReceivedMessageSize,
                streamConcurrency,
                openTelemetry,
                prometheusRegistry
            );
        }
    }
}
