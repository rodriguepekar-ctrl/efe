{{- define "efe-triton.fullname" -}}
{{- printf "%s-triton" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe-triton.labels" -}}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version | replace "+" "_" }}
app.kubernetes.io/name: {{ include "efe-triton.fullname" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe-stack
{{- end -}}

{{- define "efe-triton.selectorLabels" -}}
app.kubernetes.io/name: efe-triton
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
