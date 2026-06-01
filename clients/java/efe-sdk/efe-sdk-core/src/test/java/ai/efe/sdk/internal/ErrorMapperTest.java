package ai.efe.sdk.internal;

import ai.efe.sdk.exception.*;

import io.grpc.Status;
import io.grpc.StatusRuntimeException;

import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThat;

/** Sprint D26 — unit tests : ErrorMapper (gRPC Status → SDK sentinel). */
class ErrorMapperTest {

    @Test
    void mapUnauthenticatedToEfeUnauthenticated() {
        StatusRuntimeException sre = Status.UNAUTHENTICATED.withDescription("bad token").asRuntimeException();
        EfeException result = ErrorMapper.fromGrpcStatus(sre);
        assertThat(result).isInstanceOf(EfeUnauthenticatedException.class);
        assertThat(result.getCause()).isSameAs(sre);
    }

    @Test
    void mapPermissionDeniedToEfePermissionDenied() {
        StatusRuntimeException sre = Status.PERMISSION_DENIED.asRuntimeException();
        assertThat(ErrorMapper.fromGrpcStatus(sre)).isInstanceOf(EfePermissionDeniedException.class);
    }

    @Test
    void mapNotFoundToEfeCatalogNotFound() {
        StatusRuntimeException sre = Status.NOT_FOUND.asRuntimeException();
        assertThat(ErrorMapper.fromGrpcStatus(sre)).isInstanceOf(EfeCatalogNotFoundException.class);
    }

    @Test
    void mapFailedPreconditionToEfeCatalogInvalid() {
        StatusRuntimeException sre = Status.FAILED_PRECONDITION.asRuntimeException();
        assertThat(ErrorMapper.fromGrpcStatus(sre)).isInstanceOf(EfeCatalogInvalidException.class);
    }

    @Test
    void mapDeadlineExceededAndCancelledToEfeTimeout() {
        assertThat(ErrorMapper.fromGrpcStatus(Status.DEADLINE_EXCEEDED.asRuntimeException()))
            .isInstanceOf(EfeTimeoutException.class);
        assertThat(ErrorMapper.fromGrpcStatus(Status.CANCELLED.asRuntimeException()))
            .isInstanceOf(EfeTimeoutException.class);
    }

    @Test
    void mapUnavailableToEfeUnavailable() {
        StatusRuntimeException sre = Status.UNAVAILABLE.asRuntimeException();
        assertThat(ErrorMapper.fromGrpcStatus(sre)).isInstanceOf(EfeUnavailableException.class);
    }

    @Test
    void mapResourceExhaustedToEfeRateLimited() {
        StatusRuntimeException sre = Status.RESOURCE_EXHAUSTED.asRuntimeException();
        assertThat(ErrorMapper.fromGrpcStatus(sre)).isInstanceOf(EfeRateLimitedException.class);
    }

    @Test
    void mapInvalidArgumentAndOutOfRangeToEfeInvalidArgument() {
        assertThat(ErrorMapper.fromGrpcStatus(Status.INVALID_ARGUMENT.asRuntimeException()))
            .isInstanceOf(EfeInvalidArgumentException.class);
        assertThat(ErrorMapper.fromGrpcStatus(Status.OUT_OF_RANGE.asRuntimeException()))
            .isInstanceOf(EfeInvalidArgumentException.class);
    }

    @Test
    void mapInternalUnknownDataLossToEfeInternal() {
        assertThat(ErrorMapper.fromGrpcStatus(Status.INTERNAL.asRuntimeException()))
            .isInstanceOf(EfeInternalException.class);
        assertThat(ErrorMapper.fromGrpcStatus(Status.UNKNOWN.asRuntimeException()))
            .isInstanceOf(EfeInternalException.class);
        assertThat(ErrorMapper.fromGrpcStatus(Status.DATA_LOSS.asRuntimeException()))
            .isInstanceOf(EfeInternalException.class);
    }
}
