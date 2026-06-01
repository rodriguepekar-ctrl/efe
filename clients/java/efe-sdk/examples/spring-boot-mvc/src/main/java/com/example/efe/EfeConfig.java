/*
 * Sprint D26 — Spring Boot MVC example : EfeClient bean factory.
 *
 * Demonstrates the canonical pattern for wiring an EfeClient as a
 * Spring-managed bean. Uses `destroyMethod = "close"` so the gRPC
 * channel shuts down cleanly when the Spring context closes.
 */
package com.example.efe;

import ai.efe.sdk.EfeClient;
import ai.efe.sdk.auth.JwtAuth;

import org.springframework.beans.factory.annotation.Value;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;

import java.time.Duration;

@Configuration
public class EfeConfig {

    @Bean(destroyMethod = "close")
    public EfeClient efeClient(@Value("${efe.endpoint}") String endpoint,
                                 @Value("${efe.jwt}") String jwt) {
        return EfeClient.builder()
            .endpoint(endpoint)
            .auth(JwtAuth.of(jwt))
            .deadline(Duration.ofSeconds(10))
            .userAgent("efe-sdk-spring-boot-mvc/0.0.0")
            .build();
    }
}
