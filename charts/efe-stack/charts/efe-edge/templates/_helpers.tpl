{{/*
  efe-edge sub-chart helpers.
*/}}

{{- define "efe-edge.fullname" -}}
{{- printf "%s-edge" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe-edge.labels" -}}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version | replace "+" "_" }}
app.kubernetes.io/name: {{ include "efe-edge.fullname" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe-stack
{{- end -}}

{{- define "efe-edge.envoy.selectorLabels" -}}
app.kubernetes.io/name: efe-envoy
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "efe-edge.ratelimit.selectorLabels" -}}
app.kubernetes.io/name: efe-ratelimit
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "efe-edge.redis.selectorLabels" -}}
app.kubernetes.io/name: efe-redis-cluster
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "efe-edge.pgbouncer.selectorLabels" -}}
app.kubernetes.io/name: efe-pgbouncer
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
