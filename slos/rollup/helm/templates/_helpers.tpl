{{- /* PR #22 § 16 — template helpers. */ -}}

{{- define "efe-slo-rollup.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "efe-slo-rollup.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe-slo-rollup.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | quote }}
app.kubernetes.io/name: {{ include "efe-slo-rollup.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/component: slo-rollup
app.kubernetes.io/part-of: efe-meta-swarm
stability_contract: v1.4
{{- end -}}

{{- define "efe-slo-rollup.selectorLabels" -}}
app.kubernetes.io/name: {{ include "efe-slo-rollup.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "efe-slo-rollup.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "efe-slo-rollup.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "efe-slo-rollup.validate" -}}
{{- if eq (.Values.mode | lower) "production" -}}
{{- fail "mode=production is forbidden (PR #22 AD11)" -}}
{{- end -}}
{{- if not .Values.chartVersion -}}
{{- fail "chartVersion is required (PR #22 § 16.2 frozen values surface)" -}}
{{- end -}}
{{- if not .Values.rbac.namespaceScoped -}}
{{- fail "rbac.namespaceScoped must be true (PR #22 T103/T104)" -}}
{{- end -}}
{{- end -}}
