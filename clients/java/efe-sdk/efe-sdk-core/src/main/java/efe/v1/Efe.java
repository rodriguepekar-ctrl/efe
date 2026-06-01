/*
 * Sprint D26 (Φ-B) — HAND-CODED PLACEHOLDER for protoc-gen-java output
 * of `protos/efe/v1/efe.proto` (ExecuteRequest + ExecuteResponse messages).
 *
 * Cf. Types.java header for the regeneration story.
 *
 * **Audit fix M11-A2** — messages implement Serializable so the
 * placeholder marshaller round-trip works at runtime.
 */
package efe.v1;

import java.io.Serializable;

/** Placeholder for `efe.v1.Efe` outer class produced by protoc. */
public final class Efe {

    private Efe() {}

    /** ExecuteRequest mirrors `efe.v1.ExecuteRequest`. */
    public static final class ExecuteRequest implements Serializable {
        private static final long serialVersionUID = 1L;

        private final String requestId;
        private final String rawPrompt;

        private ExecuteRequest(Builder b) {
            this.requestId = b.requestId;
            this.rawPrompt = b.rawPrompt;
        }

        public String getRequestId() { return requestId; }
        public String getRawPrompt() { return rawPrompt; }

        public static Builder newBuilder() { return new Builder(); }

        public static final class Builder {
            private String requestId = "";
            private String rawPrompt = "";

            public Builder setRequestId(String id) { this.requestId = id; return this; }
            public Builder setRawPrompt(String p) { this.rawPrompt = p; return this; }
            public ExecuteRequest build() { return new ExecuteRequest(this); }
        }
    }

    /** ExecuteResponse mirrors `efe.v1.ExecuteResponse`. */
    public static final class ExecuteResponse implements Serializable {
        private static final long serialVersionUID = 1L;

        private final String swarmId;
        private final String requestId;
        private final Types.DecodedOutput output;
        private final float confidence;
        private final Types.Verdict verdict;
        private final int roundsExecuted;
        private final float totalMs;
        private final float alignmentDelta;

        private ExecuteResponse(Builder b) {
            this.swarmId = b.swarmId;
            this.requestId = b.requestId;
            this.output = b.output;
            this.confidence = b.confidence;
            this.verdict = b.verdict;
            this.roundsExecuted = b.roundsExecuted;
            this.totalMs = b.totalMs;
            this.alignmentDelta = b.alignmentDelta;
        }

        public String getSwarmId() { return swarmId; }
        public String getRequestId() { return requestId; }
        public Types.DecodedOutput getOutput() { return output; }
        public boolean hasOutput() { return output != null; }
        public float getConfidence() { return confidence; }
        public Types.Verdict getVerdict() { return verdict; }
        public int getRoundsExecuted() { return roundsExecuted; }
        public float getTotalMs() { return totalMs; }
        public float getAlignmentDelta() { return alignmentDelta; }

        public static Builder newBuilder() { return new Builder(); }

        public static final class Builder {
            private String swarmId = "";
            private String requestId = "";
            private Types.DecodedOutput output;
            private float confidence = 0f;
            private Types.Verdict verdict = Types.Verdict.VERDICT_UNSPECIFIED;
            private int roundsExecuted = 0;
            private float totalMs = 0f;
            private float alignmentDelta = 0f;

            public Builder setSwarmId(String id) { this.swarmId = id; return this; }
            public Builder setRequestId(String id) { this.requestId = id; return this; }
            public Builder setOutput(Types.DecodedOutput o) { this.output = o; return this; }
            public Builder setOutput(Types.DecodedOutput.Builder b) { this.output = b.build(); return this; }
            public Builder setConfidence(float c) { this.confidence = c; return this; }
            public Builder setVerdict(Types.Verdict v) { this.verdict = v; return this; }
            public Builder setRoundsExecuted(int r) { this.roundsExecuted = r; return this; }
            public Builder setTotalMs(float t) { this.totalMs = t; return this; }
            public Builder setAlignmentDelta(float d) { this.alignmentDelta = d; return this; }
            public ExecuteResponse build() { return new ExecuteResponse(this); }
        }
    }
}
