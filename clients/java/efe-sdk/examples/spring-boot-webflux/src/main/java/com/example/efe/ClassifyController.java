/*
 * Sprint D26 — Spring Boot WebFlux example : reactive REST controller.
 *
 * Bridges the SDK's CompletableFuture API to Reactor's Mono via
 * `Mono.fromFuture(...)`. SDK sentinels → HTTP status codes via
 * `onErrorResume`.
 */
package com.example.efe;

import ai.efe.sdk.EfeClient;
import ai.efe.sdk.Verdict;
import ai.efe.sdk.exception.*;

import org.springframework.http.ResponseEntity;
import org.springframework.web.bind.annotation.*;

import reactor.core.publisher.Mono;

import java.util.Map;

@RestController
@RequestMapping("/api")
public class ClassifyController {

    private final EfeClient efe;

    public ClassifyController(EfeClient efe) {
        this.efe = efe;
    }

    @PostMapping("/classify")
    public Mono<ResponseEntity<?>> classify(@RequestBody ClassifyRequest req) {
        if (req == null || req.text() == null || req.text().isBlank()) {
            return Mono.just(ResponseEntity.badRequest()
                .body(Map.of("error", "text is required", "code", "bad_request")));
        }
        return Mono.fromFuture(efe.classifyAsync(req.text()))
            .map(v -> ResponseEntity.ok().body((Object) v))
            .onErrorResume(EfeException.class, e ->
                Mono.just(ResponseEntity.status(efeErrorStatus(e))
                    .body(Map.of(
                        "error", e.getMessage(),
                        "code", EfeException.errorCode(e)
                    ))));
    }

    @GetMapping("/healthz")
    public Mono<ResponseEntity<Map<String, String>>> healthz() {
        return Mono.fromRunnable(efe::healthcheck)
            .then(Mono.just(ResponseEntity.ok(Map.of("status", "serving"))))
            .onErrorResume(EfeException.class, e ->
                Mono.just(ResponseEntity.status(503)
                    .body(Map.of("status", "not_serving", "error", e.getMessage()))));
    }

    private static int efeErrorStatus(EfeException e) {
        if (e instanceof EfeUnauthenticatedException) return 401;
        if (e instanceof EfePermissionDeniedException) return 403;
        if (e instanceof EfeInvalidArgumentException) return 400;
        if (e instanceof EfeCatalogNotFoundException) return 404;
        if (e instanceof EfeCatalogInvalidException) return 422;
        if (e instanceof EfeTimeoutException) return 504;
        if (e instanceof EfeUnavailableException) return 503;
        if (e instanceof EfeRateLimitedException) return 429;
        return 500;
    }

    public record ClassifyRequest(String text) {}
}
