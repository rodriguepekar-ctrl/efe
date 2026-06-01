{{/*
Common Helm helpers for the EFE chart (M12.8).

All names are truncated at 63 characters to honour the DNS subdomain
label limit k8s imposes on most object names.
*/}}

{{- define "efe.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "efe.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe.labels" -}}
helm.sh/chart: {{ include "efe.chart" . }}
{{ include "efe.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe
{{/* Sprint D9.A9 — multi-tenancy hook prepared for D11. Single-tenant
deployments leave .Values.tenant.id at "default" (no behavioural
change). D11 will read this label to apply per-tenant ResourceQuota
and LimitRange. Pinning it now avoids a chart refactor later. */}}
app.kubernetes.io/tenant: {{ default "default" .Values.tenant.id | quote }}
{{- end -}}

{{- define "efe.selectorLabels" -}}
app.kubernetes.io/name: {{ include "efe.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/*
efe.tenantId — exposed as a separate helper so individual templates
can consume it directly (e.g. NetworkPolicy ingress matchExpressions).
Sprint D9.A9.
*/}}
{{- define "efe.tenantId" -}}
{{- default "default" .Values.tenant.id -}}
{{- end -}}

{{- define "efe.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "efe.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/*
Compute the final image reference.
When `image.digest` is set, the digest wins over the tag — the strictly
preferred form in production because it is immune to tag re-mapping
attacks and makes the deployment pin-point reproducible.
*/}}
{{- define "efe.image" -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- if .Values.image.digest -}}
{{- printf "%s@%s" .Values.image.repository .Values.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}
{{- end -}}

{{/*
Render a gRPC-health-probe exec probe spec.

Callers pass the sub-values map (initialDelaySeconds, periodSeconds,
timeoutSeconds, failureThreshold) via `.probe` so the same helper can
serve liveness, readiness, and startup.

Note the `grpc_health_probe` binary is bundled in the image at
/usr/local/bin/grpc_health_probe (see docker/Dockerfile.prod).
*/}}
{{- define "efe.grpcHealthProbe" -}}
exec:
  command:
    - /usr/local/bin/grpc_health_probe
    - -addr=:{{ .Values.service.grpcPort }}
    - -connect-timeout=2s
initialDelaySeconds: {{ .probe.initialDelaySeconds }}
periodSeconds: {{ .probe.periodSeconds }}
timeoutSeconds: {{ .probe.timeoutSeconds }}
failureThreshold: {{ .probe.failureThreshold }}
{{- if .probe.successThreshold }}
successThreshold: {{ .probe.successThreshold }}
{{- end }}
{{- end -}}

{{/*
Resolve which Secret object to reference for HS256 key material.

Priority:
  1. `secrets.existingSecretName` (caller provides pre-made Secret).
  2. `secrets.externalSecretName` (created by external-secrets operator).
  3. `secrets.inline.enabled` (Helm-managed Secret, dev only).
*/}}
{{- define "efe.secretsName" -}}
{{- if .Values.secrets.existingSecretName -}}
{{- .Values.secrets.existingSecretName -}}
{{- else if .Values.secrets.externalSecretName -}}
{{- .Values.secrets.externalSecretName -}}
{{- else if .Values.secrets.inline.enabled -}}
{{- printf "%s-secrets" (include "efe.fullname" .) -}}
{{- end -}}
{{- end -}}
