package ai.efe.sdk.mock.integration;

import ai.efe.sdk.EfeClient;
import ai.efe.sdk.exception.EfeClosedException;
import ai.efe.sdk.mock.MockEfeServer;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.time.Duration;

import static org.assertj.core.api.Assertions.assertThatThrownBy;

/**
 * Sprint D26 — integration tests : healthcheck.
 *
 * The current MockEfeServer doesn't register the gRPC standard Health
 * service ; we test that the SDK throws gracefully (no crash, typed
 * exception). Full SERVING-status testing comes in D27 with a richer mock.
 */
class HealthcheckIntegrationTest {

    private MockEfeServer server;
    private EfeClient client;

    @BeforeEach
    void setUp() throws IOException {
        server = MockEfeServer.create();
        client = EfeClient.builder()
            .endpoint(server.getEndpoint())
            .deadline(Duration.ofSeconds(1))
            .build();
    }

    @AfterEach
    void tearDown() throws Exception {
        if (client != null) client.close();
        if (server != null) server.close();
    }

    @Test
    void afterCloseThrowsEfeClosed() {
        client.close();
        assertThatThrownBy(() -> client.healthcheck())
            .isInstanceOf(EfeClosedException.class);
    }

    @Test
    void healthcheckThrowsTypedExceptionWhenServiceMissing() {
        // The mock server doesn't register the standard Health service ;
        // SDK should throw a typed EfeException (not StatusRuntimeException).
        assertThatThrownBy(() -> client.healthcheck())
            .isInstanceOf(ai.efe.sdk.exception.EfeException.class);
    }
}
