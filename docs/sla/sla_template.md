# Modèle de Service Level Agreement — EFE (Espace Latent Éphémère)

> Version : `v0.2.0-draft` (livré sprint D19 P1-Foundation)
> Référence architecture : `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 9.
> **Statut** : *template juridique brut*. Doit être revu par un
> cabinet juridique avant signature client. Toutes les valeurs
> numériques (seuils, pénalités, durées) sont **par défaut** ; un
> contrat peut les ajuster en clauses particulières.

---

## §1. Définitions

Dans le présent Accord :

- **Service** désigne la plateforme EFE telle que mise à disposition
  du Client par le Fournisseur via l'environnement de production
  identifié en Annexe A.
- **Mode-1** désigne le mode d'exécution "zero-shot" du Service,
  caractérisé par `mode=1` dans le paramètre de chaque requête
  `ExecuteRequest` au sens de la documentation technique
  publiée à `https://docs.efe.ai/api/v1`.
- **Requête** désigne un appel RPC unitaire à `efe.v1.SwarmService.Execute`.
- **Latence** désigne la durée mesurée du côté serveur entre la
  réception complète des bytes de la requête sur la socket TCP du
  Service et la fin de l'écriture des bytes de la réponse sur cette
  même socket. La mesure utilise l'horloge `CLOCK_MONOTONIC` du
  kernel Linux du Service.
- **Mois de Service** désigne un mois calendaire complet (00:00 UTC
  le 1er → 23:59:59 UTC le dernier jour).
- **`host_fingerprint`** désigne le hash SHA-256 du tuple matériel
  défini en Annexe A § A.4.
- **Rapport SLA** désigne le document mensuel produit par le
  Fournisseur, signé HMAC-SHA-256 par sa clé d'audit publique
  (cf. Annexe B).

---

## §2. Disponibilité

> *Clause indépendante de P1 ; reproduite ici pour completness.*

Le Fournisseur s'engage à ce que le Service soit Disponible **au
moins 99.9 %** du temps cumulé d'un Mois de Service. La Disponibilité
est mesurée selon les modalités précisées en Annexe C.

---

## §3. Performance — engagement de Latence (« P1 »)

### §3.1 Engagement

Le Fournisseur garantit que, sur un Mois de Service, le **99e
centile** (« p99 ») de la Latence des Requêtes Mode-1 émises par
le Client dans les Conditions de Référence (cf. Annexe A) sera
**inférieur ou égal à deux cent millisecondes (200 ms)**.

### §3.2 Modalités de mesure

(a) La mesure est réalisée sur des **fenêtres mensuelles calendaires**.

(b) Le Fournisseur produit, dans les **10 jours ouvrables** suivant
la fin de chaque Mois de Service, un Rapport SLA contenant au moins :
- le p99, p99.9 et p50 mesurés ;
- le burn rate aux fenêtres 1 h, 6 h, 24 h ;
- la liste exhaustive des incidents impactant la Latence (avec
  référence au post-mortem public) ;
- le `host_fingerprint_sha256` actif pendant le Mois ;
- l'attestation HMAC du Rapport (cf. Annexe B).

(c) Le Rapport SLA est mis à disposition du Client à l'adresse
`https://sla.efe.ai/<tenant_id>/<year>-<month>.json` et est
**vérifiable indépendamment** via l'outil `efe-attest verify-sla`
publié dans le SDK officiel.

### §3.3 Pénalités

En cas de **Violation Soutenue** (cf. §3.4), le Fournisseur accorde
au Client un crédit de service sur la prochaine facture mensuelle :

| Durée de Violation Soutenue dans le Mois | Crédit |
|------------------------------------------|:------:|
| ≥ 7 jours et < 14 jours | **10 %** de la facture du Mois |
| ≥ 14 jours et < 21 jours | **25 %** de la facture du Mois |
| ≥ 21 jours | **50 %** de la facture du Mois |

Le crédit n'est jamais remboursé en espèces.

### §3.4 Violation Soutenue

Une journée calendaire (00:00 → 23:59:59 UTC) est dite « Violée »
si, sur cette journée, le p99 des Requêtes Mode-1 dépasse 200 ms.

### §3.5 Conditions de défense

(a) Toute Requête servie pendant que `host_fingerprint_sha256`
diverge de la baseline signée (cf. Annexe A § A.4) est **exclue
de la mesure** et le SLO §3.1 est **suspendu** pour la fenêtre de
mesure correspondante. Le Fournisseur déclare la dérive au Client
dans les 24 h via le canal d'incident.

(b) Toute Requête émise hors des Conditions de Référence (charge
> 220 RPS, taille d'entrée > 512 tokens, mode autre que Mode-1, etc.)
est exclue de la mesure.

(c) Les périodes de **maintenance planifiée** (annoncées ≥ 5 jours
ouvrables à l'avance) sont exclues, plafond 4 h / Mois.

(d) Les cas de **force majeure** au sens du droit applicable
(Annexe D) suspendent l'engagement pour leur durée.

### §3.6 Procédure de contestation par tiers

Le Client peut, à ses frais, faire constater par un cabinet d'audit
technique indépendant la conformité ou la non-conformité du
Fournisseur sur les modalités de mesure §3.2. La procédure suit
Annexe E.

**Si le cabinet établit une non-conformité confirmée**, les frais
d'audit sont remboursés intégralement par le Fournisseur, et le
crédit §3.3 du Mois concerné est appliqué d'office sur la facture
suivante.

---

## §4. Notification d'incident

Toute violation supérieure à **15 minutes consécutives** déclenche
une notification proactive au Client par email à l'adresse du
contact technique déclaré, sous un délai de **30 minutes**.

Un compte-rendu d'incident (RCA) est publié dans les **4 heures**
suivant la résolution.

Un post-mortem complet est publié dans les **5 jours ouvrables**
suivant la résolution. Le format est public et accessible à
`https://status.efe.ai/incidents/<incident_id>`.

---

## §5. Évolution du SLA

Le présent SLA peut être révisé annuellement à la date anniversaire
du contrat. Toute modification fait l'objet d'un préavis écrit de
**60 jours**. À défaut d'acceptation explicite du Client, l'ancien
SLA reste en vigueur jusqu'au renouvellement contractuel.

---

## Annexe A — Conditions de Référence matérielles

Reproduit § 1.5 du document `EFE_P1_SURFACE_ARCHITECTURE.md` (URL :
`https://docs.efe.ai/architecture/p1#1-5`) :

### A.1 Hardware

| Paramètre | Valeur contractuelle |
|-----------|----------------------|
| GPU | NVIDIA T4 (16 GiB VRAM) |
| CPU host | 4 vCPU Intel Cascade Lake / AMD Milan ou plus récent |
| RAM host | 16 GiB |
| Réplicas | 3 minimum |
| Charge | 0 → 200 RPS sustained (tolérance pic à 220 RPS, 110 % nominal) |
| Bundle embedder | MiniLM-L12-v2 (D16), bundle officiel signé cosign |
| Mode | Mode-1 uniquement (zero-shot) |
| Network | DC-local p99 < 1 ms RTT host↔host |
| Kernel Linux | ≥ 5.15 |

### A.2 Tokens

- Entrée : ≤ 512 tokens / Requête.
- Sortie : verdict + métadonnées, ≤ 10 KiB sérialisé.

### A.3 Connectivité

- mTLS TLS 1.3 obligatoire.
- Keep-alive recommandé (handshake initial exclu du budget §3.1
  cf. §1.4 de l'architecture, gated séparément par `SLO-PC-mTLS-INIT`).

### A.4 `host_fingerprint_sha256` baseline

Le `host_fingerprint_sha256` baseline pour cet engagement est :

```
<PLACEHOLDER — à remplir par efe-attest sign-baseline lors du provisioning>
```

Tout changement matériel (kernel upgrade, GPU driver, bundle
remplacé, …) ré-calcule le fingerprint. Une dérive non documentée
suspend §3.1 jusqu'à re-baseline (cf. § 3.5(a)).

---

## Annexe B — Format du Rapport SLA mensuel

Schéma JSON (extrait) :

```json
{
  "tenant_id": "<UUID>",
  "month": "2026-04",
  "host_fingerprint_sha256": "0a3b...c91f",
  "measurements": {
    "p50_ms": 93.4,
    "p99_ms": 187.2,
    "p999_ms": 312.5,
    "burn_rate_30d_ratio": 0.34,
    "burn_rate_1h_avg": 0.18,
    "burn_rate_6h_avg": 0.22,
    "requests_total": 518000000,
    "requests_above_200ms": 173000
  },
  "incidents": [
    {
      "id": "INCIDENT-2026-04-12-01",
      "duration_min": 18,
      "rca_url": "https://status.efe.ai/incidents/INCIDENT-2026-04-12-01",
      "credit_applied": false
    }
  ],
  "signed_by": "efe-attest-v0.2.0",
  "signature_hmac_sha256": "<base64>",
  "audit_chain_head": "<sha256-of-D14bis-tip>"
}
```

---

## Annexe C — Mesure de Disponibilité (§2)

> Hors scope P1, mais le SLA complet inclut C-12. Modalités précisées
> séparément.

---

## Annexe D — Force majeure

Aux fins du présent Accord, sont considérés comme cas de force
majeure : catastrophes naturelles, guerres, attaques cyberterroristes
documentées, pannes de fournisseurs upstream (DNS root, opérateurs
fibre transcontinentaux), décisions d'autorités publiques. La partie
invoquant la force majeure notifie l'autre dans les 48 h, preuves
documentaires à l'appui.

---

## Annexe E — Procédure de contestation par tiers (§3.6)

1. Le Client notifie au Fournisseur sa volonté de contester par
   lettre recommandée + email.
2. Le Fournisseur dispose de **5 jours ouvrables** pour fournir au
   tiers désigné l'accès en lecture-seule à :
   (a) l'endpoint Prometheus `/metrics` historique sur la fenêtre,
   (b) le `bench_baselines/<version>/baseline.json` signé cosign,
   (c) les Rapports SLA HMAC-signés de la période,
   (d) le ledger `host_fingerprint` audit chain (Annexe A § A.4).
3. Le tiers exécute `efe-attest verify-sla` (ou équivalent
   indépendant) et produit son attestation sous **30 jours**.
4. La décision du tiers lie les deux parties.

---

## Signatures

| Partie | Représentant | Date | Signature |
|--------|--------------|------|-----------|
| Fournisseur (EFE) | | | |
| Client | | | |
