{{/*
PR #20 — Helm template helpers.  Standard naming + label conventions.
*/}}

{{- define "efe-loadgen.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{- define "efe-loadgen.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end }}
{{- end }}

{{- define "efe-loadgen.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "efe-loadgen.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end }}

{{/* Standard labels applied to every object. */}}
{{- define "efe-loadgen.labels" -}}
app.kubernetes.io/name: {{ include "efe-loadgen.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/component: loadgen
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version | replace "+" "_" }}
efe.pr: "20"
{{- end }}

{{/* Selector labels — subset stable for matchLabels. */}}
{{- define "efe-loadgen.selectorLabels" -}}
app.kubernetes.io/name: {{ include "efe-loadgen.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
