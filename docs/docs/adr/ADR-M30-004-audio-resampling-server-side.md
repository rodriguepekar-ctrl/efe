# ADR-M30-004 : Re-échantillonnage audio côté serveur (pas client)

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M30_MULTIMODAL_ARCHITECTURE.md § 7

## Décision

M30 refuse les WAV dont le sample-rate header ≠ 16 kHz. Le caller
doit pré-convertir (ffmpeg dans leur proxy). M30.1 ajoutera un
re-échantillonneur software (`rubato`) côté serveur.

## Justification

1. **Dépendance minimale** — éviter `rubato` (~ 200 KB binary
   bloat) tant que le volume audio est faible (< 5 RPS).
2. **Erreur claire** — `EncodeFailed { reason: "sample rate X != 16000" }`
   guide directement le client vers la solution (ffmpeg -ar 16000).
3. **Server load** — re-sampling à 192 kHz → 16 kHz coûte ~ 5 ms
   CPU. Acceptable mais pas gratuit ; mieux vaut que le client
   le fasse.
4. **Whisper hardcode 16 kHz** — un re-sampling défectueux côté
   serveur (artefacts d'aliasing) dégrade silencieusement la WER.

## Conséquences

### Positives
- Pas de pièce de plus à monitorer dans le pipeline audio.
- Le client a un signal clair quand son enregistrement n'est pas
  conforme.

### Négatives
- Friction onboarding — les caller doivent connaître ffmpeg.
- M30.1 devra ajouter `rubato 0.16` + ré-écrire `audio.rs`
  resample step.

## Migration M30.1

Quand `rubato` arrive, le check `sr != 16000` devient un
`resample(audio_bytes, sr → 16000)`. L'interface publique
`AudioEncoder::encode` ne change pas.
