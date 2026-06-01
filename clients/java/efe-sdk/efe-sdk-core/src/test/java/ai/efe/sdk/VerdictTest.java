package ai.efe.sdk;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.CsvSource;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/** Sprint D26 — unit tests : Verdict record + helpers. */
class VerdictTest {

    private Verdict verdictWithZone(String zone, boolean reject) {
        return new Verdict("foo", 0.9, reject, 4, 0.1, "cat", zone, "swarm", "", 1L);
    }

    @Test
    void compactConstructorRejectsNullLabel() {
        assertThatThrownBy(() -> new Verdict(null, 0.5, false, 0, 0.0, "c", "warn", "s", "", 0))
            .isInstanceOf(NullPointerException.class);
    }

    @Test
    void compactConstructorRejectsConfidenceOutOfRange() {
        assertThatThrownBy(() -> new Verdict("x", 1.5, false, 0, 0.0, "c", "warn", "s", "", 0))
            .isInstanceOf(IllegalArgumentException.class)
            .hasMessageContaining("confidence");
    }

    @Test
    void isTrustedTrueOnlyForNonRejectedTrustedZone() {
        assertThat(verdictWithZone("trusted", false).isTrusted()).isTrue();
        assertThat(verdictWithZone("warn", false).isTrusted()).isFalse();
        assertThat(verdictWithZone("trusted", true).isTrusted()).isFalse();
    }

    @Test
    void isWarnTrueOnlyForNonRejectedWarnZone() {
        assertThat(verdictWithZone("warn", false).isWarn()).isTrue();
        assertThat(verdictWithZone("trusted", false).isWarn()).isFalse();
    }

    @Test
    void isRejectTrueForExplicitRejectOrZoneReject() {
        assertThat(verdictWithZone("reject", false).isReject()).isTrue();
        assertThat(verdictWithZone("warn", true).isReject()).isTrue();
        assertThat(verdictWithZone("trusted", false).isReject()).isFalse();
    }

    @ParameterizedTest
    @CsvSource({
        "0.99, trusted",
        "0.80, trusted",
        "0.79, warn",
        "0.50, warn",
        "0.49, reject",
        "0.00, reject"
    })
    void deriveZoneCanonicalThresholds(double confidence, String expectedZone) {
        assertThat(Verdict.deriveZone(confidence)).isEqualTo(expectedZone);
    }

    @Test
    void toStringDoesNotLeakAuth() {
        Verdict v = verdictWithZone("trusted", false);
        String s = v.toString();
        assertThat(s)
            .doesNotContain("Bearer")
            .doesNotContain("authorization")
            .contains("foo")
            .contains("trusted");
    }
}
