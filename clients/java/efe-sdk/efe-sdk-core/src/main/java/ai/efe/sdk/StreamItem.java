/*
 * Sprint D26 (Φ-B) — StreamItem record (used by EfeClient.classifyStream).
 */
package ai.efe.sdk;

import org.jspecify.annotations.Nullable;

/**
 * One item in the stream returned by {@link EfeClient#classifyStream}.
 *
 * <p>{@code index} preserves the input order even when the server returns
 * out-of-order. {@code error} is non-null iff this particular item failed ;
 * the stream as a whole continues unless ctx is cancelled.
 *
 * <p>Either {@code verdict} or {@code error} is non-null (never both
 * non-null, never both null).
 *
 * @param index position of the source text in the input iterable.
 * @param verdict verdict (non-null on success ; null on error).
 * @param error error (non-null on failure ; null on success).
 *
 * @since 0.4.0
 */
public record StreamItem(int index, @Nullable Verdict verdict, @Nullable Throwable error) {

    public StreamItem {
        if ((verdict == null) == (error == null)) {
            throw new IllegalArgumentException(
                "exactly one of verdict / error must be non-null");
        }
    }

    /** Returns true iff this item carries a successful verdict. */
    public boolean isSuccess() {
        return verdict != null;
    }
}
