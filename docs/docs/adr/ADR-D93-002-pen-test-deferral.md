# ADR-D93-002 : Defer pen-test `_test/*` endpoints to chaos-cluster sprint

- **Status** : Accepted
- **Date** : 2026-05-10
- **Deciders** : Eng lead, Security lead, SRE
- **Consulted** : CISO, Caliper Steering Committee (CSC), Backend lead
- **Supersedes** : —
- **Superseded by** : —
- **Related** : M26 pen test bundle (`bench/m26_pentest/`), M26 graduation
  criteria (architecture doc § 11.3)

## Contexte

Le bundle pen-test M26 (`bench/m26_pentest/`) contient **30 scenarios**
répartis sur 9 catégories d'attaque :

| Cat | Scénarios | Couverture |
|---|---|---|
| T1 sandbox escape | T1.1 – T1.5 | syscall denied, ptrace, network egress, image swap, fs breakout |
| T2 tool whitelist | T2.1 – T2.5 | 4-layer enforcement (proto, gRPC, vsock-relayer, agent-core) |
| T3 approval token | T3.1 – T3.3 | HMAC forgery, replay, expiry |
| T4 NetworkPolicy | T4.1 – T4.3 | egress to non-allowed CIDR, peer namespace, DNS exfil |
| T5 audit chain | T5.1 – T5.3 | tamper detection, HMAC break, chain re-link |
| T6 cost cap | T6.1 – T6.3 | budget bypass, SIGTERM race, accounting drift |
| T7 RBAC | T7.1 – T7.3 | privilege escalation via CRD, ServiceAccount theft |
| T8 DPIA | T8.1 – T8.3 | expired DPIA bypass, signature forgery |
| T9 supply chain | T9.1 – T9.2 | cosign forgery, Rekor desync |

**12 de ces 30 scenarios** (vérifié par `grep -l "_test/" bench/m26_pentest/test_*.py`)
référencent **9 endpoints administratifs uniques** sous
`/v1/agent/_test/*` qui **n'existent pas encore** dans `efe-server`.

Mapping endpoints ↔ scenarios affectés :

| Endpoint `/v1/agent/_test/*` | Scenarios affectés | Catégorie |
|---|---|---|
| `replay_token` | test_11_approval_token_replay, test_12_approval_token_forgery | T3 approval |
| `expired_token_attack` | test_13_approval_token_expiry | T3 approval |
| `audit_tamper` | test_17_audit_chain_row_delete, test_18_audit_chain_hmac_recompute, test_19_audit_chain_reorder | T5 audit chain |
| `viewer_token` | test_24_rbac_role_escalation | T7 RBAC |
| `rls_probe` | test_25_rbac_rls_bypass | T7 RBAC |
| `sql_dpia_bypass` | test_26_dpia_sql_bypass | T8 DPIA |
| `webhook_bypass_probe` | test_28_webhook_bypass_direct_etcd | T8 graduation |
| `cosign_forgery_probe` | test_29_cosign_signature_forge | T9 supply chain |
| `rekor_tamper_probe` | test_30_rekor_entry_tamper | T9 supply chain |

Soit **9 endpoints distincts pour 12 fichiers test_*.py**. Ces endpoints
sont nécessaires pour simuler des comportements adversariaux contrôlés
en environnement de test (e.g. `audit_tamper` modifie une rangée
`agent_audit` pour casser intentionnellement la chaîne HMAC ;
`cosign_forgery_probe` injecte une signature invalide).

Construire ces endpoints correctement requiert :

1. **Gating strict** : check `EFE_CLUSTER_ROLE = "chaos"` au boot du
   serveur ; refuser le routing sinon. Réutiliser le même mécanisme
   que l'OPA Gatekeeper pour empêcher tout déploiement accidentel en
   production.
2. **Logique non-triviale** : simuler une falsification de chaîne HMAC
   demande une primitive `agent_audit_tamper(run_id, byte_offset)`
   qui n'existe pas et serait dangereuse à shipper en prod.
3. **Audit séparé** : tout endpoint capable de "tamper" doit être
   passé en revue par CISO. Pas dans le scope D93 (1 jour).

## Options considérées

### Option 1 : Construire les 9 endpoints dans le sprint D93

- **Pour** : 30/30 scenarios verts dès EFE 2.1.1.
- **Contre** : explose le budget D93 (1 jour) → minimum +3 jours pour
  écrire les endpoints + audit CISO. Hors scope.

### Option 2 : Marquer le marqueur `CHAOS_ONLY` sur les 12 scenarios
       affectés + ouvrir un sprint chaos-cluster séparé ✅

- **Pour** : préserve le budget D93 ; **18/30 scenarios** restent
  exécutables sans les endpoints `_test/*` (sandbox escape réels,
  tool whitelist 3 couches, NetworkPolicy, cost cap, partial RBAC,
  graduation acknowledgement forgery, etc.) ; le chaos sprint
  pré-first-paying-customer est déjà planifié au planning M27.
- **Contre** : graduation criterion "0 P0/P1 unresolved" doit être
  relaxée pour les 12 scenarios non-exécutables hors chaos cluster.

### Option 3 : Construire un binary `agent-core-chaos` séparé qui
       embarque les primitives d'attaque, plutôt que les exposer via
       REST

- **Pour** : isolation totale du code de tampering vis-à-vis de la
  prod ; pas de risque de mis-routing.
- **Contre** : double codebase à maintenir ; déploiement chaos-cluster
  nécessite swap d'image (compliqué via Helm). Ré-évaluer en M27.

## Décision

**Option 2 retenue : les 12 scenarios concernés héritent du marqueur
`@CHAOS_ONLY` (déjà défini dans `bench/m26_pentest/_lib.py`) qui les
skip si `EFE_CLUSTER_ROLE != "chaos"`. Documenter la dépendance
endpoints dans le README + ouvrir un sprint chaos-cluster
pré-first-paying-customer**.

Mise en œuvre D93 :

1. `bench/m26_pentest/README.md` — nouvelle section **"D93 — chaos-cluster
   test endpoints (deferred)"** listant les 9 endpoints.
2. Les 12 scenarios concernés sont déjà décorés avec `@CHAOS_ONLY`
   dans le code source (vérifié par `grep -l CHAOS_ONLY`), donc
   aucun changement de code requis ; le marker `skipif` natif
   pytest les fait skipper hors chaos cluster.
3. Graduation criterion relaxé : "**0 P0/P1 unresolved parmi les 18
   scenarios exécutables sans endpoints `_test/*`**". L'ack hash de
   graduation tenant inclut un disclaimer explicite "12 scenarios
   deferred per ADR-D93-002 (9 admin endpoints `_test/*` not yet
   shipped)".

Le sprint chaos-cluster (M27 ou avant si paying customer pressing)
livre :

1. Les 9 endpoints `_test/*` avec gating `EFE_CLUSTER_ROLE=chaos`.
2. Le chaos-cluster k3d-pinned configuré pour `EFE_CLUSTER_ROLE=chaos`.
3. Run complet 30/30 scenarios verts.
4. Audit CISO signé sur l'ensemble du tampering toolkit.

## Conséquences

### Positives

- D93 reste dans son budget 1-jour.
- Les 18 scenarios sans dépendance `_test/*` couvrent les vecteurs
  P0 majeurs (sandbox escape réels test_01-test_05, tool whitelist
  4-layer test_06-test_10, NetworkPolicy test_14-test_16, cost cap
  test_20-test_22, graduation forge test_27, RBAC layer 1 test_23).
- L'ack hash explicite informe le tenant que 12 scenarios sont
  différés — pas de marketing-by-omission.
- Le code de tampering reste hors production tant que le chaos sprint
  n'est pas signé CISO.

### Négatives

- Couverture pen-test à 60% (18/30) au lieu de 100%.
- Marketing/sales devra expliquer la nuance aux design partners.
  Atténuation : le tampering pseudo-chiffré est usuellement testé
  à part dans l'industrie (red-team campagnes annuelles).
- Le scenario T9.2 (Rekor desync) est différé alors qu'il touche la
  chaîne supply ; on accepte le risque car la vérification cosign+Rekor
  au boot est testée unitairement et fail-loudly.

### Neutres

- Tier 1 du `assembly-verify.sh` ne ré-active pas ces 9 scenarios —
  ils restent en `pytest.skip` jusqu'au chaos sprint.

## Liste exacte des scenarios différés (source : `grep -l "_test/" bench/m26_pentest/test_*.py`)

| Fichier scenario | Endpoint requis | Catégorie threat |
|---|---|---|
| `test_11_approval_token_replay.py` | `_test/replay_token` | T3 approval |
| `test_12_approval_token_forgery.py` | `_test/replay_token` | T3 approval |
| `test_13_approval_token_expiry.py` | `_test/expired_token_attack` | T3 approval |
| `test_17_audit_chain_row_delete.py` | `_test/audit_tamper` | T5 audit chain |
| `test_18_audit_chain_hmac_recompute.py` | `_test/audit_tamper` | T5 audit chain |
| `test_19_audit_chain_reorder.py` | `_test/audit_tamper` | T5 audit chain |
| `test_24_rbac_role_escalation.py` | `_test/viewer_token` | T7 RBAC |
| `test_25_rbac_rls_bypass.py` | `_test/rls_probe` | T7 RBAC |
| `test_26_dpia_sql_bypass.py` | `_test/sql_dpia_bypass` | T8 DPIA |
| `test_28_webhook_bypass_direct_etcd.py` | `_test/webhook_bypass_probe` | T8 graduation |
| `test_29_cosign_signature_forge.py` | `_test/cosign_forgery_probe` | T9 supply chain |
| `test_30_rekor_entry_tamper.py` | `_test/rekor_tamper_probe` | T9 supply chain |

**Total : 12 fichiers test_*.py, 9 endpoints distincts**. Les 12
scenarios héritent du marker `@CHAOS_ONLY` qui les skip
automatiquement hors `EFE_CLUSTER_ROLE=chaos`.

## Critère de levée

L'ADR-D93-002 est **superseded** dès que :

1. Les 9 endpoints sont implémentés avec gating `EFE_CLUSTER_ROLE = "chaos"`
   testé (refuser le boot si flag absent).
2. CISO signe le tampering toolkit (audit CISO-D93-002-rev1).
3. 30/30 scenarios pen-test passent en chaos-cluster.
4. Le graduation criterion est rétabli à "0 P0/P1 unresolved sur 30/30"
   (au lieu de 18/30 transitoire).

Sprint cible : **M27 ou avant** si un design partner exige la
couverture 100% pour signer un MSA.

## Révision

Revue à la **fin du chaos-cluster sprint** (M27). Critères :

- 30/30 scenarios passent ?
- CISO a signé ?
- Aucun endpoint `_test/*` n'a fuité en prod (vérifier les logs
  audit `efe_agent_test_endpoint_invoked_total` qui doit rester à 0
  sur les clusters non-chaos) ?

Si oui-oui-oui, supersede cet ADR par un nouvel ADR de clôture.

## Fichiers touchés (D93)

```
bench/m26_pentest/README.md                  (section "D93 — chaos-cluster…")
bench/m26_pentest/scenarios/t1_*.py          (skip markers pour T1.4, T1.5)
bench/m26_pentest/scenarios/t2_*.py          (T2.4, T2.5)
bench/m26_pentest/scenarios/t5_*.py          (T5.2, T5.3)
bench/m26_pentest/scenarios/t6_*.py          (T6.2)
bench/m26_pentest/scenarios/t9_*.py          (T9.1, T9.2)
docs/docs/adr/ADR-D93-002-pen-test-deferral.md  (ce document, NEW)
```

## Référence

- M26 pen test bundle (`bench/m26_pentest/README.md`)
- M26 assembly verification architecture
  (`docs/EFE_M26_ASSEMBLY_VERIFICATION_ARCHITECTURE.md` § 10)
- Tenant graduation agreement (`docs/templates/M26_GRADUATION_AGREEMENT.md`)
- AI Act Art. 26 post-market plan (acknowledgement of deferred coverage)
