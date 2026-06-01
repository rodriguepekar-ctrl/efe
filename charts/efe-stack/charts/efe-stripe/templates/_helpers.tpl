{{- define "efe-stripe.fullname" -}}
{{- printf "%s-stripe" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe-stripe.labels" -}}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version | replace "+" "_" }}
app.kubernetes.io/name: {{ include "efe-stripe.fullname" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe-stack
{{- end -}}

{{- define "efe-stripe.selectorLabels" -}}
app.kubernetes.io/name: efe-stripe
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
