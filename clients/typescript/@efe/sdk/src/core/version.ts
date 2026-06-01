// Sprint D24 (Φ-B) — `@efe/sdk` version constant.
//
// Pin tested by `test_d24_typescript_sdk_version_const`.
// Bumps : private `v0.x` → public `v1.0.0` at GA.

/**
 * Current SDK version.
 *
 * Aligned with the EFE platform's semver. Sub-1.0 = private/beta
 * (Φ-B) ; v1.0.0 = first public npm publication.
 */
export const Version = '0.3.0' as const;
