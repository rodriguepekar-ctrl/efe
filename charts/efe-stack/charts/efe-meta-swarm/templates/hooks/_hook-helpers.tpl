{{/*
  ─────────────────────────────────────────────────────────────────
  PR #14 — Helm hook helpers (shared between pre-upgrade snapshot +
  post-upgrade SIGHUP-all Jobs).
  Cf. `docs/EFE_A1_PHASE2_PR14_ARCHITECTURE.md` § 10.1.
  ─────────────────────────────────────────────────────────────────
*/}}

{{/*
  efe-meta-swarm.hookName — release-prefixed hook resource name.
  Caller passes (root, hookKind) as a list, e.g.,
    (include "efe-meta-swarm.hookName" (list . "pre-upgrade-snapshot"))
*/}}
{{- define "efe-meta-swarm.hookName" -}}
{{- $root := index . 0 -}}
{{- $hook := index . 1 -}}
{{- printf "%s-hook-%s" (include "efe-meta-swarm.fullname" $root) $hook | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
  efe-meta-swarm.hookServiceAccountName — dedicated SA used by
  every PR #14 hook Job.  Phase-2 RBAC : least-privilege via the
  Role rendered in `hook-role.yaml`.
*/}}
{{- define "efe-meta-swarm.hookServiceAccountName" -}}
{{- printf "%s-hooks" (include "efe-meta-swarm.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
  efe-meta-swarm.hookLabels — labels applied to every hook resource
  (Job + SA + Role + RoleBinding).  Extends the chart's standard
  labels with `app.kubernetes.io/component: hook`.
*/}}
{{- define "efe-meta-swarm.hookLabels" -}}
{{ include "efe-meta-swarm.labels" . }}
app.kubernetes.io/component: hook
{{- end -}}
