/*
 * Sprint D26 (Φ-B) — HAND-CODED PLACEHOLDER for protoc-gen-grpc-java output.
 *
 * In production, `mvn generate-sources` invokes `protoc-gen-grpc-java`
 * which produces this file under
 * `target/generated-sources/protobuf/grpc-java/efe/v1/SwarmServiceGrpc.java`.
 *
 * This hand-coded subset exposes ONLY the unary `Execute` RPC + its
 * BlockingStub / FutureStub / Stub variants — sufficient for the SDK
 * to compile + the integration tests via InProcessServer to work.
 *
 * Pin tested by `test_d26_java_sdk_swarm_service_grpc_present`.
 */
package efe.v1;

import io.grpc.CallOptions;
import io.grpc.Channel;
import io.grpc.MethodDescriptor;
import io.grpc.ServerServiceDefinition;
import io.grpc.ServiceDescriptor;
// **Audit fix M11-A4** — `ProtoUtils` import removed (was unused).
// Real protoc-gen-grpc-java output uses ProtoUtils ; the placeholder
// uses Java Serialization via PlaceholderMarshaller below.
import io.grpc.stub.AbstractAsyncStub;
import io.grpc.stub.AbstractBlockingStub;
import io.grpc.stub.AbstractFutureStub;
import io.grpc.stub.ClientCalls;
import io.grpc.stub.ServerCalls;
import io.grpc.stub.StreamObserver;

import com.google.common.util.concurrent.ListenableFuture;

import efe.v1.Efe.ExecuteRequest;
import efe.v1.Efe.ExecuteResponse;

/** Placeholder gRPC service descriptor for `efe.v1.SwarmService`. */
public final class SwarmServiceGrpc {

    private SwarmServiceGrpc() {}

    public static final String SERVICE_NAME = "efe.v1.SwarmService";

    /**
     * MethodDescriptor for SwarmService/Execute (unary).
     *
     * **Audit fix M11-A* placeholder** — this version uses Java's built-in
     * placeholder serialization (length-delimited byte arrays) ; real
     * `protoc-gen-grpc-java` output uses `ProtoLiteUtils` which serializes
     * the actual protobuf wire format. In tests via InProcessServer +
     * matching mock impl, both ends use the SAME placeholder serializer
     * so the round-trip works.
     */
    public static final MethodDescriptor<ExecuteRequest, ExecuteResponse> METHOD_EXECUTE =
        MethodDescriptor.<ExecuteRequest, ExecuteResponse>newBuilder()
            .setType(MethodDescriptor.MethodType.UNARY)
            .setFullMethodName(MethodDescriptor.generateFullMethodName(SERVICE_NAME, "Execute"))
            .setRequestMarshaller(new PlaceholderMarshaller<>(ExecuteRequest.class))
            .setResponseMarshaller(new PlaceholderMarshaller<>(ExecuteResponse.class))
            .setSampledToLocalTracing(true)
            .build();

    /** Service descriptor — used by gRPC server registration. */
    private static final ServiceDescriptor SERVICE_DESCRIPTOR =
        ServiceDescriptor.newBuilder(SERVICE_NAME)
            .addMethod(METHOD_EXECUTE)
            .build();

    /** Build the BlockingStub for synchronous calls. */
    public static SwarmServiceBlockingStub newBlockingStub(Channel channel) {
        return SwarmServiceBlockingStub.newStub(SwarmServiceBlockingStub::new, channel);
    }

    /** Build the FutureStub for ListenableFuture-returning calls. */
    public static SwarmServiceFutureStub newFutureStub(Channel channel) {
        return SwarmServiceFutureStub.newStub(SwarmServiceFutureStub::new, channel);
    }

    /** Build the Stub for async StreamObserver-style calls. */
    public static SwarmServiceStub newStub(Channel channel) {
        return SwarmServiceStub.newStub(SwarmServiceStub::new, channel);
    }

    /** Server-side base for SwarmService implementations. */
    public abstract static class SwarmServiceImplBase implements io.grpc.BindableService {
        public void execute(ExecuteRequest request, StreamObserver<ExecuteResponse> responseObserver) {
            ServerCalls.asyncUnimplementedUnaryCall(METHOD_EXECUTE, responseObserver);
        }

        @Override
        public final ServerServiceDefinition bindService() {
            return ServerServiceDefinition.builder(SERVICE_DESCRIPTOR)
                .addMethod(METHOD_EXECUTE, ServerCalls.asyncUnaryCall(this::execute))
                .build();
        }
    }

    // ─── BlockingStub ───
    public static final class SwarmServiceBlockingStub extends AbstractBlockingStub<SwarmServiceBlockingStub> {
        private SwarmServiceBlockingStub(Channel channel, CallOptions callOptions) {
            super(channel, callOptions);
        }

        @Override
        protected SwarmServiceBlockingStub build(Channel channel, CallOptions callOptions) {
            return new SwarmServiceBlockingStub(channel, callOptions);
        }

        public ExecuteResponse execute(ExecuteRequest request) {
            return ClientCalls.blockingUnaryCall(getChannel(), METHOD_EXECUTE, getCallOptions(), request);
        }
    }

    // ─── FutureStub ───
    public static final class SwarmServiceFutureStub extends AbstractFutureStub<SwarmServiceFutureStub> {
        private SwarmServiceFutureStub(Channel channel, CallOptions callOptions) {
            super(channel, callOptions);
        }

        @Override
        protected SwarmServiceFutureStub build(Channel channel, CallOptions callOptions) {
            return new SwarmServiceFutureStub(channel, callOptions);
        }

        public ListenableFuture<ExecuteResponse> execute(ExecuteRequest request) {
            return ClientCalls.futureUnaryCall(
                getChannel().newCall(METHOD_EXECUTE, getCallOptions()), request);
        }
    }

    // ─── Async Stub ───
    public static final class SwarmServiceStub extends AbstractAsyncStub<SwarmServiceStub> {
        private SwarmServiceStub(Channel channel, CallOptions callOptions) {
            super(channel, callOptions);
        }

        @Override
        protected SwarmServiceStub build(Channel channel, CallOptions callOptions) {
            return new SwarmServiceStub(channel, callOptions);
        }

        public void execute(ExecuteRequest request, StreamObserver<ExecuteResponse> responseObserver) {
            ClientCalls.asyncUnaryCall(
                getChannel().newCall(METHOD_EXECUTE, getCallOptions()), request, responseObserver);
        }
    }

    /**
     * Placeholder marshaller that uses Java Serialization for the message
     * round-trip. Replaced by ProtoLiteUtils-based marshallers when real
     * protoc-gen-grpc-java output is generated.
     *
     * **Audit fix M11-A2** — pre-fix wrapped values in a `SerializableHolder`
     * to bypass the fact that ExecuteRequest/ExecuteResponse weren't
     * Serializable. Now those placeholder classes implement Serializable
     * directly, so the marshaller is a clean ObjectOutputStream / ObjectInputStream
     * pair.
     *
     * Trade-off : the placeholder doesn't speak real proto wire format
     * (it speaks Java Serialization). Tests via InProcessServer use the
     * SAME marshaller on client + server, so the round-trip works. Real
     * gRPC servers (Lettre E in Rust) will reject this format until
     * `mvn generate-sources` overwrites these stubs.
     */
    private static final class PlaceholderMarshaller<T> implements MethodDescriptor.Marshaller<T> {
        @SuppressWarnings("unused")
        private final Class<T> type;

        PlaceholderMarshaller(Class<T> type) {
            this.type = type;
        }

        @Override
        public java.io.InputStream stream(T value) {
            try {
                java.io.ByteArrayOutputStream baos = new java.io.ByteArrayOutputStream();
                try (java.io.ObjectOutputStream oos = new java.io.ObjectOutputStream(baos)) {
                    oos.writeObject(value);
                }
                return new java.io.ByteArrayInputStream(baos.toByteArray());
            } catch (java.io.IOException e) {
                throw new RuntimeException("placeholder marshal failed", e);
            }
        }

        @Override
        @SuppressWarnings("unchecked")
        public T parse(java.io.InputStream stream) {
            try (java.io.ObjectInputStream ois = new java.io.ObjectInputStream(stream)) {
                return (T) ois.readObject();
            } catch (java.io.IOException | ClassNotFoundException e) {
                throw new RuntimeException("placeholder parse failed", e);
            }
        }
    }
}
