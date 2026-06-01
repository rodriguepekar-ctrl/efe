/*
 * Sprint D26 (Φ-B) — HAND-CODED PLACEHOLDER stubs for protoc-gen-java
 * output of `protos/efe/v1/types.proto`.
 *
 * In production, `mvn generate-sources` invokes `protoc + protoc-gen-grpc-java`
 * which produces this file under `target/generated-sources/protobuf/java/efe/v1/Types.java`.
 *
 * This hand-coded version pins the wire-contract subset the SDK actually
 * uses (Verdict enum + minimal message accessors) so the SDK compiles
 * end-to-end without `protoc` on the dev machine. CI runs `mvn generate-sources`
 * first to replace this file with the real generated code.
 *
 * Pin tested by `test_d26_java_sdk_protos_in_sync` (Python).
 */
package efe.v1;

import java.io.Serializable;

/**
 * Placeholder for `efe.v1.Types` outer class produced by protoc.
 *
 * <p>**Audit fix M11-A2** — message classes implement {@link Serializable}
 * so the placeholder marshaller in {@code SwarmServiceGrpc} (Java
 * Serialization round-trip) works at runtime. Real protoc-gen-grpc-java
 * uses ProtoLiteUtils which speaks proto binary directly.
 */
public final class Types {

    private Types() {}

    /** Verdict enum mirrors `efe.v1.Verdict` proto enum. */
    public enum Verdict {
        VERDICT_UNSPECIFIED(0),
        VERDICT_CONTINUE(1),
        VERDICT_CONVERGED(2),
        VERDICT_DIVERGENCE(3),
        VERDICT_MODE_COLLAPSE(4);

        private final int number;

        Verdict(int number) { this.number = number; }

        public int getNumber() { return number; }

        public static Verdict forNumber(int n) {
            return switch (n) {
                case 0 -> VERDICT_UNSPECIFIED;
                case 1 -> VERDICT_CONTINUE;
                case 2 -> VERDICT_CONVERGED;
                case 3 -> VERDICT_DIVERGENCE;
                case 4 -> VERDICT_MODE_COLLAPSE;
                default -> VERDICT_UNSPECIFIED;
            };
        }
    }

    /** ConeRef placeholder — minimal getter surface. */
    public static final class ConeRef implements Serializable {
        private static final long serialVersionUID = 1L;

        private final String tenant;
        private final String task;
        private final int version;

        private ConeRef(Builder b) {
            this.tenant = b.tenant;
            this.task = b.task;
            this.version = b.version;
        }

        public String getTenant() { return tenant; }
        public String getTask() { return task; }
        public int getVersion() { return version; }

        public static Builder newBuilder() { return new Builder(); }

        public static final class Builder {
            private String tenant = "";
            private String task = "";
            private int version = 0;

            public Builder setTenant(String t) { this.tenant = t; return this; }
            public Builder setTask(String t) { this.task = t; return this; }
            public Builder setVersion(int v) { this.version = v; return this; }
            public ConeRef build() { return new ConeRef(this); }
        }
    }

    /** DecodedOutput placeholder — `output.text` accessor canonical. */
    public static final class DecodedOutput implements Serializable {
        private static final long serialVersionUID = 1L;

        private final String text;

        private DecodedOutput(Builder b) { this.text = b.text; }

        public String getText() { return text; }

        public static Builder newBuilder() { return new Builder(); }

        public static final class Builder {
            private String text = "";

            public Builder setText(String t) { this.text = t; return this; }
            public DecodedOutput build() { return new DecodedOutput(this); }
        }
    }
}
