package ai.efe.sdk.mock.integration;

import ai.efe.sdk.EfeClient;
import ai.efe.sdk.StreamItem;
import ai.efe.sdk.mock.MockEfeServer;

import efe.v1.Efe.ExecuteResponse;
import efe.v1.Types;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.IntStream;

import static org.assertj.core.api.Assertions.assertThat;

/**
 * Sprint D26 — integration test : classifyStream order preservation.
 *
 * Pin test for the dispatcher pattern : even when results arrive
 * out-of-order on the output queue, each StreamItem.index reflects
 * the input position. Mirrors Go SDK M9-A3 + TS SDK M10-A5/A6 fixes.
 */
class StreamIntegrationTest {

    private MockEfeServer server;
    private EfeClient client;

    @BeforeEach
    void setUp() throws IOException {
        server = MockEfeServer.create();
        client = EfeClient.builder().endpoint(server.getEndpoint()).build();
        // Echo the prompt back as the label so we can reconstruct (idx → text).
        server.onClassify(req -> ExecuteResponse.newBuilder()
            .setOutput(Types.DecodedOutput.newBuilder().setText(req.getRawPrompt()).build())
            .setConfidence(0.9f)
            .setVerdict(Types.Verdict.VERDICT_CONVERGED)
            .setRoundsExecuted(1)
            .build());
    }

    @AfterEach
    void tearDown() throws Exception {
        if (client != null) client.close();
        if (server != null) server.close();
    }

    @Test
    @DisplayName("classifyStream preserves input-order via index for 50 inputs")
    void preservesInputOrder() {
        final int N = 50;
        List<String> inputs = IntStream.range(0, N)
            .mapToObj(i -> String.format("text-%02d", i))
            .toList();

        List<StreamItem> items = client.classifyStream(inputs).toList();

        // Same number of items as inputs.
        assertThat(items).hasSize(N);

        // No duplicates + every index 0..N-1 covered.
        boolean[] seen = new boolean[N];
        String[] labelByIndex = new String[N];
        for (StreamItem item : items) {
            int idx = item.index();
            assertThat(idx).isBetween(0, N - 1);
            assertThat(seen[idx]).as("no duplicate for index %d", idx).isFalse();
            seen[idx] = true;
            assertThat(item.error()).isNull();
            assertThat(item.verdict()).isNotNull();
            labelByIndex[idx] = item.verdict().label();
        }
        for (int i = 0; i < N; i++) {
            assertThat(seen[i]).as("missing index %d", i).isTrue();
            // Each label echoes the corresponding input.
            assertThat(labelByIndex[i]).isEqualTo(inputs.get(i));
        }
    }

    @Test
    @DisplayName("classifyStream handles empty input")
    void emptyInput() {
        List<StreamItem> items = client.classifyStream(new ArrayList<>()).toList();
        assertThat(items).isEmpty();
    }
}
