{{/*
Expand the name of the chart.
*/}}
{{- define "efe-stack.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a fully qualified app name.
*/}}
{{- define "efe-stack.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Common labels applied to every resource.
*/}}
{{- define "efe-stack.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{ include "efe-stack.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
efe.io/tenant: {{ .Values.global.tenant | quote }}
efe.io/environment: {{ .Values.global.environment | default "prod" | quote }}
{{- with .Values.global.commonLabels }}
{{ toYaml . }}
{{- end }}
{{- end }}

{{/*
Selector labels — stable subset used by Service / Deployment selectors.
*/}}
{{- define "efe-stack.selectorLabels" -}}
app.kubernetes.io/name: {{ include "efe-stack.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Compute the full image reference.
Usage: {{ include "efe-stack.image" (dict "image" .Values.image "context" .) }}
*/}}
{{- define "efe-stack.image" -}}
{{- $registry := .image.registry | default .context.Values.global.imageRegistry -}}
{{- $repository := .image.repository -}}
{{- $tag := .image.tag | default .context.Chart.AppVersion -}}
{{- printf "%s/%s:%s" $registry $repository $tag -}}
{{- end -}}

{{/*
imagePullSecrets render block.
*/}}
{{- define "efe-stack.imagePullSecrets" -}}
{{- with .Values.global.imagePullSecrets }}
imagePullSecrets:
{{- toYaml . | nindent 2 }}
{{- end }}
{{- end -}}
