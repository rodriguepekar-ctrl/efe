/*
 * Sprint D26 — Spring Boot Actuator example application.
 *
 * Demonstrates the @AutoConfiguration starter pattern with Prometheus
 * metrics expose via /actuator/prometheus.
 */
package com.example.efe;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;

@SpringBootApplication
public class Application {

    public static void main(String[] args) {
        SpringApplication.run(Application.class, args);
    }
}
