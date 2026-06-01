/*
 * Sprint D26 — Spring Boot MVC example : REST controller.
 *
 * Maps SDK sentinel exceptions → HTTP status codes.
 */
package com.example.efe;

import ai.efe.sdk.EfeClient;
import ai.efe.sdk.Verdict;
import ai.efe.sdk.exception.*;

import org.springframework.http.HttpStatus;
import org.springframework.http.ResponseEntity;
import org.springframework.web.bind.annotation.*;

import java.util.Map;

@RestController
@RequestMapping("/api")
public class ClassifyController {

    private final EfeClient efe;

    public ClassifyController(EfeClient efe) {
        this.efe = efe;
    }

    /** POST /api/classify — body : `{"text": "..."}`. */
    @PostMapping("/classify")
    public ResponseEntity<?> classify(@RequestBody ClassifyRequest req) {
        if (req == null || req.text() == null || req.text().isBlank()) {
            return ResponseEntity.badRequest()
                .body(Map.of("error", "text is required", "code", "bad_request"));
        }
        try {
            Verdict v = efe.classify(req.text());
            return ResponseEntity.ok(v);
        } catch (EfeException e) {
            return ResponseEntity.status(efeErrorStatus(e))
                .body(Map.of(
                    "error", e.getMessage(),
                    "code", EfeException.errorCode(e)
                ));
        }
    }

    /** GET /api/healthz — probe upstream. */
    @GetMapping("/healthz")
    public ResponseEntity<Map<String, String>> healthz() {
        try {
            efe.healthcheck();
            return ResponseEntity.ok(Map.of("status", "serving"));
        } catch (EfeException e) {
            return ResponseEntity.status(HttpStatus.SERVICE_UNAVAILABLE)
                .body(Map.of("status", "not_serving", "error", e.getMessage()));
        }
    }

    /** Maps SDK sentinels → HTTP status codes (mirrors Go SDK gin/fiber/chi examples). */
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
