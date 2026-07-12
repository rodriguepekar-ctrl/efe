# Couche 2 — Le modèle de confiance du reçu R4 (frontière servie)

> Document destiné à l'évaluation externe (partenaire/investisseur) · 2026-07-12 · Couche 2 (M0–M4).
> Objet : énoncer **exactement** ce que le reçu de la frontière `/v1/llm/*` prouve — et, tout aussi
> important, ce qu'il **ne prouve pas**. La crédibilité tient à la **borne**, pas à la promesse.

---

## 0. Posture

La thèse EFE est la **souveraineté** : l'inférence tourne **dans la frontière du client**, les données ne
sortent pas. La Couche 2 ajoute une capacité **délibérément différente** : coordonner des modèles
**frontière** (Claude / Gemini / GPT), qui sont des **tiers externes payants**. C'est un **escape-hatch**,
pas le chemin souverain. Ce document borne la confiance qu'on peut y placer, pour qu'aucune formulation
publique ne dépasse ce que le code tient.

Une règle traverse tout : **on préfère un énoncé étroit et vrai à un énoncé large et fragile.**

---

## 1. Ce que le reçu R4 **EST**

Chaque mission frontière produit un reçu à deux racines (`provenance_root` d'essaim + `n17_root`
forensique), scellé sous une **clé HMAC dérivée par tenant** (HKDF-SHA256, master server-side). Ce reçu :

1. **est tamper-evident après scellement** — muter n'importe quel record casse la chaîne : `verify`
   renvoie `verified=false` avec la faute **localisée** (`first_break`, N17 **et** essaim) ;
2. **prouve la complétude de l'egress** (l'apport central de M2) — un appel LLM déclaré mais **manquant**
   laisse un nœud `Missing`/`Unlinked` ⟹ `verified=false`. Le reçu ne prouve donc pas seulement
   l'intégrité de **ce qu'il contient**, mais l'**absence de trou** dans ce qui a été appelé ;
3. **rapporte la force du barreau** (`held_strength` : `any_remote`, `weakest=R4`) **à côté** de
   `verified`, jamais replié dedans — le lecteur sait que la mission a **quitté la frontière** ;
4. **est re-vérifiable par le détenteur de la clé** — quiconque détient la clé tenant peut recomputer les
   racines et confirmer, **hors ligne**, sans rejouer aucun provider.

---

## 2. Ce que le reçu R4 **N'EST PAS** (les bornes qui font la crédibilité)

- **Ce n'est PAS une preuve trustless par un tiers.** Le modèle est **côté-écriture / détenteur-de-clé** :
  l'opérateur qui détient la clé pourrait, en théorie, réécrire une histoire alternative **complète et
  cohérente** qui se re-vérifierait. La garantie protège contre l'**altération après coup** par un tiers
  sans la clé, **pas** contre un producteur malhonnête qui détient la clé. Il n'y a **aucune** re-dérivation
  indépendante par un vérificateur sans clé.
- **`verified` n'est PAS une garantie de correction.** La preuve établit l'**intégrité** et la
  **complétude** de ce qui a été fait ; **jamais** que la réponse est **juste**. Un mauvais résultat
  parfaitement scellé **reste un mauvais résultat**. On n'affirme **jamais** « small beats big » ni aucune
  supériorité de qualité déduite du reçu.
- **Le checker d'arité (`/v1/llm/audit`) n'est PAS un audit « tierce ».** Il re-dérive l'arité déclarée
  **sous la clé tenant** — c'est un contrôle **détenteur-de-clé**, hors-bande, complémentaire de
  l'invariant de sur-déclaration ; **pas** une vérification sans confiance par un tiers.

---

## 3. La frontière est un **escape-hatch non-souverain**

- **Opt-in, OFF par défaut** : compilée sous la feature `frontier-llm`, activée au runtime par
  `EFE_LLM_FRONTIER_ENABLED=true`. Un déploiement standard ne la sert pas.
- **Bornée** : allowlist d'egress **par tenant**, **fail-closed** (aucune policy ⟹ 403) ; authentification
  **OIDC obligatoire** (`aud=efe-llm`) ; plafond de coût par tenant ; admission (429).
- **NON-souveraine** : quand elle est activée, le **prompt et le contexte de l'utilisateur transitent EN
  CLAIR** vers Anthropic / OpenAI / Google. La provenance l'**atteste honnêtement** (`PiiStrategy::None`
  quand aucune redaction n'a eu lieu ; le hash porte sur le prompt **réellement envoyé**). Une redaction
  PII **locale** est disponible **en opt-in** (policy tenant `Redact`/`Strict`) ; **sans** elle, le prompt
  sort tel quel.

> **À n'activer que pour les tenants qui ont explicitement accepté que leurs données quittent le
> périmètre vers un tiers.** C'est le contraire du chemin essaim souverain — les deux coexistent, mais ne
> se confondent pas.

---

## 4. « Aucun credential ne fuit » ≠ « les données sont protégées »

Deux faits **vrais** qu'il ne faut **jamais** confondre :

- **(a)** Le reçu scellé **et** le corps HTTP ne fuitent **aucun** credential : la clé provider n'entre
  jamais dans le bundle ; le record pose un `error_kind` **constant** (jamais la `reason` keyée) ; le
  mapping d'erreur ne sérialise jamais l'URL keyée d'un provider ; et la clé Gemini transite désormais en
  en-tête (`x-goog-api-key`), plus en query-string.
- **(b)** **MAIS** le **prompt + contexte** partent en clair vers le tiers (§3). (a) est une propriété de
  **non-fuite de secret d'infrastructure** ; ce **n'est pas** une garantie de **confidentialité des
  données utilisateur**. Ne jamais présenter (a) comme (b).

---

## 5. Ce que le « gate production » couvre — et ne couvre **pas**

**Couvre (invariants de boot, refus de démarrer sinon)** : non-forgeabilité (clé HMAC master obligatoire,
sinon la re-preuve serait forgeable) ; authentification (OIDC `aud=efe-llm` obligatoire) ; durabilité
(store Postgres obligatoire, sinon les reçus sont perdus au restart) ; non-régression du chemin essaim.

**Ne couvre PAS (hors périmètre, différé ou documenté)** : pas de **SLA** ni de **reprise après sinistre**
au-delà de la durabilité Postgres ; l'**observabilité de coût** (compteurs `/metrics`) est fournie mais
sans alerting livré ; le **kill-switch runtime** est une **procédure** opérationnelle (scale-to-zero /
redeploy `flag=off`), pas une coupure à chaud ; **aucune certification de conformité**. Le plafond de coût
est un **plafond par requête**, pas un metering/quota cumulatif inter-requêtes.

---

## 6. Formulations — à bannir / à employer

| À BANNIR | À EMPLOYER |
|---|---|
| « re-preuve **trustless** / audit **tierce** / auditable par un **tiers** » | « re-vérifiable par le **détenteur de la clé**, hors ligne » |
| « `verified` prouve la qualité / la correction » | « `verified` prouve l'**intégrité + la complétude**, jamais la correction » |
| « frontière **prod-ready** » (nu) | « escape-hatch frontière **opt-in, non-souverain, borné** (allowlist + auth + cap) » |
| « aucune donnée ne fuit » | « aucun **credential** ne fuit ; le **prompt** sort en clair vers le tiers (attesté) » |
| « gate production » (impliquant SLA/DR/compliance) | « go/no-go : non-forgeabilité + auth + durabilité + non-régression ; SLA/DR/conformité **hors périmètre** » |

---

## 7. Portée d'audit (pour un évaluateur externe)

Un évaluateur détenteur de la clé tenant peut, **hors ligne** : recomputer `provenance_root`/`n17_root` et
confirmer `verified` ; constater qu'un appel retiré fait **échouer** la vérification (le test-gardien de
M2) ; lire `held_strength` et voir que la mission a **quitté** la frontière (`any_remote`, `weakest=R4`) ;
re-dériver l'arité déclarée via `/v1/llm/audit` et la comparer aux nœuds scellés. Il **ne peut pas** — et
on ne le prétend **pas** — établir sans la clé que le producteur n'a pas réécrit une histoire cohérente,
ni que la réponse est correcte. **C'est précisément cette honnêteté sur la borne qui rend le reste
crédible.**
