/*
 * Sprint D26 (Φ-B) — Public Verdict record.
 *
 * Cross-language pinned : 10 fields match Python M8 + Go M9 + TS M10.
 * camelCase Java names match the wire JSON tags by proto3 default.
 */
package ai.efe.sdk;

import java.util.Locale;
import java.util.Objects;

/**
 * Verdict — the public verdict type returned by {@link EfeClient#classify}.
 *
 * <p>Java 17 record — immutable by construction. Provides
 * {@link #isTrusted()} / {@link #isWarn()} / {@link #isReject()}
 * convenience predicates that match Go SDK + Python SDK helpers.
 *
 * <p>Cross-language pinned : the 10 fields match Python M8 + Go M9 +
 * TS M10 SDKs. Adding/renaming a field requires a coordinated bump
 * across the 4 SDKs + the wire contract proto.
 *
 * @param label predicted class name (e.g. "fraud", "billing"). Empty
 *              string when {@code reject == true}.
 * @param confidence in [0, 1]. Higher = more confident the label is right.
 * @param reject true iff the swarm decided the input is OUT-of-cone
 *               (alignment violation).
 * @param rounds number of fusion rounds the swarm executed before converging.
 * @param alignmentDelta distance from the alignment cone boundary, in [0, 1].
 *                       0 = perfectly aligned.
 * @param catalog canonical key of the catalog used (e.g. "acme/billing@v3").
 * @param zone "trusted" | "warn" | "reject" — derived from confidence
 *             + threshold policy of the catalog.
 * @param swarmId server-assigned id of the ephemeral swarm instance.
 * @param requestId gRPC request id (echoed by the server). Empty when
 *                  caller didn't supply one + server didn't generate.
 * @param elapsedMs SDK-side wall-clock latency from RPC start to RPC return.
 *
 * @since 0.4.0
 */
public record Verdict(
    String label,
    double confidence,
    boolean reject,
    int rounds,
    double alignmentDelta,
    String catalog,
    String zone,
    String swarmId,
    String requestId,
    long elapsedMs
) {

    /**
     * Compact constructor — validates non-null invariants + confidence range.
     */
    public Verdict {
        Objects.requireNonNull(label, "label");
        Objects.requireNonNull(catalog, "catalog");
        Objects.requireNonNull(zone, "zone");
        Objects.requireNonNull(swarmId, "swarmId");
        Objects.requireNonNull(requestId, "requestId");
        if (confidence < 0.0 || confidence > 1.0) {
            throw new IllegalArgumentException("confidence must be in [0,1], got " + confidence);
        }
    }

    /** Returns true iff the verdict is in the "trusted" zone. */
    public boolean isTrusted() {
        return !reject && "trusted".equals(zone);
    }

    /** Returns true iff the verdict is borderline (warn zone). */
    public boolean isWarn() {
        return !reject && "warn".equals(zone);
    }

    /** Returns true iff the verdict is rejected. */
    public boolean isReject() {
        return reject || "reject".equals(zone);
    }

    /**
     * Default zone derivation : trusted if confidence ≥ 0.80, warn if ≥ 0.50,
     * else reject. Matches Go SDK {@code deriveZone} + TS SDK
     * {@code deriveZone}.
     */
    public static String deriveZone(double confidence) {
        if (confidence >= 0.80) return "trusted";
        if (confidence >= 0.50) return "warn";
        return "reject";
    }

    /**
     * {@inheritDoc}
     *
     * <p>NEVER includes auth material (JWT token, mTLS bytes). Safe for logs.
     */
    @Override
    public String toString() {
        StringBuilder sb = new StringBuilder()
            .append("Verdict[label=").append(label)
            .append(" confidence=").append(String.format(Locale.ROOT, "%.3f", confidence))
            .append(" zone=").append(zone)
            .append(" rounds=").append(rounds);
        if (reject) sb.append(" reject=true");
        if (!requestId.isEmpty()) sb.append(" req=").append(requestId);
        sb.append(" elapsed=").append(elapsedMs).append("ms]");
        return sb.toString();
    }
}
