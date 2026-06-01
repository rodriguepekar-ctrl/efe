/*
 * Sprint D26 (Φ-B) — JPMS module declaration for ai.efe.sdk.mock.
 */

/**
 * EFE SDK Java mock fixtures : MockEfeClient + MockEfeServer +
 * VerdictFactory.
 *
 * @since 0.4.0
 */
module ai.efe.sdk.mock {

    exports ai.efe.sdk.mock;

    requires transitive ai.efe.sdk;
    requires io.grpc.api;
    requires io.grpc.stub;
    requires io.grpc.inprocess;
    requires com.google.protobuf;

    requires static org.jspecify;
}
