# PR #17 § 19.2.3 / T49 — defence-in-depth lint against PII leakage
# via metric label values.
#
# The runtime PII redaction filter (T49 mitigation layer 2) replaces
# matched PII patterns with `<redacted>`.  This Rego rule is layer 3 :
# it catches mistakes at CI time — operators who try to inject
# capital-letter participant IDs or other patterns that would either
# get redacted at runtime (cosmetically wrong) or slip past the
# runtime filter.
#
# Lints the rendered Helm chart's controller + participant
# Deployments + ConfigMap manifest for any participant_id env var or
# manifest field that doesn't conform to the documented lowercase-
# kebab-case-DNS-1123 taxonomy.

package efe.metric_label_lowercase

# ─── Helpers ────────────────────────────────────────────────────────

# The canonical participant_id taxonomy : DNS-1123 subdomain, max 63
# chars, lowercase only.
participant_id_regex := "^[a-z][a-z0-9-]{0,62}$"

is_meta_swarm_deployment(obj) {
    obj.kind == "Deployment"
    obj.metadata.labels["app.kubernetes.io/part-of"] == "efe-meta-swarm"
}

# ─── Rules ──────────────────────────────────────────────────────────

# Deny : a Deployment carries a participant_id label that violates the
# lowercase-kebab-case taxonomy.
deny[msg] {
    obj := input
    is_meta_swarm_deployment(obj)
    pid := obj.metadata.labels["efe.io/participant-id"]
    not regex.match(participant_id_regex, pid)
    msg := sprintf(
        "[efe-meta-swarm] %s/%s : participant_id label %q violates the lowercase-kebab-case taxonomy (PR #17 T49)",
        [obj.metadata.namespace, obj.metadata.name, pid]
    )
}

# Deny : ALLOWED_PEER_SANS env var contains an upper-case character.
# (Defence-in-depth — the chart's `_helpers.tpl` already lowercases,
# but a misconfigured override could slip through.)
deny[msg] {
    obj := input
    is_meta_swarm_deployment(obj)
    container := obj.spec.template.spec.containers[_]
    env := container.env[_]
    env.name == "EFE_META_SWARM_TLS_ALLOWED_PEER_SANS"
    contains_uppercase(env.value)
    msg := sprintf(
        "[efe-meta-swarm] %s/%s : EFE_META_SWARM_TLS_ALLOWED_PEER_SANS value %q contains upper-case (PR #17 T49)",
        [obj.metadata.namespace, obj.metadata.name, env.value]
    )
}

contains_uppercase(s) {
    upper := upper(s)
    not upper == s
}

# Warn : a participant_id is shorter than 3 characters — likely a
# placeholder.
warn[msg] {
    obj := input
    is_meta_swarm_deployment(obj)
    pid := obj.metadata.labels["efe.io/participant-id"]
    count(pid) < 3
    msg := sprintf(
        "[efe-meta-swarm] %s/%s : participant_id %q is suspiciously short",
        [obj.metadata.namespace, obj.metadata.name, pid]
    )
}
