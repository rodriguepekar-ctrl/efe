/*
 * Sprint D26 (Φ-B) — `efe-sdk-java` version constant.
 *
 * Pin tested by `test_d26_java_sdk_version_const` (Python).
 * Bumps : private `v0.x` → public `v1.0.0` at GA.
 */
package ai.efe.sdk;

/**
 * Current SDK version.
 *
 * <p>Aligned with the EFE platform's semver. Sub-1.0 = private/beta
 * (Φ-B) ; v1.0.0 = first public Maven Central publication.
 *
 * @since 0.4.0
 */
public final class Version {

    private Version() {}

    /** Canonical SDK version string. */
    public static final String VERSION = "0.4.0";
}
