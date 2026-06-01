package ai.efe.sdk.mock.integration;

import ai.efe.sdk.EfeClient;
import ai.efe.sdk.Verdict;
import ai.efe.sdk.exception.EfeClosedException;
import ai.efe.sdk.exception.EfeInvalidArgumentException;
import ai.efe.sdk.mock.MockEfeServer;

import efe.v1.Efe.ExecuteResponse;
import efe.v1.Types;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import java.io.IOException;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/** Sprint D26 — integration tests : EfeClient.classify vs in-process mock server. */
class ClassifyIntegrationTest {

    private MockEfeServer server;
    private EfeClient client;

    @BeforeEach
    void setUp() throws IOException {
        server = MockEfeServer.create();
        client = EfeClient.builder()
            .endpoint(server.getEndpoint())
            .build();
    }

    @AfterEach
    void tearDown() throws Exception {
        if (client != null) client.close();
        if (server != null) server.close();
    }

    @Test
    @DisplayName("classify returns server-side verdict with derived zone")
    void happyPath() {
        server.onClassify(req -> ExecuteResponse.newBuilder()
            .setOutput(Types.DecodedOutput.newBuilder().setText("billing").build())
            .setConfidence(0.91f)
            .setVerdict(Types.Verdict.VERDICT_CONVERGED)
            .setSwarmId("swarm-abc")
            .setRequestId("req-123")
            .setRoundsExecuted(4)
            .build());

        Verdict v = client.classify("I want a refund");

        assertThat(v.label()).isEqualTo("billing");
        assertThat(v.confidence()).isCloseTo(0.91, org.assertj.core.data.Offset.offset(0.001));
        assertThat(v.zone()).isEqualTo("trusted");
        assertThat(v.reject()).isFalse();
        assertThat(v.swarmId()).isEqualTo("swarm-abc");
        assertThat(v.requestId()).isEqualTo("req-123");
    }

    @Test
    @DisplayName("classify throws EfeInvalidArgument on empty text")
    void emptyTextRejected() {
        assertThatThrownBy(() -> client.classify("  "))
            .isInstanceOf(EfeInvalidArgumentException.class);
    }

    @Test
    @DisplayName("classify throws EfeClosed after close()")
    void afterCloseThrowsEfeClosed() {
        client.close();
        assertThatThrownBy(() -> client.classify("foo"))
            .isInstanceOf(EfeClosedException.class);
    }

    @Test
    @DisplayName("VERDICT_DIVERGENCE marks reject=true")
    void divergenceSetsReject() {
        server.onClassify(req -> ExecuteResponse.newBuilder()
            .setOutput(Types.DecodedOutput.newBuilder().setText("hostile").build())
            .setConfidence(0.5f)
            .setVerdict(Types.Verdict.VERDICT_DIVERGENCE)
            .build());

        Verdict v = client.classify("evil prompt");
        assertThat(v.reject()).isTrue();
        assertThat(v.zone()).isEqualTo("reject");
    }

    @Test
    @DisplayName("VERDICT_MODE_COLLAPSE marks reject=true")
    void modeCollapseSetsReject() {
        server.onClassify(req -> ExecuteResponse.newBuilder()
            .setOutput(Types.DecodedOutput.newBuilder().setText("looped").build())
            .setConfidence(0.5f)
            .setVerdict(Types.Verdict.VERDICT_MODE_COLLAPSE)
            .build());

        Verdict v = client.classify("looped prompt");
        assertThat(v.reject()).isTrue();
    }

    @Test
    @DisplayName("VERDICT_CONTINUE surfaces as warn")
    void continueAsWarn() {
        server.onClassify(req -> ExecuteResponse.newBuilder()
            .setOutput(Types.DecodedOutput.newBuilder().setText("inconclusive").build())
            .setConfidence(0.6f)
            .setVerdict(Types.Verdict.VERDICT_CONTINUE)
            .build());

        Verdict v = client.classify("inconclusive prompt");
        assertThat(v.zone()).isEqualTo("warn");
    }

    @Test
    @DisplayName("requestId field preserved from server")
    void requestIdPreserved() {
        server.onClassify(req -> ExecuteResponse.newBuilder()
            .setOutput(Types.DecodedOutput.newBuilder().setText("ok").build())
            .setConfidence(0.9f)
            .setVerdict(Types.Verdict.VERDICT_CONVERGED)
            .setRequestId("req-XYZ-123")
            .build());

        Verdict v = client.classify("test");
        assertThat(v.requestId()).isEqualTo("req-XYZ-123");
    }
}
