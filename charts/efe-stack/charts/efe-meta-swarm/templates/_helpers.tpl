{{/*
  ─────────────────────────────────────────────────────────────────
  efe-meta-swarm — named templates (PR #13).
  Cf. docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md § 10.
  ─────────────────────────────────────────────────────────────────
*/}}

{{/*
  efe-meta-swarm.name — short chart name (max 63 chars, DNS-compliant).
*/}}
{{- define "efe-meta-swarm.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
  efe-meta-swarm.fullname — release-prefixed, max 63 chars.
*/}}
{{- define "efe-meta-swarm.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{/*
  efe-meta-swarm.chart — chart-version label (sanitized).
*/}}
{{- define "efe-meta-swarm.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
  efe-meta-swarm.labels — full label set + chart version + tenant.
  Applied to ALL resources rendered by this sub-chart.

  PR #14 — adds `global.commonLabels` propagation from the umbrella
  (e.g. cost-tracking labels, `efe.io/managed-by: helm`, etc.).
*/}}
{{- define "efe-meta-swarm.labels" -}}
helm.sh/chart: {{ include "efe-meta-swarm.chart" . }}
{{ include "efe-meta-swarm.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe
efe.io/tenant: {{ .Values.global.tenant | default "default" | quote }}
{{- with .Values.global.commonLabels }}
{{ toYaml . }}
{{- end }}
{{- end -}}

{{/*
  efe-meta-swarm.selectorLabels — used in Deployment.spec.selector
  AND Service.spec.selector.  MUST stay identical for pod targeting.
*/}}
{{- define "efe-meta-swarm.selectorLabels" -}}
app.kubernetes.io/name: {{ include "efe-meta-swarm.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/*
  efe-meta-swarm.controllerName — Deployment / Service name for the
  controller.  Trunc-63 enforced so this name remains a legal K8s
  metadata.name regardless of release-name length.  (PR #14 audit
  pass : prior version omitted trunc, leaving the door open to
  DNS-63 overflow for long release names.)
*/}}
{{- define "efe-meta-swarm.controllerName" -}}
{{- printf "%s-controller" (include "efe-meta-swarm.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
  efe-meta-swarm.participantName — Deployment / Service name for a
  given participant id.  Caller passes (root, id) as a list.
  Trunc-63 enforced (PR #14 audit pass).
*/}}
{{- define "efe-meta-swarm.participantName" -}}
{{- $root := index . 0 -}}
{{- $id := index . 1 -}}
{{- printf "%s-participant-%s" (include "efe-meta-swarm.fullname" $root) $id | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
  ─────────────────────────────────────────────────────────────────
  PR #14 audit pass — dedicated helpers for derived names that get
  *another* suffix appended at callsites.  Without these, the suffix
  is appended to an already-63-char base, overflowing the K8s
  metadata.name limit silently.  Concrete cases caught in audit :
      ${controllerName}-tls         (cert-manager Secret)
      ${controllerName}-priority    (PriorityClass cluster-scoped)
      ${controllerName}-pdb         (PodDisruptionBudget)
      ${participantName}-tls        (cert-manager Secret per id)
  Each helper reserves enough buffer for its own suffix so the final
  rendered name still fits 63 chars even with a max-length base name.
  Critically, the Secret name in the Certificate MUST equal the
  secretName in the Deployment's volume — both go through the same
  helper here, guaranteeing the match.
  ─────────────────────────────────────────────────────────────────
*/}}

{{/*
  efe-meta-swarm.controllerTlsSecretName — Secret name produced by the
  cert-manager Certificate for the controller, AND the
  volume.secret.secretName reference in controller-deployment.yaml.
  Reserve 4 chars for "-tls" suffix.
*/}}
{{- define "efe-meta-swarm.controllerTlsSecretName" -}}
{{- printf "%s-tls" (include "efe-meta-swarm.controllerName" . | trunc 59 | trimSuffix "-") -}}
{{- end -}}

{{/*
  efe-meta-swarm.participantTlsSecretName — Secret name per participant
  id.  Caller passes (root, id) as a list.  Same matching constraint
  as controllerTlsSecretName.
*/}}
{{- define "efe-meta-swarm.participantTlsSecretName" -}}
{{- $root := index . 0 -}}
{{- $id := index . 1 -}}
{{- printf "%s-tls" (include "efe-meta-swarm.participantName" (list $root $id) | trunc 59 | trimSuffix "-") -}}
{{- end -}}

{{/*
  efe-meta-swarm.controllerPriorityClassName — PriorityClass cluster-
  scoped name + the spec.priorityClassName reference in the controller
  Deployment.  Reserve 9 chars for "-priority" suffix.
*/}}
{{- define "efe-meta-swarm.controllerPriorityClassName" -}}
{{- printf "%s-priority" (include "efe-meta-swarm.controllerName" . | trunc 54 | trimSuffix "-") -}}
{{- end -}}

{{/*
  efe-meta-swarm.controllerPdbName — PodDisruptionBudget name.  Reserve
  4 chars for "-pdb" suffix.
*/}}
{{- define "efe-meta-swarm.controllerPdbName" -}}
{{- printf "%s-pdb" (include "efe-meta-swarm.controllerName" . | trunc 59 | trimSuffix "-") -}}
{{- end -}}

{{/*
  efe-meta-swarm.controllerServiceAccountName — operator-overridable,
  defaults to <fullname>-controller.
*/}}
{{- define "efe-meta-swarm.controllerServiceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (printf "%s-controller" (include "efe-meta-swarm.fullname" .)) .Values.serviceAccount.controllerName -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.controllerName -}}
{{- end -}}
{{- end -}}

{{/*
  efe-meta-swarm.participantServiceAccountName — per-participant SA
  name.  Caller passes (root, id) as a list.
*/}}
{{- define "efe-meta-swarm.participantServiceAccountName" -}}
{{- $root := index . 0 -}}
{{- $id := index . 1 -}}
{{- if $root.Values.serviceAccount.create -}}
{{- printf "%s-participant-%s" (include "efe-meta-swarm.fullname" $root) $id -}}
{{- else -}}
default
{{- end -}}
{{- end -}}

{{/*
  efe-meta-swarm.image — fully qualified image ref.  Rejects empty tag
  (defence in depth ; values.schema.json also enforces it).
*/}}
{{- define "efe-meta-swarm.image" -}}
{{- $tag := .Values.image.tag -}}
{{- if empty $tag -}}
{{- fail "efe-meta-swarm: .Values.image.tag MUST be set (no :latest policy ; AD17). See README.md / runbook." -}}
{{- end -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}

{{/*
  efe-meta-swarm.participantsEnv — comma-separated env value for the
  controller's `EFE_META_SWARM_PARTICIPANTS` ; renders
  `id=svc.ns.svc:port,id=svc.ns.svc:port,...` from .Values.participants.
*/}}
{{- define "efe-meta-swarm.participantsEnv" -}}
{{- $root := . -}}
{{- $entries := list -}}
{{- range .Values.participants -}}
{{- $svc := printf "%s-participant-%s" (include "efe-meta-swarm.fullname" $root) .id -}}
{{- $entry := printf "%s=%s.%s.svc:%d" .id $svc $root.Release.Namespace (int .service.grpcPort) -}}
{{- $entries = append $entries $entry -}}
{{- end -}}
{{- join "," $entries -}}
{{- end -}}

{{/*
  PR #16 — efe-meta-swarm.controllerAllowedParticipantSans

  Comma-separated list of DNS SANs of every participant service.
  The controller's `EFE_META_SWARM_TLS_ALLOWED_PEER_SANS` env var
  consumes this when `tls.mutual=true`.

  Each participant contributes two entries :
      <svc>.<namespace>.svc
      <svc>.<namespace>.svc.cluster.local

  Pre-computed here (instead of inlining in controller-deployment.yaml)
  so the YAML value can be a single quoted string rather than a fragile
  folded scalar `>-` that embeds Go template logic.

  Caller : `{{ include "efe-meta-swarm.controllerAllowedParticipantSans" . }}`.
*/}}
{{- define "efe-meta-swarm.controllerAllowedParticipantSans" -}}
{{- $root := . -}}
{{- $sans := list -}}
{{- range $p := .Values.participants -}}
{{- $svc := include "efe-meta-swarm.participantName" (list $root $p.id) -}}
{{- $sans = append $sans (printf "%s.%s.svc" $svc $root.Release.Namespace) -}}
{{- $sans = append $sans (printf "%s.%s.svc.cluster.local" $svc $root.Release.Namespace) -}}
{{- end -}}
{{- join "," $sans -}}
{{- end -}}

{{/*
  PR #16 — efe-meta-swarm.participantAllowedControllerSans

  Comma-separated list of DNS SANs the participant accepts from the
  controller.  Phase 2 single-controller topology : exactly 2 entries
  for the controller's Service FQDN (short + cluster.local).
*/}}
{{- define "efe-meta-swarm.participantAllowedControllerSans" -}}
{{- $ctrl := include "efe-meta-swarm.controllerName" . -}}
{{- $ns := .Release.Namespace -}}
{{- printf "%s.%s.svc,%s.%s.svc.cluster.local" $ctrl $ns $ctrl $ns -}}
{{- end -}}

{{/*
  efe-meta-swarm.manifestYaml — the rendered Phase-2 manifest YAML body.
  Consumed by templates/configmap-manifest.yaml AND (in CI) by the
  golden-snapshot tooling for round-tripping through Rust's
  `manifest::load_and_validate` (PR #8).
*/}}
{{- define "efe-meta-swarm.manifestYaml" -}}
version: {{ .Values.manifest.version | quote }}
meta:
  dim: {{ .Values.manifest.meta.dim }}
  safety:
    kind: {{ .Values.manifest.meta.safety.kind }}
{{- if eq .Values.manifest.meta.safety.kind "box" }}
    radius: {{ required "manifest.meta.safety.radius is required when kind=box" .Values.manifest.meta.safety.radius }}
{{- else if eq .Values.manifest.meta.safety.kind "poly" }}
    dim_a: {{ required "manifest.meta.safety.poly.dim_a is required when kind=poly" .Values.manifest.meta.safety.poly.dim_a }}
    a_flat: {{ required "manifest.meta.safety.poly.a_flat is required when kind=poly" .Values.manifest.meta.safety.poly.a_flat | toJson }}
    b:      {{ required "manifest.meta.safety.poly.b is required when kind=poly" .Values.manifest.meta.safety.poly.b | toJson }}
{{- else if eq .Values.manifest.meta.safety.kind "hull" }}
    version: {{ required "manifest.meta.safety.hull.version is required when kind=hull" .Values.manifest.meta.safety.hull.version | quote }}
    serialized_model_path: {{ required "manifest.meta.safety.hull.serialized_model_path is required when kind=hull" .Values.manifest.meta.safety.hull.serialized_model_path | quote }}
{{- end }}
participants:
{{- range .Values.participants }}
  - id: {{ .id }}
    dim: {{ .dim }}
    kappa: {{ .kappa }}
    seed: {{ .seed }}
    blas_backend: {{ .blasBackend | default "none" | quote }}
{{- end }}
iteration:
  max_iters:      {{ .Values.manifest.iteration.max_iters }}
  tolerance:      {{ .Values.manifest.iteration.tolerance }}
  alpha_default:  {{ .Values.manifest.iteration.alpha_default }}
  coupling_gamma: {{ .Values.manifest.iteration.coupling_gamma }}
  deterministic:  {{ .Values.manifest.iteration.deterministic }}
  max_wall_ms:    {{ .Values.manifest.iteration.max_wall_ms }}
{{- end -}}

{{/*
  efe-meta-swarm.commonPodSecurityContext — pod-level securityContext
  shared by controller + participants.  Phase-2 hardening (AD8).
*/}}
{{- define "efe-meta-swarm.commonPodSecurityContext" -}}
runAsNonRoot: true
runAsUser: 65532
runAsGroup: 65532
fsGroup: 65532
seccompProfile:
  type: RuntimeDefault
{{- end -}}

{{/*
  efe-meta-swarm.commonContainerSecurityContext — per-container
  securityContext.  Phase-2 hardening (AD9).
*/}}
{{- define "efe-meta-swarm.commonContainerSecurityContext" -}}
allowPrivilegeEscalation: false
capabilities:
  drop: ["ALL"]
readOnlyRootFilesystem: true
runAsNonRoot: true
runAsUser: 65532
{{- end -}}
