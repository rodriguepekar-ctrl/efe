# ADR-M27-003 : Cloudflare over AWS CloudFront + WAF

- **Status** : Accepted
- **Date** : 2026-05-11
- **Deciders** : Eng lead, SRE lead, Founder
- **Related** : EFE_M27_COMMERCIAL_EDGE_ARCHITECTURE.md § 6

## Contexte

M27 nécessite un edge layer global : CDN + WAF + DDoS protection +
bot management. Trois candidats : Cloudflare, AWS CloudFront + WAF,
Fastly.

## Options considérées

### Option 1 : Cloudflare Enterprise ✅

- **Pour** :
  - Argo Tunnel : pas d'IP publique origin → scan-immune.
  - Anycast DDoS L3/L4 + L7 inclus.
  - Bot Management Enterprise (fingerprinting + JS challenge).
  - mTLS authentication path simple (per-zone).
  - Workers + Page Shield disponibles si besoin futur.
  - Pricing négocié → ~$200-500/mo/zone à notre volume initial.
- **Contre** :
  - Vendor lock-in modéré.
  - Bot Mgmt nécessite Enterprise (Business OK pour CDN + WAF basique).

### Option 2 : AWS CloudFront + WAF

- **Pour** : intégration AWS-natif si on est AWS-first.
- **Contre** :
  - Pas d'équivalent Argo Tunnel : IP publique sur ALB requis (sauf
    PrivateLink + Global Accelerator, complexe).
  - WAF managed rules à la carte → coûts cumulés.
  - Bot Mgmt = AWS Bot Control = moins évolué que CF.

### Option 3 : Fastly

- **Pour** : VCL programmable, image optimization.
- **Contre** :
  - Pas de Argo-Tunnel-équivalent intégré.
  - Pricing plus opaque.
  - Adoption EU < Cloudflare.

## Décision

**Option 1 retenue : Cloudflare Enterprise**.

Justification :
1. **Argo Tunnel** est la feature différenciante : sans elle, origin
   IP est exposé et scan-able. Cloudflare est unique sur ce point.
2. **EU + Anycast** : Cloudflare opère depuis Frankfurt / Paris,
   meilleur RTT EU que CloudFront.
3. **Bot Management** : essentiel contre cost-amplification attacks
   (T2 du threat model § 4).

## Conséquences

### Positives
- IP origin non-publique. Zero attack surface direct.
- Bot Management Enterprise inclus.
- Terraform provider mature.
- Logpush vers Loki en standard.

### Négatives
- Engagement Cloudflare Enterprise (annuel, ~$5k+/an).
- Vendor lock-in (mitigation : Terraform abstrait l'essentiel,
  migration vers CloudFront possible si nécessaire).

### Neutres
- Tunnels k8s nécessitent un DaemonSet `cloudflared` (2 replicas HA).

## Pivot scenarios

Si Cloudflare introduit un breaking change incompatible :
- Migration vers Fastly avec Compute@Edge équivalent.
- Code in-cluster (Envoy + Redis) inchangé — seul l'edge L4/L7
  shifte.

## Révision

Revue fin Q3 — pricing négociation + features adoption.
