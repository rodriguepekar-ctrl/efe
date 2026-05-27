# Phase 3 R2 — Sprint S1 : Live Validation

**Date de validation** : 2026-05-27 (15:26 UTC)
**Hardware** : Mac M-series, 36 GB RAM
**OS** : macOS
**llama-server** : build 9297

---

## Résultat final

✅ **4/4 daemons UP avec inférence Metal validée**

```text
ROBOT          PORT   PID    STATE   p50     VERDICT
analyzer       8090   82633  ALIVE   93 ms   GPU (Metal/CUDA)
reformulator   8091   82678  ALIVE   167 ms  GPU (Metal/CUDA)
compressor     8092   82719  ALIVE   107 ms  GPU (Metal/CUDA)
verifier       8093   82778  ALIVE   429 ms  GPU (Metal/CUDA)
```

**Critère d'acceptation S1 atteint** :
- 4 health-checks PASS
- 4 inférences réelles PASS
- 4 verdicts GPU Metal/CUDA
- 0 failure sur 12 requêtes bench
- `.env.slm` écrit avec les 4 endpoints

---

## Métriques live

### Téléchargement des 4 modèles (3 nouveaux, 1 déjà présent)

| Robot | Modèle | Repo HF | Taille DL |
|---|---|---|---|
| analyzer | SmolLM2-360M-Instruct Q5_K_M | unsloth/SmolLM2-360M-Instruct-GGUF | 276 MB |
| reformulator | Qwen3-0.6B Q5_K_M | unsloth/Qwen3-0.6B-GGUF | 423 MB |
| compressor | Llama-3.2-1B-Instruct Q5_K_M | bartowski/Llama-3.2-1B-Instruct-GGUF | 869 MB |
| verifier | Phi-4-mini-instruct Q4_K_M | unsloth/Phi-4-mini-instruct-GGUF | 2376 MB |
| **Total** | | | **~3.9 GB** |

### Démarrage daemons (chargement modèle + ouverture port)

| Robot | Latence de READY |
|---|---|
| analyzer | 0 s |
| reformulator | 0 s |
| compressor | 0 s |
| verifier | 1 s |

Metal sur Mac M-series charge instantanément un modèle GGUF déjà sur SSD.

### Inférence réelle (POST /completion, n_predict=1)

| Robot | Latence |
|---|---|
| analyzer | 50 ms |
| reformulator | 80 ms |
| compressor | 98 ms |
| verifier | 175 ms |

Tous validés `✓ inference OK`.

### Bench latence (3 req × 16 tokens, post-warmup)

| Robot | MIN | P50 | P99 | MAX | AVG | FAIL |
|---|---|---|---|---|---|---|
| analyzer | 93 | **93** | 106 | 145 | 114 | 0 |
| reformulator | 167 | **167** | 168 | 202 | 179 | 0 |
| compressor | 107 | **107** | 109 | 140 | 118 | 0 |
| verifier | 429 | **429** | 430 | 507 | 455 | 0 |

**Verdict bench.sh** : `GPU (Metal/CUDA)` sur les 4 robots
(critère : p50 < 500 ms = Metal actif).

---

## Bugs corrigés pendant la validation live (4 commits)

| Commit | Bug |
|---|---|
| `4d4d25e` | 3 URLs HuggingFace cassées (404 + 401 gating). Migration vers unsloth/* |
| `4d4d25e` | exit silencieux e2e_smoke : `set -e` + capture rc. Fix avec `&& rc=0 \|\| rc=$?` |
| `8e4defc` | Espace dans `jjjj copie` cassait `--model`. Fix avec array bash `"${LLAMA_ARGS[@]}"` |
| `99c0053` | `status.sh printf %.1f` rejeté sur macOS. Fix arithmétique entière pure |
| `99c0053` | e2e_smoke filtrait l'output des étapes 5/6. Fix : fallback `tail -15` |
| `dc77f70` | macOS BSD `date +%s%N` non supporté. Fix `ns_now()` avec python3 |
| `dc77f70` | bash printf rejette floats. Fix `float_to_int_ms()` avec awk |
| `dc77f70` | bash 3.2 rejette literals `1_000_000`. Fix `1000000` |

---

## Procédure de validation reproductible

```bash
# Pré-requis : llama-server b9297+, port 8090 libre
# 1. Libérer port 8090 si occupé
docker stop monitor-region-router  # ou tout autre process

# 2. Lancer la méta-validation S1
./scripts/slm/e2e_smoke.sh --download --keep-running

# 3. Vérifier la sortie finale :
#    ╔════════════════════════════════════════════════════════╗
#    ║         ✅ Sprint S1 validé end-to-end                 ║
#    ╚════════════════════════════════════════════════════════╝

# 4. Tester chaque composant individuellement
./scripts/slm/health.sh                    # 4 × ● UP
./scripts/slm/verify_inference.sh          # 4 × ✓ inference OK
./scripts/slm/bench.sh -n 5                # verdict GPU sur 4/4

# 5. Endpoints prêts pour Sprint S2
source .env.slm                            # ANALYZER_URL etc.
```

---

## Commits R2 S1 cumulés (7 commits)

```text
dc77f70 fix(phase-3-r2-s1): macOS nanos + float printf + bash 3.2 literals
99c0053 fix(phase-3-r2-s1): status.sh printf locale + e2e_smoke output filtering
8e4defc fix(phase-3-r2-s1): espace dans path repo casse passage --model à llama-server
4d4d25e fix(phase-3-r2-s1): 3 URLs HuggingFace cassées + bug exit silencieux e2e_smoke
2e95a0f feat(phase-3-r2-s1-patch): combler 8 gaps chirurgicaux pour atteindre 100% S1
214e191 feat(phase-3-r2-s1): bootstrap llama-server local pour nano-robots Phase 1
d89d957 docs(phase-3-r2): architecture chirurgicale Missions Réelles Live
```

Total LOC S1 : ~3700 lignes (11 scripts + 2 docs + Justfile + .gitignore).

---

## Conclusion S1

Phase 3 R2 Sprint S1 est **livré et validé en conditions réelles** sur Mac M-series. Les 4 nano-robots Phase 1 sont opérationnels en Metal avec des latences sub-500 ms, prêts à être branchés dans `Coordinator::execute_mission_a/b/speculative` au Sprint S2.

**Prochaine étape : Sprint S2 — Mission A naked (Coordinator + Claude, aucun nano-robot activé)** dans `crates/efe-coordinator/tests/live_missions.rs`.
