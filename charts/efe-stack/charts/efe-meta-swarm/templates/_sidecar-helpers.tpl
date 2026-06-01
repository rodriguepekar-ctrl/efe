{{/*
  ─────────────────────────────────────────────────────────────────
  PR #14 — bundle weight-loader sidecar helper (AD7-AD9).
  Cf. `docs/EFE_A1_PHASE2_PR14_ARCHITECTURE.md` § 13.1.

  In Phase-2 the sidecar image defaults to `gcr.io/google-containers/pause:3.9`
  (a no-op) so operators can opt-in to the Pod-level shape (second
  container + shared volume + shareProcessNamespace) WITHOUT running
  any real bundle-loader logic.  PR #15 will swap the image for the
  real polling binary and uses :
    GET `${EFE_BUNDLE_ENDPOINT_URL}/metadata?id=${EFE_BUNDLE_PARTICIPANT_ID}`
    GET `${EFE_BUNDLE_ENDPOINT_URL}/blob?id=...`     (on SHA change)
    atomic rename → `${EFE_BUNDLE_WEIGHTS_PATH}`
    `kill -HUP 1` (signals participant via shareProcessNamespace)
  ─────────────────────────────────────────────────────────────────
*/}}

{{- define "efe-meta-swarm.sidecarContainer" -}}
{{- $root := index . 0 -}}
{{- $p := index . 1 -}}
- name: bundle-loader
  # PR #14 audit pass — drop the `| default ...` from `image` and
  # `pollIntervalSeconds` : Sprig treats `""` and `0` as empty, so
  # operator-explicit edge values get silently rewritten.  values.yaml
  # carries the defaults via Helm's deep-merge ; the schema validates
  # both fields (`minLength: 1` for image, `[5, 3600]` for poll).
  image: {{ $root.Values.bundle.sidecar.image | quote }}
  imagePullPolicy: IfNotPresent
  securityContext:
    {{- include "efe-meta-swarm.commonContainerSecurityContext" $root | nindent 4 }}
  env:
    - name: EFE_BUNDLE_PARTICIPANT_ID
      value: {{ $p.id | quote }}
    - name: EFE_BUNDLE_ENDPOINT_URL
      value: {{ $root.Values.bundle.endpointUrl | quote }}
    - name: EFE_BUNDLE_POLL_INTERVAL_S
      value: {{ $root.Values.bundle.pollIntervalSeconds | quote }}
    - name: EFE_BUNDLE_WEIGHTS_PATH
      value: /var/lib/efe-meta-swarm/weights/current.bin
  resources:
    {{- toYaml $root.Values.bundle.sidecar.resources | nindent 4 }}
  volumeMounts:
    - name: weights
      mountPath: /var/lib/efe-meta-swarm/weights
{{- end -}}
