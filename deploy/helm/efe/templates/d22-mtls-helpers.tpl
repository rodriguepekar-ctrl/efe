{{/*
Sprint D22 (Φ-B) — mTLS helper templates.

Cf. docs/EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md §10.4.

These helpers are EXTENSIONS to `_helpers.tpl` (post-D18 + A11 dedup).
Living in a sprint-tagged file so audit traceability is clean +
M12 deltas are easy to revert if needed.
*/}}

{{/*
mTLS Secret name. Either user-provided via `.Values.mtls.secretName`
or derived as `<fullname>-mtls-tls`.

The Deployment mounts this Secret read-only at `/etc/efe/tls/` ;
cert-manager populates it via the Certificate resource.

Pin tested by `test_m12_helm_extension_mtls_secret_name_helper`.
*/}}
{{- define "efe.mtls.secretName" -}}
{{- if and .Values.mtls (ne (default "" .Values.mtls.secretName) "") -}}
{{- .Values.mtls.secretName -}}
{{- else -}}
{{- printf "%s-mtls-tls" (include "efe.fullname" .) -}}
{{- end -}}
{{- end -}}

{{/*
Bundle hash for rolling-restart annotation (Sprint D27).

Combines the bundle slug + version + manifest SHA-256 into a stable
16-char hex digest. Mounted as a Pod annotation `efe.io/bundle-hash`
so a `helm upgrade` that changes the bundle triggers a rolling
restart automatically (Kubernetes-native pattern, mirrors
ConfigMap/Secret hash approach).

Pin tested by `test_m12_helm_extension_bundle_hash_helper`.
*/}}
{{- define "efe.bundle.hash" -}}
{{- $bundle := .Values.bundle | default dict -}}
{{- $slug := default "default" $bundle.slug -}}
{{- $ver := default "" $bundle.version -}}
{{- $sha := default "" $bundle.manifestSha256 -}}
{{- printf "%s-%s-%s" $slug $ver $sha | sha256sum | trunc 16 -}}
{{- end -}}
