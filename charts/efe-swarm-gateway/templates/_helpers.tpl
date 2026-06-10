{{/* SPDX-License-Identifier: UNLICENSED */}}

{{- define "efe-swarm-gateway.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe-swarm-gateway.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name (include "efe-swarm-gateway.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "efe-swarm-gateway.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/* Common labels. */}}
{{- define "efe-swarm-gateway.labels" -}}
helm.sh/chart: {{ include "efe-swarm-gateway.chart" . }}
{{ include "efe-swarm-gateway.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: efe-swarm
{{- end -}}

{{- define "efe-swarm-gateway.selectorLabels" -}}
app.kubernetes.io/name: {{ include "efe-swarm-gateway.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "efe-swarm-gateway.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "efe-swarm-gateway.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/* Image tag : valeur explicite sinon appVersion. */}}
{{- define "efe-swarm-gateway.image" -}}
{{- printf "%s:%s" .Values.image.repository (default .Chart.AppVersion .Values.image.tag) -}}
{{- end -}}

{{/* Nom du Secret HKDF (existant ou géré par le chart). */}}
{{- define "efe-swarm-gateway.hkdfSecretName" -}}
{{- if .Values.hkdf.existingSecret -}}
{{- .Values.hkdf.existingSecret -}}
{{- else -}}
{{- printf "%s-hkdf" (include "efe-swarm-gateway.fullname" .) -}}
{{- end -}}
{{- end -}}

{{- define "efe-swarm-gateway.hkdfSecretKey" -}}
{{- if .Values.hkdf.existingSecret -}}
{{- .Values.hkdf.existingSecretKey -}}
{{- else -}}
master
{{- end -}}
{{- end -}}
