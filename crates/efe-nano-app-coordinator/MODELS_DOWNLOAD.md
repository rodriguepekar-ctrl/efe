# MODELS_DOWNLOAD.md — Modèles GGUF des nano-robots Phase 1

Ce document liste les **4 modèles GGUF** utilisés par les nano-robots Phase 1
(`analyzer`, `reformulator`, `compressor`, `verifier`), leurs URLs HuggingFace
canoniques, leurs tailles, leurs licences, et la procédure de téléchargement.

> **Pour télécharger en une commande** :
> ```bash
> ./scripts/slm/download_models.sh
> ```
> (Sprint S1 de la Phase 3 R2 — Missions Réelles Live)

---

## 1. Vue d'ensemble

| Robot | Modèle | Quant. | Port | Taille | Licence | Repo HF |
|---|---|---|---|---|---|---|
| `analyzer` | SmolLM2-360M-Instruct | Q5_K_M | 8090 | ~290 MB | Apache-2.0 | [unsloth/SmolLM2-360M-Instruct-GGUF](https://huggingface.co/unsloth/SmolLM2-360M-Instruct-GGUF) |
| `reformulator` | Qwen3-0.6B | Q5_K_M | 8091 | ~444 MB | Apache-2.0 | [unsloth/Qwen3-0.6B-GGUF](https://huggingface.co/unsloth/Qwen3-0.6B-GGUF) |
| `compressor` | Llama-3.2-1B-Instruct | Q5_K_M | 8092 | ~912 MB | Llama 3.2 Community License | [bartowski/Llama-3.2-1B-Instruct-GGUF](https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF) |
| `verifier` | Phi-4-mini-instruct | Q4_K_M | 8093 | ~2.5 GB | MIT | [unsloth/Phi-4-mini-instruct-GGUF](https://huggingface.co/unsloth/Phi-4-mini-instruct-GGUF) |

> **Note R2 S1 patch** : on privilégie les repos `unsloth/*` (validés HTTP 200 le 2026-05-27).
> Les repos officiels HF (HuggingFaceTB, Qwen) sont souvent incomplets en Q5_K_M.
> Le repo Bartowski Phi-4-mini est gated (401), unsloth est non-gated.

**Total cumulé** : ~4.1 GB. Mémoire RAM requise pour les 4 chargés simultanément :
~5-6 GB (modèle + KV cache 2-4 K).

**Pourquoi ces 4 modèles ?** Choix issus du PDF Phase 1 (LETTRES A & B) :
- **Analyzer** doit être très petit et rapide (sub-200 ms) → SmolLM2-360M.
- **Reformulator** doit gérer le multilingue (français + anglais) → Qwen3-0.6B.
- **Compressor** doit avoir une bonne qualité de résumé sans hallucination → Llama-3.2-1B (compromis qualité/taille).
- **Verifier** doit faire du raisonnement fact-checking → Phi-4-mini, le plus gros mais le plus rigoureux.

---

## 2. URLs HuggingFace directes

⚠️ **Les URLs peuvent évoluer.** Si une URL retourne 404, vérifier le repo HF
correspondant et mettre à jour `ROBOT_HF_URLS` dans `scripts/slm/lib_common.sh`.

### Analyzer — SmolLM2-360M-Instruct Q5_K_M

```
URL    : https://huggingface.co/unsloth/SmolLM2-360M-Instruct-GGUF/resolve/main/SmolLM2-360M-Instruct-Q5_K_M.gguf
Repo   : https://huggingface.co/unsloth/SmolLM2-360M-Instruct-GGUF
Taille : ~290 MB
Licence: Apache-2.0
```

Téléchargement manuel :
```bash
curl -L -o models/analyzer.gguf \
  https://huggingface.co/unsloth/SmolLM2-360M-Instruct-GGUF/resolve/main/SmolLM2-360M-Instruct-Q5_K_M.gguf
```

> Note : le repo officiel `HuggingFaceTB/SmolLM2-360M-Instruct-GGUF` n'a que la
> quantization Q8_0 (pas Q5_K_M). `unsloth/*` republie toutes les quants.

### Reformulator — Qwen3-0.6B Q5_K_M

```
URL    : https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q5_K_M.gguf
Repo   : https://huggingface.co/unsloth/Qwen3-0.6B-GGUF
Taille : ~444 MB
Licence: Apache-2.0
```

Téléchargement manuel :
```bash
curl -L -o models/reformulator.gguf \
  https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q5_K_M.gguf
```

> Note : le repo officiel `Qwen/Qwen3-0.6B-GGUF` n'a que la quantization Q8_0.
> Fallback si `unsloth/Qwen3-0.6B-GGUF` indisponible :
> `bartowski/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/Qwen2.5-0.5B-Instruct-Q5_K_M.gguf`.

### Compressor — Llama-3.2-1B-Instruct Q5_K_M

```
URL    : https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q5_K_M.gguf
Repo   : https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF
Taille : ~912 MB
Licence: Llama 3.2 Community License (gating requis via Meta — accepter sur HF)
```

⚠️ **Gating** : Llama 3.2 nécessite d'accepter la licence sur HuggingFace.
Soit te connecter (`huggingface-cli login`), soit utiliser le mirror Bartowski
qui republie souvent les modèles Llama sans gating (cf. URL ci-dessus).

Téléchargement manuel :
```bash
curl -L -o models/compressor.gguf \
  https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q5_K_M.gguf
```

### Verifier — Phi-4-mini-instruct Q4_K_M

```
URL    : https://huggingface.co/unsloth/Phi-4-mini-instruct-GGUF/resolve/main/Phi-4-mini-instruct-Q4_K_M.gguf
Repo   : https://huggingface.co/unsloth/Phi-4-mini-instruct-GGUF
Taille : ~2.5 GB
Licence: MIT
```

Téléchargement manuel :
```bash
curl -L -o models/verifier.gguf \
  https://huggingface.co/unsloth/Phi-4-mini-instruct-GGUF/resolve/main/Phi-4-mini-instruct-Q4_K_M.gguf
```

> Note : le repo `bartowski/Phi-4-mini-instruct-GGUF` retourne HTTP 401 (gated).
> Le repo `unsloth/*` est non-gated et propose les mêmes quantizations.

---

## 3. Vérification post-téléchargement

```bash
./scripts/slm/download_models.sh --check
```

Sortie attendue :
```
[ OK ]  analyzer.gguf présent (290 MB ≈ 290 MB attendu)
[ OK ]  reformulator.gguf présent (444 MB ≈ 444 MB attendu)
[ OK ]  compressor.gguf présent (912 MB ≈ 912 MB attendu)
[ OK ]  verifier.gguf présent (2500 MB ≈ 2500 MB attendu)
[ OK ]  Tous les modèles requis sont présents.
```

Tolérance ±10 % sur la taille (quantizations exactes peuvent légèrement varier
d'une version à l'autre).

---

## 4. Alternatives et profils RAM

| Profil | Quantizations recommandées | RAM totale ≈ |
|---|---|---|
| **Default R2** | Q5 / Q5 / Q5 / Q4 | 5-6 GB |
| **Light** (8 GB Mac) | Q4 / Q4 / Q4 / Q3 | 3-4 GB |
| **Heavy** (32 GB+ Mac) | Q8 / Q8 / Q8 / Q6 | 9-10 GB |

Pour activer un profil léger : modifier `ROBOT_HF_URLS` dans `lib_common.sh`
en remplaçant les suffixes `_q5_k_m` par `_q4_k_m`. Idem pour les tailles
dans `ROBOT_SIZES_MB`.

Note : les nano-robot clients **n'imposent pas** une quantization donnée —
ils parlent à llama-server via HTTP+GBNF, indépendamment du modèle servi.

---

## 5. Hash / intégrité (optionnel)

`download_models.sh` ne vérifie **pas** de SHA256. Raison : HuggingFace ne
publie pas systématiquement les SHA256 par fichier dans une API stable, et
les fichiers peuvent être ré-uploadés en cas de fix.

Pour vérifier toi-même :
```bash
shasum -a 256 models/*.gguf > models/SHA256SUMS
# puis comparer avec ce que HuggingFace montre dans "File info"
```

---

## 6. Hors-scope S1

- **Téléchargements via `huggingface-cli`** (plus rapide avec parallélisme) :
  envisagé en Phase 4 si le projet est massivement utilisé en CI.
- **Hash strict** : ajouté en Phase 4 si reproductibilité bit-perfect requise.
- **CDN miroir** : Phase 7 (si latence HF problématique en prod).
- **Fine-tuned variantes maison** : Phase 8 RLHF.
