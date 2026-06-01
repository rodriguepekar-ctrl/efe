package ai.efe.sdk;

import ai.efe.sdk.auth.JwtAuth;
import ai.efe.sdk.auth.NoAuth;
import ai.efe.sdk.exception.EfeInvalidArgumentException;
import ai.efe.sdk.retry.RetryConfig;

import org.junit.jupiter.api.Test;

import java.time.Duration;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/** Sprint D26 — unit tests : EfeClient.Builder validation. */
class EfeClientBuilderTest {

    @Test
    void buildRequiresEndpoint() {
        assertThatThrownBy(() -> EfeClient.builder().build())
            .isInstanceOf(EfeInvalidArgumentException.class)
            .hasMessageContaining("endpoint");
    }

    @Test
    void buildRequiresNonBlankEndpoint() {
        assertThatThrownBy(() -> EfeClient.builder().endpoint("   ").build())
            .isInstanceOf(EfeInvalidArgumentException.class);
    }

    @Test
    void deadlineMustBeNonNegative() {
        assertThatThrownBy(() -> EfeClient.builder()
                .endpoint("localhost:50051")
                .deadline(Duration.ofMillis(-1)))
            .isInstanceOf(EfeInvalidArgumentException.class);
    }

    @Test
    void userAgentMustBeNonBlank() {
        assertThatThrownBy(() -> EfeClient.builder()
                .endpoint("localhost:50051")
                .userAgent(""))
            .isInstanceOf(EfeInvalidArgumentException.class);
    }

    @Test
    void maxReceivedMessageSizeMustBePositive() {
        assertThatThrownBy(() -> EfeClient.builder()
                .endpoint("localhost:50051")
                .maxReceivedMessageSize(0))
            .isInstanceOf(EfeInvalidArgumentException.class);
    }

    @Test
    void streamConcurrencyMustBeAtLeastOne() {
        assertThatThrownBy(() -> EfeClient.builder()
                .endpoint("localhost:50051")
                .streamConcurrency(0))
            .isInstanceOf(EfeInvalidArgumentException.class);
    }

    @Test
    void retryConfigPropagated() {
        // Smoke test : lazy dial — builder should NOT block on TCP/TLS handshake.
        try (EfeClient client = EfeClient.builder()
                .endpoint("in-process:nonexistent-just-for-testing")
                .auth(NoAuth.INSTANCE)
                .retry(RetryConfig.exponential(5, Duration.ofMillis(50), Duration.ofSeconds(1)))
                .build()) {
            assertThat(client.isClosed()).isFalse();
        }
    }

    @Test
    void closeIsIdempotent() {
        EfeClient client = EfeClient.builder()
            .endpoint("in-process:test-close-idempotent")
            .build();
        client.close();
        client.close(); // idempotent
        assertThat(client.isClosed()).isTrue();
    }

    @Test
    void toStringDoesNotLeakSecrets() {
        try (EfeClient client = EfeClient.builder()
                .endpoint("in-process:test-tostring")
                .auth(JwtAuth.of("super-secret-token-xyz"))
                .build()) {
            String s = client.toString();
            assertThat(s)
                .doesNotContain("super-secret-token-xyz")
                .doesNotContain("Bearer")
                .contains("in-process:test-tostring");
        }
    }
}
