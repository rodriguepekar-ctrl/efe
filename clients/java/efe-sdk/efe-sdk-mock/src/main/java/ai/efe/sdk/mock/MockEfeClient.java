/*
 * Sprint D26 (Φ-B) — MockEfeClient : pure Java fixture, no gRPC.
 *
 * Use this for unit tests that :
 *   - inject canned verdicts ;
 *   - assert the texts your handler classified ;
 *   - simulate errors via setError().
 *
 * For tests that exercise the FULL gRPC stack, use MockEfeServer instead.
 */
package ai.efe.sdk.mock;

import ai.efe.sdk.Verdict;
import ai.efe.sdk.exception.EfeClosedException;

import org.jspecify.annotations.Nullable;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.CompletableFuture;

/**
 * Drop-in replacement for {@code EfeClient} in unit tests.
 *
 * <p>NOT a true subclass — provides the same public methods that
 * consumer code typically depends on (classify / classifyAsync /
 * healthcheck / close). Use it as a stand-in for tests where you
 * don't need the actual gRPC stack.
 *
 * @example
 * <pre>{@code
 * var mock = new MockEfeClient();
 * mock.setVerdict(VerdictFactory.builder().label("fraud").build());
 *
 * myHandler.process(mock);
 *
 * assertThat(mock.getCalls()).containsExactly("input text");
 * }</pre>
 *
 * @since 0.4.0
 */
public final class MockEfeClient implements AutoCloseable {

    private @Nullable Verdict verdict;
    private @Nullable Throwable error;
    private final List<String> calls = Collections.synchronizedList(new ArrayList<>());
    private volatile boolean closed = false;

    /** Set the verdict returned by subsequent classify calls. */
    public void setVerdict(Verdict v) {
        this.verdict = v;
    }

    /** Cause subsequent classify/healthcheck calls to throw {@code err}. Pass null to clear. */
    public void setError(@Nullable Throwable err) {
        this.error = err;
    }

    /** Snapshot of the texts {@code classify} was called with, in invocation order. */
    public List<String> getCalls() {
        return List.copyOf(calls);
    }

    /** Number of recorded calls. */
    public int getCallCount() {
        return calls.size();
    }

    /** Clear all recorded state (calls + verdict + error + closed flag). */
    public void reset() {
        verdict = null;
        error = null;
        calls.clear();
        closed = false;
    }

    /** Mirror of {@code EfeClient.classify}. */
    public Verdict classify(String text) {
        if (closed) throw new EfeClosedException("mock client is closed");
        calls.add(text);
        if (error != null) {
            if (error instanceof RuntimeException re) throw re;
            throw new RuntimeException(error);
        }
        return verdict != null ? verdict : VerdictFactory.newVerdict();
    }

    /** Mirror of {@code EfeClient.classifyAsync}. */
    public CompletableFuture<Verdict> classifyAsync(String text) {
        try {
            return CompletableFuture.completedFuture(classify(text));
        } catch (Throwable t) {
            return CompletableFuture.failedFuture(t);
        }
    }

    /**
     * Mirror of {@code EfeClient.healthcheck}.
     *
     * <p>Symmetric with classify — any non-null setError() fails
     * healthcheck (post M9-A4 Go-SDK audit cohérence).
     */
    public void healthcheck() {
        if (closed) throw new EfeClosedException("mock client is closed");
        if (error != null) {
            if (error instanceof RuntimeException re) throw re;
            throw new RuntimeException(error);
        }
    }

    @Override
    public void close() {
        closed = true;
    }

    public boolean isClosed() {
        return closed;
    }
}
