package ai.efe.sdk.auth;

import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/** Sprint D26 — unit tests : JwtCallCredentials. */
class JwtCallCredentialsTest {

    @Test
    void emptyTokenRejected() {
        assertThatThrownBy(() -> new JwtCallCredentials(""))
            .isInstanceOf(IllegalArgumentException.class);
        assertThatThrownBy(() -> new JwtCallCredentials("   "))
            .isInstanceOf(IllegalArgumentException.class);
    }

    @Test
    void nullTokenRejected() {
        assertThatThrownBy(() -> new JwtCallCredentials(null))
            .isInstanceOf(NullPointerException.class);
    }

    @Test
    void toStringRedactsToken() {
        JwtCallCredentials creds = new JwtCallCredentials("super-secret-12345");
        String s = creds.toString();
        assertThat(s)
            .doesNotContain("super-secret-12345")
            .contains("redacted");
    }

    @Test
    void jwtAuthRecordRejectsBlank() {
        assertThatThrownBy(() -> new JwtAuth(""))
            .isInstanceOf(IllegalArgumentException.class);
        assertThatThrownBy(() -> new JwtAuth("   "))
            .isInstanceOf(IllegalArgumentException.class);
    }

    @Test
    void jwtAuthFactoryWorks() {
        JwtAuth auth = JwtAuth.of("eyJhbGc...");
        assertThat(auth.token()).isEqualTo("eyJhbGc...");
    }

    @Test
    void jwtAuthToStringRedacts() {
        JwtAuth auth = JwtAuth.of("super-secret-xyz");
        String s = auth.toString();
        assertThat(s).doesNotContain("super-secret-xyz");
    }
}
