{{- define "portal-admin.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "portal-admin.fullname" -}}
{{- printf "%s-%s" .Release.Name (include "portal-admin.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "portal-admin.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{ include "portal-admin.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/component: ui
efe.io/tenant: {{ .Values.global.tenant | quote }}
{{- end -}}

{{- define "portal-admin.selectorLabels" -}}
app.kubernetes.io/name: {{ include "portal-admin.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "portal-admin.image" -}}
{{- $reg := .Values.image.registry | default .Values.global.imageRegistry -}}
{{- $tag := .Values.image.tag | default .Chart.AppVersion -}}
{{- printf "%s/%s:%s" $reg .Values.image.repository $tag -}}
{{- end -}}

{{- define "portal-admin.host" -}}
{{- .Values.ingress.host | default (printf "portal.%s" .Values.global.domain) -}}
{{- end -}}
