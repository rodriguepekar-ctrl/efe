/*
 * Sprint D26 — Spring Boot Actuator example : REST controller.
 */
package com.example.efe;

import ai.efe.sdk.EfeClient;
import ai.efe.sdk.Verdict;
import ai.efe.sdk.exception.EfeException;

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

    @PostMapping("/classify")
    public ResponseEntity<?> classify(@RequestBody ClassifyRequest req) {
        try {
            Verdict v = efe.classify(req.text());
            return ResponseEntity.ok(v);
        } catch (EfeException e) {
            return ResponseEntity.status(500).body(Map.of(
                "error", e.getMessage(),
                "code", EfeException.errorCode(e)
            ));
        }
    }

    public record ClassifyRequest(String text) {}
}
