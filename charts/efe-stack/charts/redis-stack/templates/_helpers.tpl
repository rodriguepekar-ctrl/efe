{{- define "redis-stack.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "redis-stack.fullname" -}}
{{- printf "%s-%s" .Release.Name (include "redis-stack.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "redis-stack.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{ include "redis-stack.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/component: cache
{{- end -}}

{{- define "redis-stack.selectorLabels" -}}
app.kubernetes.io/name: {{ include "redis-stack.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
