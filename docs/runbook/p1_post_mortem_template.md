# Post-mortem P1 — `<incident_id>`

> **Mode** : *Blameless*. L'objectif est d'identifier des **mécanismes**,
> pas des fautifs.
> **Référence** : `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 8.3.

---

## 1. Métadonnées

| Champ | Valeur |
|-------|--------|
| ID incident | `<INCIDENT-YYYY-MM-DD-NN>` |
| Date début | `<ISO-8601>` |
| Date fin | `<ISO-8601>` |
| Durée totale | `<HH:MM>` |
| Alerte d'origine | P1_BURN_FAST / P1_BURN_SLOW / P1_TAIL_p99_9_HIGH / autre |
| Sévérité | P0 / P1 / P2 |
| Auteur du post-mortem | `<email>` |
| Reviewers | `<emails>` |
| Statut | DRAFT / REVIEWED / SIGNED-OFF |

---

## 2. Résumé exécutif

> *Une phrase. Si vous devez raconter à un directeur en 30 secondes
> ce qui s'est passé.*

Exemple : "Pendant N min, le SLO C-1 a été violé pour M % des requêtes
Mode-1 d'un tenant à cause de Z."

---

## 3. Impact mesuré

### 3.1 Métriques techniques

| Métrique | Avant | Pendant | Après |
|----------|------:|--------:|------:|
| p99 latency (Mode-1) | 187 ms | 410 ms | 188 ms |
| Burn rate 1 h | 0.2× | 28× | 0.2× |
| Requêtes affectées | n/a | `<N>` | n/a |
| Error budget consommé | n/a | `<X %>` | n/a |
| Réplicas en panne | 0 | `<n>` | 0 |

### 3.2 Impact client

- Tenants impactés : `<liste>`
- Requêtes lentes (> 200 ms) : `<N>`
- Requêtes 503 (auto-shed) : `<N>`
- Pénalités SLA déclenchées : oui / non

### 3.3 Impact commercial

- Crédits SLA dus : `<montant>` (calcul cf. § 9.3 doc P1)
- Posts publics : status page / blog / customer comms

---

## 4. Timeline

> *Horodatage UTC ; précision à la minute.*

| Heure | Événement | Source |
|-------|-----------|--------|
| 14:02:31 | Deploy v0.2.5 commencé | Argo CD |
| 14:04:11 | Pods `efe-server-{0,1,2}` ready | k8s |
| 14:07:45 | p99 commence à dériver | dashboard |
| 14:12:30 | Alerte `P1_BURN_FAST` fire | Prometheus |
| 14:13:02 | On-call paged (PagerDuty) | PD |
| 14:14:00 | On-call open le dashboard, voit la corrélation deploy | obs |
| 14:14:45 | Rollback v0.2.4 initié | `kubectl rollout undo` |
| 14:17:30 | Rollout terminé, p99 retombe | obs |
| 14:22:00 | Alerte resolves | Prometheus |
| 14:25:00 | Confirmé stable, escalade fermée | on-call |

---

## 5. Root cause analysis

### 5.1 Cause immédiate (*proximate*)

> *La chose technique qui a déclenché l'incident.*

Exemple : "Une refactor du `BudgetTracker` introduisait une
allocation `Vec::with_capacity()` à chaque étage, contournant la
lint AST (ratchet baseline = 86, l'ajout n'a pas été détecté car
le test ratchet ne vérifie que les counts, pas leur composition)."

### 5.2 Cause profonde (*root cause*)

> *Le mécanisme qui a permis à la cause immédiate d'arriver.
> Typiquement 3+ "pourquoi ?" en arrière.*

Exemple :
- *Pourquoi* l'alloc est arrivée ? → Refactor non revu attentivement.
- *Pourquoi* pas attrapée en CI ? → Le ratchet baseline n'invalide
  pas les composition shifts (déplacer 1 alloc d'un fichier à un
  autre quand les counts baseline le permettent).
- *Pourquoi* le ratchet est ainsi designé ? → Compromis pragmatique
  pour le démarrer sur 86 violations existantes.
- *Pourquoi* on a 86 violations existantes ? → Hot-path policy
  introduite en D19 mais sprint de cleanup pas planifié.

### 5.3 Contributing factors

- Le runner CI bench-p1 ne tournait pas sur la PR (config skip
  par défaut sur PR pour économie GPU).
- Pas de canary deploy : 100 % du trafic basculait vers la nouvelle
  version sans validation graduelle.
- L'alloc en question ne se déclenchait que sous charge sustained ;
  les tests unitaires ne l'ont pas vue.

---

## 6. Action items

| ID | Description | Sévérité | Owner | ETA |
|----|-------------|:--------:|-------|-----|
| P1-AI-001 | Étendre le ratchet baseline pour gater aussi sur le **shift cumulé** (somme de tous les fichiers) en plus du shift per-file | HIGH | `<owner>` | D20 |
| P1-AI-002 | Configurer le job `bench-p1-surface` pour run sur PR si le diff touche `crates/efe-{server,core}/src/**` | HIGH | `<owner>` | D20 |
| P1-AI-003 | Implémenter canary deploy 5 % → 50 % → 100 % via Argo Rollouts | MED | `<owner>` | D22 |
| P1-AI-004 | Sprint de cleanup hot-path (visée -50 % violations) | MED | tech lead | D25 |
| P1-AI-005 | Ajouter chaos test "alloc injection sur fusion" qui simule l'incident | LOW | `<owner>` | D31 |

---

## 7. Ce qui a bien fonctionné

- Alerte FAST a fire en 5 min après le début de la dérive.
- Le runbook `p1_burn_fast.md` a permis à l'on-call de rollback
  en 2 min après être pagé.
- Le `host_fingerprint_match` est resté à 1 → la mesure reste
  opposable, le client peut consulter le rapport SLA.

---

## 8. Ce qui a moins bien fonctionné

- Le déclenchement a coïncidé avec un cluster maintenance (un node
  drainé) → confusion sur la nature de l'incident pendant les 2 premières min.
- La doc status page n'a pas été mise à jour automatiquement
  (process manuel actuellement).

---

## 9. Suivi des Action Items

> *Ce post-mortem est revu en sprint review tant que tous les AI ne
> sont pas done. Le tech lead suit l'avancement dans le board Jira
> sous l'epic `P1-PM-<incident_id>`.*

| AI | Status | Date close |
|----|:------:|------------|
| P1-AI-001 | TODO / DOING / DONE | |
| P1-AI-002 | TODO / DOING / DONE | |
| ... | | |

---

## 10. Sign-off

| Rôle | Nom | Date | Signature |
|------|-----|------|-----------|
| Auteur post-mortem | | | |
| Tech lead Core | | | |
| Directeur SRE | | | |
| (Si crédit SLA) Customer Success | | | |
