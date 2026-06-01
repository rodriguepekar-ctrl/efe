# ADR-M30-006 : Failover régional manuel (pas automatique)

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M30_MULTIMODAL_ARCHITECTURE.md § 13

## Décision

Le failover régional EFE est **manuel**. Initié par un opérateur via
`POST /v1/region/failover` ou le binaire `efe-region failover --to=`.
Aucun watchdog ne flippe automatiquement.

## Justification

1. **Cas irréversibles** — `pg_promote()` reset les slots de
   replication logique. Si un faux positif déclenche un failover,
   on perd potentiellement quelques minutes de transactions et la
   topologie de réplication est cassée jusqu'à reconstruction
   manuelle. Un humain est requis.
2. **Surface de bug réduite** — un watchdog automatique est un
   service de plus à monitorer + audit + tester chaos. M30 préfère
   minimiser la surface.
3. **Compliance SOC 2** — chaque flip est attribué à un opérateur
   nommé dans la chaîne d'audit HMAC. L'auditeur peut tracer.
4. **MTTR acceptable** — la cible d'RTO est 15 min. Avec un
   opérateur on-call paginé via PagerDuty, on est à 8-12 min en
   pratique (cf. runbook RB_M30_FAILOVER_PLAYBOOK).

vs **failover automatique** : nécessiterait un consensus
multi-région (Raft, etcd) que la stack actuelle (PG, NATS, Redis)
ne supporte pas nativement. Reporté à M31.

## Conséquences

### Positives
- Pas de "split-brain" possible — il y a toujours exactement un
  opérateur qui flippe.
- Audit trail clair.

### Négatives
- RTO dépend de la rapidité de réponse on-call (humain).
- Pas de RTO < 5 min — incompatible avec certains clients
  exigeants (SLA enterprise > 99.99 %).

## Migration M31

M31 ajoutera un mode "consensus failover" via etcd : trois
nœuds-arbitre + quorum 2/3 pour autoriser un flip. Activable par
config tenant.
