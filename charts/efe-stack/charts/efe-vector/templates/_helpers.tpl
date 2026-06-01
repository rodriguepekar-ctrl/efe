{{- define "efe-vector.fullname" -}}
{{- printf "%s-vector" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe-vector.labels" -}}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version | replace "+" "_" }}
app.kubernetes.io/name: {{ include "efe-vector.fullname" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe-stack
{{- end -}}

{{- define "efe-vector.selectorLabels" -}}
app.kubernetes.io/name: efe-vector
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
