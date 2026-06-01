/*
 * Sprint D26 (Φ-B) — classifyStream impl with strict input-order indices.
 *
 * Single-dispatcher pattern (mirrors Go SDK M9-A3 audit fix + TS SDK
 * M10-A5/A6 fix) : a single goroutine reads `texts`, assigns monotonic
 * indices, fans out to a worker pool ; results are collected into a
 * Stream that yields in non-deterministic order BUT each StreamItem.index
 * matches the input position.
 */
package ai.efe.sdk.internal;

import ai.efe.sdk.EfeClient;
import ai.efe.sdk.StreamItem;

import java.util.ArrayList;
import java.util.List;
import java.util.Spliterator;
import java.util.Spliterators;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.SynchronousQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import java.util.stream.Stream;
import java.util.stream.StreamSupport;

/**
 * Worker-pool batch classify with strict input-order indices.
 *
 * <p>Architecture :
 * <ol>
 *   <li>DISPATCHER : single virtual thread reads {@code texts}, assigns
 *       monotonic indices, pushes (index, text) to {@code fanoutQueue}.</li>
 *   <li>WORKERS : N virtual threads (default 8) consume from
 *       {@code fanoutQueue}, call {@code client.classify}, push results
 *       to {@code outputQueue}.</li>
 *   <li>SHUTDOWN orchestrator : waits for dispatcher + all workers to
 *       finish, then signals "done" via a sentinel poison-pill in
 *       {@code outputQueue}.</li>
 *   <li>CONSUMER : the returned {@link Stream} polls {@code outputQueue}
 *       until the poison-pill arrives.</li>
 * </ol>
 */
public final class StreamImpl {

    private StreamImpl() {}

    /** Sentinel value pushed to outputQueue to signal "no more items". */
    private static final StreamItem POISON_PILL = new StreamItem(
        Integer.MIN_VALUE, null, new RuntimeException("poison"));

    private static final int DEFAULT_CONCURRENCY = 8;

    public static Stream<StreamItem> run(EfeClient client, Iterable<String> texts, int concurrency) {
        if (concurrency < 1) concurrency = DEFAULT_CONCURRENCY;

        BlockingQueue<IndexedText> fanout = new LinkedBlockingQueue<>(concurrency * 2);
        BlockingQueue<StreamItem> output = new LinkedBlockingQueue<>();
        AtomicReference<Throwable> dispatcherError = new AtomicReference<>();
        IndexedText END_OF_INPUT = new IndexedText(-1, "");

        // **Audit fix M11-A1** — pre-fix used `Executors.newVirtualThreadPerTaskExecutor()`
        // which requires Java 21+. The project targets Java 17 LTS.
        // Replaced with a cached thread pool of DAEMON threads sized to
        // (1 dispatcher + concurrency workers + 1 orchestrator).
        // Daemon flag ensures the JVM can exit even if a worker hangs.
        final AtomicInteger threadCounter = new AtomicInteger();
        ExecutorService executor = new ThreadPoolExecutor(
            0,
            concurrency + 2,             // dispatcher + workers + orchestrator
            60L, TimeUnit.SECONDS,
            new SynchronousQueue<>(),
            r -> {
                Thread t = new Thread(r, "efe-sdk-stream-" + threadCounter.incrementAndGet());
                t.setDaemon(true);
                return t;
            }
        );

        // Dispatcher.
        executor.submit(() -> {
            int idx = 0;
            try {
                for (String text : texts) {
                    fanout.put(new IndexedText(idx++, text));
                }
            } catch (InterruptedException ie) {
                Thread.currentThread().interrupt();
                dispatcherError.set(ie);
            } catch (Throwable t) {
                dispatcherError.set(t);
            } finally {
                // Signal end-of-input to ALL workers via concurrency poison-pills.
                for (int i = 0; i < concurrency; i++) {
                    try {
                        fanout.put(END_OF_INPUT);
                    } catch (InterruptedException ie) {
                        Thread.currentThread().interrupt();
                        break;
                    }
                }
            }
        });

        // Workers.
        List<java.util.concurrent.Future<?>> workers = new ArrayList<>(concurrency);
        for (int w = 0; w < concurrency; w++) {
            workers.add(executor.submit(() -> {
                while (true) {
                    IndexedText item;
                    try {
                        item = fanout.take();
                    } catch (InterruptedException ie) {
                        Thread.currentThread().interrupt();
                        return;
                    }
                    if (item == END_OF_INPUT) return;
                    StreamItem result;
                    try {
                        result = new StreamItem(item.index(), client.classify(item.text()), null);
                    } catch (Throwable t) {
                        result = new StreamItem(item.index(), null, t);
                    }
                    try {
                        output.put(result);
                    } catch (InterruptedException ie) {
                        Thread.currentThread().interrupt();
                        return;
                    }
                }
            }));
        }

        // Orchestrator : when all workers finish, push POISON_PILL.
        executor.submit(() -> {
            try {
                for (var f : workers) {
                    f.get();
                }
            } catch (Exception e) {
                output.offer(new StreamItem(-1, null, e));
            } finally {
                output.offer(POISON_PILL);
                executor.shutdown();
            }
        });

        // Build a Stream that polls outputQueue until poison-pill.
        return StreamSupport.stream(Spliterators.spliteratorUnknownSize(
            new java.util.Iterator<StreamItem>() {
                StreamItem next;
                boolean done;

                @Override
                public boolean hasNext() {
                    if (done) return false;
                    if (next != null) return true;
                    try {
                        StreamItem item = output.take();
                        if (item == POISON_PILL) {
                            done = true;
                            Throwable derr = dispatcherError.get();
                            if (derr != null) {
                                throw new RuntimeException("classifyStream dispatcher failed", derr);
                            }
                            return false;
                        }
                        next = item;
                        return true;
                    } catch (InterruptedException ie) {
                        Thread.currentThread().interrupt();
                        done = true;
                        return false;
                    }
                }

                @Override
                public StreamItem next() {
                    if (!hasNext()) throw new java.util.NoSuchElementException();
                    StreamItem out = next;
                    next = null;
                    return out;
                }
            },
            Spliterator.ORDERED | Spliterator.NONNULL
        ), false);
    }

    /** Indexed text used by the dispatcher pattern. */
    record IndexedText(int index, String text) {}
}
