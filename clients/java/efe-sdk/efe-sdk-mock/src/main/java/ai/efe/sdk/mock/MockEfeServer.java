/*
 * Sprint D26 (Φ-B) — In-process gRPC mock server.
 *
 * Spawns a real gRPC server backed by InProcessServerBuilder (zero
 * socket — pure JVM transport). Equivalent to Go SDK bufconn or TS
 * SDK in-memory grpc-js server.
 */
package ai.efe.sdk.mock;

import efe.v1.Efe.ExecuteRequest;
import efe.v1.Efe.ExecuteResponse;
import efe.v1.SwarmServiceGrpc;

import io.grpc.Server;
import io.grpc.Status;
import io.grpc.inprocess.InProcessServerBuilder;
import io.grpc.stub.StreamObserver;

import org.jspecify.annotations.Nullable;

import java.io.IOException;
import java.util.UUID;
import java.util.concurrent.TimeUnit;
import java.util.function.Function;

/**
 * In-process gRPC mock server for SDK integration tests.
 *
 * <p>Use case : exercise the FULL gRPC stack (interceptor chain, retry,
 * OTel context propagation, transport) without going through a real
 * network. The endpoint format is {@code "in-process:<name>"} which
 * the SDK's {@link ai.efe.sdk.internal.ChannelFactory} routes to
 * {@link io.grpc.inprocess.InProcessChannelBuilder}.
 *
 * @example
 * <pre>{@code
 * try (var server = MockEfeServer.create();
 *      var client = EfeClient.builder().endpoint(server.getEndpoint()).build()) {
 *
 *     server.onClassify(req -> ExecuteResponse.newBuilder()
 *         .setOutput(DecodedOutput.newBuilder().setText("ok").build())
 *         .setConfidence(0.9f)
 *         .setVerdict(Verdict.VERDICT_CONVERGED)
 *         .build());
 *
 *     Verdict v = client.classify("hello");
 * }
 * }</pre>
 *
 * @since 0.4.0
 */
public final class MockEfeServer implements AutoCloseable {

    private final Server server;
    private final String name;
    private @Nullable Function<ExecuteRequest, ExecuteResponse> classifyHandler;

    /** Build a fresh mock server with a randomized name. Already started. */
    public static MockEfeServer create() throws IOException {
        return new MockEfeServer("efe-mock-" + UUID.randomUUID());
    }

    private MockEfeServer(String name) throws IOException {
        this.name = name;
        this.server = InProcessServerBuilder
            .forName(name)
            .directExecutor()
            .addService(new MockSwarmService())
            .build()
            .start();
    }

    /**
     * Endpoint URL to pass to {@code EfeClient.builder().endpoint(...)}.
     *
     * <p>Format : {@code "in-process:<random-uuid>"}.
     */
    public String getEndpoint() {
        return "in-process:" + name;
    }

    /** Register the handler invoked on each Execute RPC. */
    public void onClassify(Function<ExecuteRequest, ExecuteResponse> handler) {
        this.classifyHandler = handler;
    }

    @Override
    public void close() {
        server.shutdownNow();
        try {
            server.awaitTermination(5, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    private final class MockSwarmService extends SwarmServiceGrpc.SwarmServiceImplBase {
        @Override
        public void execute(ExecuteRequest request, StreamObserver<ExecuteResponse> responseObserver) {
            if (classifyHandler == null) {
                responseObserver.onError(Status.UNIMPLEMENTED
                    .withDescription("mock: onClassify not configured")
                    .asRuntimeException());
                return;
            }
            try {
                ExecuteResponse resp = classifyHandler.apply(request);
                responseObserver.onNext(resp);
                responseObserver.onCompleted();
            } catch (Throwable t) {
                if (t instanceof io.grpc.StatusRuntimeException sre) {
                    responseObserver.onError(sre);
                } else {
                    responseObserver.onError(Status.INTERNAL
                        .withDescription(t.getMessage() != null ? t.getMessage() : "mock failure")
                        .withCause(t)
                        .asRuntimeException());
                }
            }
        }
    }
}
