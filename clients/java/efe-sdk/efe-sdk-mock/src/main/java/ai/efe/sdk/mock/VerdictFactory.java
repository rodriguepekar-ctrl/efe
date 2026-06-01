/*
 * Sprint D26 (Φ-B) — VerdictFactory : canonical pre-baked Verdict for tests.
 *
 * Mirrors the Go SDK `mock.NewVerdict()` + TS SDK `newVerdict()` factory.
 */
package ai.efe.sdk.mock;

import ai.efe.sdk.Verdict;

/**
 * Builds canonical {@link Verdict} instances for unit tests.
 *
 * <p>Defaults :
 * <pre>
 * label="alpha", confidence=0.85, rounds=4, alignmentDelta=0.10,
 * catalog="test/cat@v1", zone="trusted", swarmId="mock-swarm",
 * elapsedMs=42, requestId="".
 * </pre>
 *
 * <p>The {@code zone} auto-derives from {@code confidence} ONLY IF
 * {@link Builder#zone} is NOT explicitly called AND {@code reject == false}.
 *
 * @since 0.4.0
 */
public final class VerdictFactory {

    private VerdictFactory() {}

    /** Returns a {@link Verdict} with canonical defaults. */
    public static Verdict newVerdict() {
        return builder().build();
    }

    /** Returns a fresh builder pre-populated with the canonical defaults. */
    public static Builder builder() {
        return new Builder();
    }

    /** Mutable builder for tests — produces an immutable {@link Verdict}. */
    public static final class Builder {
        private String label = "alpha";
        private double confidence = 0.85;
        private boolean reject = false;
        private int rounds = 4;
        private double alignmentDelta = 0.10;
        private String catalog = "test/cat@v1";
        private String zone = ""; // empty = auto-derive
        private String swarmId = "mock-swarm";
        private String requestId = "";
        private long elapsedMs = 42;
        private boolean zoneExplicit = false;

        public Builder label(String label) { this.label = label; return this; }
        public Builder confidence(double confidence) { this.confidence = confidence; return this; }
        public Builder reject(boolean reject) { this.reject = reject; return this; }
        public Builder rounds(int rounds) { this.rounds = rounds; return this; }
        public Builder alignmentDelta(double d) { this.alignmentDelta = d; return this; }
        public Builder catalog(String catalog) { this.catalog = catalog; return this; }
        public Builder zone(String zone) {
            this.zone = zone;
            this.zoneExplicit = true;
            return this;
        }
        public Builder swarmId(String swarmId) { this.swarmId = swarmId; return this; }
        public Builder requestId(String requestId) { this.requestId = requestId; return this; }
        public Builder elapsedMs(long elapsedMs) { this.elapsedMs = elapsedMs; return this; }

        public Verdict build() {
            String resolvedZone = zone;
            if (!zoneExplicit && !reject) {
                resolvedZone = Verdict.deriveZone(confidence);
            } else if (!zoneExplicit && reject) {
                resolvedZone = "reject";
            }
            return new Verdict(
                label, confidence, reject, rounds, alignmentDelta,
                catalog, resolvedZone, swarmId, requestId, elapsedMs
            );
        }
    }
}
