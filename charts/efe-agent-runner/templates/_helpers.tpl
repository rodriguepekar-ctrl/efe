{{/*
Common labels.
*/}}
{{- define "efe-agent-runner.labels" -}}
app.kubernetes.io/name: efe-agent-runner
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version }}
{{- end -}}

{{- define "efe-agent-runner.selectorLabels" -}}
app.kubernetes.io/name: efe-agent-runner
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "efe-agent-runner.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
