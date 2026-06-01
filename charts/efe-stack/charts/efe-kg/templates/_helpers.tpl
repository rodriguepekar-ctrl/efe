{{- define "efe-kg.fullname" -}}
{{- printf "%s-kg" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe-kg.labels" -}}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version | replace "+" "_" }}
app.kubernetes.io/name: {{ include "efe-kg.fullname" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe-stack
{{- end -}}

{{- define "efe-kg.selectorLabels" -}}
app.kubernetes.io/name: efe-kg
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
