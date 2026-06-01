{{- define "efe-server.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe-server.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "efe-server.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{ include "efe-server.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/component: backend
efe.io/tenant: {{ .Values.global.tenant | quote }}
{{- end -}}

{{- define "efe-server.selectorLabels" -}}
app.kubernetes.io/name: {{ include "efe-server.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "efe-server.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "efe-server.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "efe-server.image" -}}
{{- $reg := .Values.image.registry | default .Values.global.imageRegistry -}}
{{- $tag := .Values.image.tag | default .Chart.AppVersion -}}
{{- printf "%s/%s:%s" $reg .Values.image.repository $tag -}}
{{- end -}}

{{- define "efe-server.imagePullSecrets" -}}
{{- with .Values.global.imagePullSecrets -}}
imagePullSecrets:
{{- toYaml . | nindent 0 }}
{{- end -}}
{{- end -}}

{{/*
Database URL secret reference. Selects between PGO-generated secret and
external Postgres secret based on global.externalPostgres.enabled.
*/}}
{{- define "efe-server.dbSecretName" -}}
{{- if .Values.global.externalPostgres.enabled -}}
{{- .Values.global.externalPostgres.secretRef.name -}}
{{- else -}}
efe-postgres-pguser-efe
{{- end -}}
{{- end -}}

{{- define "efe-server.dbSecretKey" -}}
{{- if .Values.global.externalPostgres.enabled -}}
{{- .Values.global.externalPostgres.secretRef.key -}}
{{- else -}}
uri
{{- end -}}
{{- end -}}
