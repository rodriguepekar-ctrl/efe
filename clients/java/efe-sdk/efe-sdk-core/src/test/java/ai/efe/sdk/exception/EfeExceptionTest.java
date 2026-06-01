package ai.efe.sdk.exception;

import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThat;

/** Sprint D26 — unit tests : exception hierarchy + isRetryable + errorCode. */
class EfeExceptionTest {

    @Test
    void allTenSentinelsExtendEfeException() {
        EfeException[] sentinels = {
            new EfeUnauthenticatedException(),
            new EfePermissionDeniedException(),
            new EfeCatalogNotFoundException(),
            new EfeCatalogInvalidException(),
            new EfeTimeoutException(),
            new EfeUnavailableException(),
            new EfeRateLimitedException(),
            new EfeInvalidArgumentException(),
            new EfeInternalException(),
            new EfeClosedException(),
        };
        assertThat(sentinels).hasSize(10);
        for (EfeException e : sentinels) {
            assertThat(e).isInstanceOf(EfeException.class);
            assertThat(e).isInstanceOf(RuntimeException.class);
        }
    }

    @Test
    void everySentinelHasDistinctCode() {
        String[] codes = {
            new EfeUnauthenticatedException().code(),
            new EfePermissionDeniedException().code(),
            new EfeCatalogNotFoundException().code(),
            new EfeCatalogInvalidException().code(),
            new EfeTimeoutException().code(),
            new EfeUnavailableException().code(),
            new EfeRateLimitedException().code(),
            new EfeInvalidArgumentException().code(),
            new EfeInternalException().code(),
            new EfeClosedException().code(),
        };
        // Distinct.
        long distinct = java.util.Arrays.stream(codes).distinct().count();
        assertThat(distinct).isEqualTo(10);
    }

    @Test
    void canonicalCodeStringsMatchCrossLanguage() {
        // These strings MUST match Python M8 + Go M9 + TS M10 SDKs.
        assertThat(new EfeUnauthenticatedException().code()).isEqualTo("unauthenticated");
        assertThat(new EfePermissionDeniedException().code()).isEqualTo("permission_denied");
        assertThat(new EfeCatalogNotFoundException().code()).isEqualTo("catalog_not_found");
        assertThat(new EfeCatalogInvalidException().code()).isEqualTo("catalog_invalid");
        assertThat(new EfeTimeoutException().code()).isEqualTo("timeout");
        assertThat(new EfeUnavailableException().code()).isEqualTo("unavailable");
        assertThat(new EfeRateLimitedException().code()).isEqualTo("rate_limited");
        assertThat(new EfeInvalidArgumentException().code()).isEqualTo("invalid_argument");
        assertThat(new EfeInternalException().code()).isEqualTo("internal");
        assertThat(new EfeClosedException().code()).isEqualTo("closed");
    }

    @Test
    void isRetryableTrueForRetryableSubset() {
        assertThat(EfeException.isRetryable(new EfeUnavailableException())).isTrue();
        assertThat(EfeException.isRetryable(new EfeTimeoutException())).isTrue();
        assertThat(EfeException.isRetryable(new EfeRateLimitedException())).isTrue();
    }

    @Test
    void isRetryableFalseForNonRetryable() {
        assertThat(EfeException.isRetryable(new EfeUnauthenticatedException())).isFalse();
        assertThat(EfeException.isRetryable(new EfePermissionDeniedException())).isFalse();
        assertThat(EfeException.isRetryable(new EfeCatalogNotFoundException())).isFalse();
        assertThat(EfeException.isRetryable(new EfeCatalogInvalidException())).isFalse();
        assertThat(EfeException.isRetryable(new EfeInvalidArgumentException())).isFalse();
        assertThat(EfeException.isRetryable(new EfeInternalException())).isFalse();
        assertThat(EfeException.isRetryable(new EfeClosedException())).isFalse();
    }

    @Test
    void isRetryableFalseForNonEfeException() {
        assertThat(EfeException.isRetryable(new RuntimeException("x"))).isFalse();
        assertThat(EfeException.isRetryable(null)).isFalse();
    }

    @Test
    void errorCodeReturnsCanonicalLabel() {
        assertThat(EfeException.errorCode(new EfeUnavailableException())).isEqualTo("unavailable");
        assertThat(EfeException.errorCode(new EfeTimeoutException())).isEqualTo("timeout");
    }

    @Test
    void errorCodeReturnsUnknownForNonEfeException() {
        assertThat(EfeException.errorCode(new RuntimeException("x"))).isEqualTo("unknown");
    }

    @Test
    void errorCodeReturnsEmptyForNull() {
        assertThat(EfeException.errorCode(null)).isEmpty();
    }

    @Test
    void exceptionPreservesCause() {
        Throwable root = new RuntimeException("root");
        EfeException wrapped = new EfeTimeoutException("wrapped", root);
        assertThat(wrapped.getCause()).isSameAs(root);
    }
}
