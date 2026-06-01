/*
 * Sprint D26 — Spring Boot Actuator example : @AutoConfiguration starter pattern.
 *
 * Demonstrates how to author an `efe-sdk-spring-boot-starter` artifact.
 * The starter (publishable separately) would be released in D27 ; this
 * example is in-line for D26 to keep the publish pipeline focused on
 * the core SDK + mock.
 */
package com.example.efe;

import ai.efe.sdk.EfeClient;
import ai.efe.sdk.auth.AuthConfig;
import ai.efe.sdk.auth.JwtAuth;
import ai.efe.sdk.auth.NoAuth;

import io.prometheus.metrics.model.registry.PrometheusRegistry;

import org.springframework.boot.autoconfigure.AutoConfiguration;
import org.springframework.boot.autoconfigure.condition.ConditionalOnMissingBean;
import org.springframework.boot.context.properties.ConfigurationProperties;
import org.springframework.boot.context.properties.EnableConfigurationProperties;
import org.springframework.context.annotation.Bean;

import java.time.Duration;

/**
 * Auto-configures an {@link EfeClient} bean from properties.
 *
 * <p>Properties (under {@code efe.}) :
 * <ul>
 *   <li>{@code efe.endpoint} : gRPC endpoint (required).</li>
 *   <li>{@code efe.jwt} : JWT token (optional ; if blank, NoAuth used).</li>
 *   <li>{@code efe.deadline-ms} : per-call deadline in ms (default 10_000).</li>
 * </ul>
 */
@AutoConfiguration
@EnableConfigurationProperties(EfeAutoConfiguration.EfeProperties.class)
public class EfeAutoConfiguration {

    @Bean
    @ConditionalOnMissingBean
    public PrometheusRegistry prometheusRegistry() {
        return new PrometheusRegistry();
    }

    @Bean(destroyMethod = "close")
    @ConditionalOnMissingBean
    public EfeClient efeClient(EfeProperties props, PrometheusRegistry prometheusRegistry) {
        AuthConfig auth = (props.jwt() == null || props.jwt().isBlank())
            ? NoAuth.INSTANCE
            : JwtAuth.of(props.jwt());
        return EfeClient.builder()
            .endpoint(props.endpoint())
            .auth(auth)
            .deadline(Duration.ofMillis(props.deadlineMs()))
            .prometheus(prometheusRegistry)
            .userAgent("efe-sdk-spring-boot-actuator/0.0.0")
            .build();
    }

    @ConfigurationProperties("efe")
    public record EfeProperties(
        String endpoint,
        String jwt,
        long deadlineMs
    ) {
        public EfeProperties {
            if (deadlineMs <= 0) deadlineMs = 10_000L;
        }
    }
}
