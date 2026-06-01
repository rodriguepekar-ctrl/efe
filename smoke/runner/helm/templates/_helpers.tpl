{{/* PR #23 § 15 — template helpers. */}}

{{- define "efe-smoke.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "efe-smoke.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe-smoke.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | quote }}
app.kubernetes.io/name: {{ include "efe-smoke.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/component: smoke
app.kubernetes.io/part-of: efe-meta-swarm
stability_contract: v1.5
efe.io/smoke: "true"
{{- end -}}

{{- define "efe-smoke.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "efe-smoke.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "efe-smoke.validate" -}}
{{- if eq (.Values.mode | lower) "production" -}}
{{- fail "mode=production is forbidden (PR #21 AD11)" -}}
{{- end -}}
{{- if not .Values.chartVersion -}}
{{- fail "chartVersion is required (PR #23 § 13)" -}}
{{- end -}}
{{- if not .Values.rbac.namespaceScoped -}}
{{- fail "rbac.namespaceScoped must be true (PR #23 AD53 / T108)" -}}
{{- end -}}
{{- end -}}
