{{- define "efe-pii.fullname" -}}
{{- printf "%s-pii" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe-pii.labels" -}}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version | replace "+" "_" }}
app.kubernetes.io/name: {{ include "efe-pii.fullname" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe-stack
{{- end -}}

{{- define "efe-pii.selectorLabels" -}}
app.kubernetes.io/name: efe-pii
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
