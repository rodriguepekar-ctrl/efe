/*
 * Sprint D26 (Φ-B) — gRPC Status → SDK sentinel mapping.
 *
 * SOLE conversion path between gRPC errors and SDK-typed exceptions.
 * The SDK NEVER surfaces raw `StatusRuntimeException` to consumers.
 *
 * **Audit fix M11-A5** — pre-fix had verbose ternary boilerplate
 * `cause != null ? new X(msg, cause) : new X(msg)` for every status
 * code (44 lines). Sentinel constructors accept `(message, null)`
 * cleanly via the EfeException base passing `null` cause to
 * RuntimeException — so we can ALWAYS use the (msg, cause) form,
 * simplifying the switch from 44 to 12 lines.
 */
package ai.efe.sdk.internal;

import ai.efe.sdk.exception.*;

import io.grpc.Status;
import io.grpc.StatusRuntimeException;

/**
 * Translate a gRPC {@link Status} to a typed SDK exception.
 *
 * @since 0.4.0
 */
public final class ErrorMapper {

    private ErrorMapper() {}

    /**
     * Map a {@link StatusRuntimeException} to the corresponding SDK
     * sentinel ; preserves the original exception as `cause`.
     */
    public static EfeException fromGrpcStatus(StatusRuntimeException e) {
        return mapWithCause(e.getStatus(), e);
    }

    /** Map a {@link Status} to the corresponding SDK sentinel (no cause). */
    public static EfeException fromGrpcStatus(Status status) {
        return mapWithCause(status, null);
    }

    private static EfeException mapWithCause(Status status, Throwable cause) {
        String description = status.getDescription();
        if (description == null) description = "(no description)";
        return switch (status.getCode()) {
            case UNAUTHENTICATED -> new EfeUnauthenticatedException(description, cause);
            case PERMISSION_DENIED -> new EfePermissionDeniedException(description, cause);
            case NOT_FOUND -> new EfeCatalogNotFoundException(description, cause);
            case FAILED_PRECONDITION -> new EfeCatalogInvalidException(description, cause);
            case DEADLINE_EXCEEDED, CANCELLED -> new EfeTimeoutException(description, cause);
            case UNAVAILABLE -> new EfeUnavailableException(description, cause);
            case RESOURCE_EXHAUSTED -> new EfeRateLimitedException(description, cause);
            case INVALID_ARGUMENT, OUT_OF_RANGE -> new EfeInvalidArgumentException(description, cause);
            case INTERNAL, UNKNOWN, DATA_LOSS -> new EfeInternalException(description, cause);
            default -> new EfeInternalException(description, cause);
        };
    }
}
