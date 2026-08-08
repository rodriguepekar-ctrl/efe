# Couche 3 — Consensus latent au niveau méta-essaim (architecture, préparation complète)

> Document d'architecture · 2026-07-12 · Issu d'une reconnaissance chirurgicale (**6 lentilles** : moteur, agrégateur, certificat, câblage, transport/fédération, provenance) + **1 passe adverse soundness**. Prolonge l'arc Couche 2 ([M4 gate](./COUCHE2-M4-gate-production-ARCHITECTURE.md), [modèle de confiance](./COUCHE2-M4-modele-confiance.md)).
> Contexte : **dowry Kovant** (SLM-swarms procurement/compliance). Règle absolue : **ne jamais sur-vendre** — les docs actuelles le font déjà, et c'est le premier problème à corriger.

---

## 0. Verdict & principe

La Couche 3 est un **triptyque honnête** que l'audit doc-vs-code a mis à nu :

1. **Le moteur, la provenance et le transport sont RÉELS.** Le point-fixe couplé Krasnoselskii-Mann, la projection de sûreté fermement non-expansive (prouvée par proptest), la chaîne HMAC re-prouvable par deux vérifieurs qui se croisent, et une vraie ronde de consensus servie sur gRPC/mTLS — tout cela existe, compile, et passe. **C'est le dowry.**
2. **Les docs SUR-VENDENT sur deux axes.** (#1) « MetaConsensus par weighted vote + outlier detection » alors que le seul opérateur câblé est **`UniformMetaOp`** (moyenne uniforme, zéro détection d'outlier). (#2) « convergence prouvée (cf. `certification.rs`) » alors que `certification.rs` est un **stub `always-valid`** et que la garde d'hypothèse H1 du moteur est **dishonest-green** (ne peut jamais échouer).
3. **Rien n'est branché à un chemin servi honnête.** ~~Les participants servis sont des `MockParticipant` synthétiques (pas efe-core)~~ — **cette moitié est FERMÉE par M1** (2026-07-14, `c53a7f6` : le participant servi enveloppe un vrai `SwarmController` efe-core sous `core-integration`, fail-closed). Le **reste tient toujours**, re-vérifié le 2026-08-08 : `efe-server` n'a **aucune route `/consensus`** et n'appelle **jamais** `iterate()` (0 occurrence) ; la provenance re-prouvable **n'est pas sur le chemin servi** (`iterate_with_provenance` / `meta_provenance_root` : 0 occurrence côté serveur) ; streaming et fédération inter-org sont absents. **C'est M2**, et il n'est pas fait.

**Donc la Couche 3 n'est pas « livrer le moteur » — le moteur existe.** C'est :
- **(a) corriger l'honnêteté D'ABORD** (le plus urgent pour Kovant : des docs qui mentent sont une bombe de crédibilité devant un investisseur technique) ;
- **(b) câbler une ronde SERVIE** avec un **reçu re-vérifiable** et de **vrais participants** (efe-core, pas des mocks) ;
- **(c) cadrer honnêtement le périmètre** : mono-région, **mono-org**, moyenne uniforme, **AUCUNE robustesse byzantine**, **PAS de fédération**.

> **North star** : un consensus latent **servi, honnête, re-prouvable, mono-domaine**. L'agrégateur robuste, le certificat réel et la fédération sont des jalons ultérieurs — la fédération est un **axe de recherche**, jamais une capacité présentée comme existante.

---

## 1. Ce qui est RÉEL (l'actif à présenter tel quel)

| Actif | Preuve (fichier:ligne) | Propriété honnête |
|---|---|---|
| **Point-fixe couplé KM** | `efe-meta-swarm/src/iteration/coupled.rs:82` (`step_once`, pipeline 6 étapes lift→aggregate→project→pull→step) ; `:139` (`run`, arrêt tolérance 1e-4 / max_iters 1000) | Vrai moteur d'itération, parallélisé (rayon), résidu joint `√Σ‖Δ‖²` |
| **Ronde servie sur gRPC** | `efe-meta-swarm-server/src/controller/iterate.rs:39` (`run_iterate`) ; e2e 4 sous-process OS, `Converged`, bit-identique sur 3 reruns | Une **vraie** ronde mono-région sur le fil — pas du plumbing |
| **Projection de sûreté** | `safety/box_safety.rs:59` (clamp) ; **fermeté prouvée** par proptest 64 cas (Bauschke-Combettes 4.16, `:191`) ; **toujours** appliquée (`coupled.rs:96`) | **`BoxSafety` fermement non-expansive** — machine-vérifiée |
| **Agrégateur uniforme** | `operators/uniform.rs:37` (moyenne `(1/M)Σv_m`) ; `lipschitz_bound()=1.0` (`:60`) ; 8 proptests | **Non-expansif** (Lemme 3.1) — c'est même une `1/√M`-contraction en norme jointe |
| **Provenance re-prouvable** (le moat) | chaîne HMAC `efe-audit-core::chain_hmac` (crypto partagée nano/essaim/méta) ; `efe-meta-verify` (10 tests) + `verify-meta-mini` **zéro-dépendance** (9 tests + 4 cross-check) | Re-preuve **byte-identique** hors-ligne à travers les 3 étages ; **tamper-evident détenteur-de-clé** |
| **Transport mTLS** | `tls/config.rs:255` (`StrictSanVerifier` : WebPKI + allowlist SAN, TLS 1.3, 0-RTT off) ; rotation à chaud `tls/reload.rs` (`ArcSwap`, `notify`) | **Logique** de vérification fail-closed ; rotation lock-free authentique |

> Ces six actifs sont le socle défendable. Le CLI de vérification l'inscrit déjà honnêtement dans le code : `verify_meta.rs:124` imprime « **intégrité attestée — PAS la correction de la décision de consensus** ». C'est exactement le modèle R4 de la Couche 2 (détenteur-de-clé, pas trustless).

---

## 2. Les deux sur-ventes à TUER (honnêteté d'abord)

### 2.1 Sur-vente #1 — « weighted vote + outlier detection »

**Réalité** : le crate ne contient QUE deux opérateurs. `UniformMetaOp` (moyenne, réel) et `AttentionMetaOp` (`operators/attention.rs`) — ce dernier **feature-gated `attention-weighted` (off)**, se déclarant lui-même « Phase-1 stub, NOT validated against the convergence theorem », avec `lipschitz_bound() = 1 + 2‖q‖/τ` **> 1** (il **casse** la non-expansivité, car les poids softmax **dépendent des votes**). `grep outlier|trimmed|median|krum|byzantine` = **0**. Le `MetaOperatorConfig` (`config.rs:89`) est **parse-only** (jamais transformé en `Box<dyn MetaOperator>`), et le serveur **code en dur** `UniformMetaOp::new(meta_dim)` (`iterate.rs:651`).

### 2.2 Sur-vente #2 — « convergence prouvée (cf. certification.rs) »

**Réalité** (le plus grave, sévérité **blocker** honnêteté) :
- `certification.rs:49` : `BundleCertificate::Stub => true` **inconditionnel** — ⚠ **TOUJOURS VRAI** (re-vérifié 2026-08-08). Le certificat de *bundle* n'a pas bougé ; c'est le certificat **par participant** qui a été traité par M1.c. Ne pas confondre les deux.
- ~~`participants/certificate.rs:33` : `NonExpansiveCertificate::Stub => true` ; toutes les impls renvoient `Stub`~~ — ✅ **CORRIGÉ par M1.c** (`a8f1798`), mais **partiellement, et il faut dire jusqu'où** : `certificate.rs:66` rend désormais `Stub => !strict` (donc **rejeté** en strict), et le participant **local réel** rend un `Empirical` calculé par un canari adversarial (`participants/local.rs:245`). Rendent **encore `Stub`** : le **mock** (`mock.rs:157` — il n'a rien à certifier) et surtout le **`RemoteParticipant`** (`efe-meta-swarm-server/src/client/remote_participant.rs:400`), faute de certificat **sur le fil** — c'est le jalon **M4**, et c'est la raison pour laquelle le mode strict n'est PAS activé sur l'`Iterate` servi (il briquerait chaque ronde distante).
- ~~La **garde H1** du moteur est **dishonest-green** : le chemin d'erreur est **inatteignable**~~ — ✅ **FAUX DEPUIS M1.c**. `coupled.rs:152` appelle `cert.accepts(self.strict)` et rend `HypothesisCheckFailed{H1}` : la garde **peut** échouer, sur un `L̂ ≥ 1 − marge` comme sur un `Stub` en strict. Côté serveur elle est même posée **plus tôt** — un gate de **boot** qui refuse de démarrer le participant (`h1_boot_gate`), donc avant qu'un seul RPC ne soit servi.
- La variante signée `LocalSignature` **ignore totalement** le champ `signature` Ed25519 (`certification.rs:50`, jamais vérifié) et **fait confiance à un Lipschitz auto-déclaré**. Pire, son seuil est `< 1.0` **strict** — il **rejetterait** `UniformMetaOp` et `BoxSafety` (L=1.0 exact), c'est-à-dire **exactement les opérateurs qui SONT non-expansifs**.
- La crate **`efe-prove`** citée comme source du Lipschitz+signature (`certification.rs:7`) **n'existe pas** dans le workspace — ⚠ **TOUJOURS VRAI** (re-vérifié 2026-08-08, 0 crate de ce nom). Toute la « Phase-3 » référence un composant fantôme.
- **Ce qui EST prouvé** : la fermeté de `BoxSafety` (proptest) + la borne fermée L=1 d'`UniformMetaOp`. C'est **testé en CI**, ce n'est **pas** un certificat runtime.

### 2.3 Résidus d'honnêteté (annexes mais réels)

- `PolyhedronSafety::project` **n'est pas** la projection métrique (il l'admet : « NOT Dykstra… not necessarily the metric projection », `polyhedron.rs:89`) ⟹ **non garanti non-expansif**. `LearnedSafety` délègue à une closure « responsable » (contrat non imposé), `contains()` renvoie toujours `false`. **Seul `BoxSafety` est certifié.**
- La provenance **ne scelle NI l'identité de l'opérateur** (Uniform vs Attention), NI `alphas/gamma`, NI l'identité des participants (seulement des hashes **positionnels**). Le « replay » revendiqué n'est pas re-exécutable depuis le bundle (seules les **sorties** `z̃` sont scellées, pas les entrées brutes).

### 2.4 Le tableau des phrases à BANNIR / REMPLACER (pour Kovant)

| BANNIR | REMPLACER par |
|---|---|
| « weighted vote + outlier detection » | « **agrégation par moyenne uniforme non-expansive** (`1/√M`-contractante, prouvée par proptest) » |
| « convergence prouvée (cf. certification.rs) » | « la **non-expansivité est property-testée** (projection Box fermement non-expansive, Bauschke-Combettes 4.16 ; agrégateur uniforme L=1) ; les certificats sont des **hooks Stub**, aucun certificat cryptographique n'est encore émis » |
| « consensus tolérant aux fautes / byzantine » | « consensus **contenu** dans la boîte de sûreté déclarée ; **aucune robustesse byzantine** aujourd'hui » |
| « transport mTLS fail-closed » | « **logique** de vérification fail-closed ; activation **opt-in**, défaut en clair (à durcir) » |
| « fédéré multi-région / inter-org » | « **N participants co-localisés sous un opérateur unique** ; la fédération est un axe de recherche, 0 code » |
| « re-preuve / replay du consensus » | « **re-preuve d'intégrité** du transcript (détenteur-de-clé), **PAS** de la correction ni ré-exécution » |

---

## 3. Le chemin servi honnête (le livrable central)

Comme la Couche 2 (moteur réel non servi → surface servie re-prouvable), le cœur de la Couche 3 est de **câbler la ronde servie à un reçu re-vérifiable et à de vrais participants**.

**Le seam existe déjà, entièrement** : `provenance.rs:67` `iterate_with_provenance(key_source, tenant) → (MetaSwarmResult, MetaProvenance)` scelle chaque pas via un `HmacProvenanceSink` (observateur **non-interférent**, clé tenant dérivée puis **droppée** à `finish()`). Le proto `IterateResponse` (`messages.proto:198`) a un **slot `reserved 8..99` libre** pour `meta_provenance_root`. Il **manque uniquement le câblage** :

1. **`run_iterate` doit passer par `iterate_with_provenance`** (aujourd'hui il appelle `iterate()` **sans observateur** — `iterate.rs:87`) ; activer la feature `provenance` dans `efe-meta-swarm-server` (aujourd'hui non activée → le code de scellement **n'est même pas compilé**).
2. **Renvoyer `meta_provenance_root`** dans `IterateResponse` (slot 8) + exposer le **bundle** (`meta_steps.jsonl` + `meta_manifest.json`) via un `GetConsensusReceipt`. La **clé tenant vient du responsable** (`HmacKeySource::derive`), **jamais du fil**.
3. **Une route `/consensus`** côté efe-server (ou une mission gateway) : `efe-server` n'expose aujourd'hui que `health_probe` (`services/meta_swarm/mod.rs`), gaté `EFE_META_SWARM_ENABLED=false`, **aucun site d'appel** de `meta_swarm_client()`, **aucune route**. L'analogue exact de `/v1/llm`.
4. **De VRAIS participants** (§ le plus lourd) — ✅ **LIVRÉ par M1** (`c53a7f6`, `b9a8fce`, `665ead0`). *Constat d'origine, conservé pour la traçabilité* : « le binaire participant enveloppe `MockParticipant::with_contraction` ; le pont réel `LocalParticipant↔efe-core` n'est référencé **nulle part** dans le serveur ». **Aujourd'hui** (re-vérifié 2026-08-08) : `Engine::EfeCore` servi sous `core-integration`, topologie **fixe par défaut** (Φ continu par construction), canari H1 adversarial en **gate de boot** fail-closed, et refus de servir un mock à la place d'un bloc `controller` déclaré. **Le garde-fou reste entier** : ce sont de vrais **essaims latents efe-core**, jamais de « vrais SLM » — aucun poids de LM ni tokenizer sur le chemin du pas.

**Modèle de confiance à graver** (identique R4 Couche 2) : le reçu est **tamper-evident pour le détenteur de la clé tenant**, **PAS trustless** ; il atteste **l'intégrité du transcript, PAS la correction du consensus**.

---

## 4. L'agrégateur honnête (jalon « robuste »)

La question « peut-on faire un VRAI weighted-vote + outlier detection en restant non-expansif ? » a une **réponse mathématique tranchée** :

| Agrégateur | Non-expansif ? | Verdict |
|---|---|---|
| **Poids convexes FIXES** (`Σλ=1, λ≥0`, state-independent, ex. réputation/config) | **OUI** : `‖Σλ_m(a_m−b_m)‖ ≤ ‖λ‖₂·‖Δ‖₂ ≤ ‖Δ‖₂` | le **vrai** « weighted vote » honnête |
| **Médiane coordonnée-par-coordonnée** | **OUI** : chaque coordonnée de sortie est une statistique d'ordre **1-Lipschitz** | la **vraie** robustesse aux outliers |
| **Trimmed-mean coordonnée, fraction FIXE** | **OUI** (même argument) | robustesse aux outliers, non-expansif |
| Softmax / attention (poids **dépendant des entrées**) | **NON** : `L = 1+2‖q‖/τ > 1` | **rejeté** (ce que fait `AttentionMetaOp`) |
| Krum / médiane géométrique (Weiszfeld) | **NON** : sélection selon les entrées | **rejeté** (non-expansif faux en général) |

**Livrable** : deux opérateurs **prouvablement non-expansifs**, avec proptests miroir de ceux d'`UniformMetaOp` — **(A)** `FixedWeightMetaOp{poids convexes}` = le vrai « weighted vote » ; **(B)** `CoordinateMedianMetaOp` (ou `TrimmedMeanMetaOp` fraction fixe) = la vraie robustesse. **Rejeter explicitement** softmax/attention et Krum comme non déployables.

**Le gate qui manque** (correctif de ~10 lignes, à fort ROI honnêteté, `AGG-3`/`ADV-5`) : `build()` **ne lit jamais** `meta_op.lipschitz_bound()`. Ajouter un contrôle **fail-closed** : `lipschitz_bound() > 1+ε ⟹ Err(ExpansiveMetaOperator{bound})`. Cela **transforme** l'usage silencieux d'un opérateur expansif en erreur, et rend la garde H1 capable d'échouer.

> **Honnêteté (mémoire HB.4)** : la synthèse par médiane/trimmed **n'a pas « payé »** sur un benchmark de correction (phi4×math, erreurs asymétriques). C'est un **contexte différent** (synthèse de réponses ≠ consensus latent) : la propriété de **non-expansivité de la médiane coordonnée reste vraie**, mais ne **jamais** re-vendre l'agrégation robuste comme un **gain de correction** — seulement comme **robustesse-adversaire**.

---

## 5. Le certificat réel (jalon « prouvé »)

Aujourd'hui la provenance scelle `residual`/`safety_delta` (**traces** du comportement) mais **rien** sur la propriété mathématique. Un `NonExpansivenessCertificate` **honnête et re-vérifiable** :

```
NonExpansivenessCertificate {
    operator_id,                       // Uniform / FixedWeight / CoordinateMedian / Box / …
    config_hash,                       // SHA-256 des paramètres (cf. SafetySet::hash, aujourd'hui [0;32])
    claimed_L: f64,                    // borne de Lipschitz revendiquée
    method: ClosedForm | Empirical{n_samples, seed} | Unproven,
}
```

- **`ClosedForm`** : re-calculable par le vérifieur (norme spectrale pour un opérateur linéaire, `linear.rs:139` ; L=1 pour Uniform/Box/médiane). **Prouvé**.
- **`Empirical`** : re-échantillonnable (borné, marqué **non-preuve**).
- **`Unproven`** : **refusé** en mode strict.
- **Seuil corrigé** : **`≤ 1`** pour non-expansif (`< 1` pour contraction) — l'actuel `< 1` strict rejetterait à tort les opérateurs L=1.
- **Scellé** dans le manifeste de provenance (à côté du root) et **re-contrôlé** par les 2 vérifieurs. **Ed25519-sur-root** (aujourd'hui `Stub`) permettrait une **attestation publique** (un tiers vérifie l'intégrité **sans** la clé tenant) — le seul pas honnête vers le « trustless » partiel.

Cela **remplace** la garde no-op par un contrôle réel, et rend la revendication **auditable côté Kovant sans sur-vendre** : on atteste une **borne vérifiable**, jamais une « preuve de convergence ».

---

## 6. Soundness & durcissement (jalon « sécurité »)

La passe adverse a établi que la ronde est **contenue** (projection méta inévitable) mais offre **zéro robustesse byzantine** et un transport **fail-open par défaut**.

| # | Trou (sévérité) | Correctif / limite à documenter |
|---|---|---|
| `ADV-1` (high) | **Zéro robustesse byzantine** : un participant pilote la moyenne (états non bornés côté contrôleur), la boîte ne donne que la **contenance** | Documenter la limite honnête ; livrer `CoordinateMedianMetaOp` (§4) + valider les états retournés (`ADV-2`) + boîte de sûreté **réaliste** (le défaut `radius=10` rend la contenance **vacante**) |
| `ADV-2` (high) | Le contrôleur **ne valide pas** les états participants (`proto_to_tensor` sans borne, sans NaN/Inf, `remote_participant.rs`) | **Re-projeter** sur l'ensemble déclaré + rejeter NaN/Inf/dim (fail-closed), ou documenter « états untrusted, hors garantie » |
| `ADV-3` (high) | **mTLS fail-OPEN par défaut** : `TLS_REQUIRED=false`, `TLS_MUTUAL=false`, bind `0.0.0.0` (`config/controller.rs:90,104`) | **Basculer les défauts prod** (`required=mutual=true`) OU refuser de servir sur bind non-loopback sans TLS |
| `ADV-4` (high) | **Aucune authz applicative** : `MtlsAuthorizer`=Deny-stub, `AllowAll` autorise tout, aucun intercepteur ⟹ tout pair handshaké peut `ReloadConfig`/`Reset` | Intercepteur tonic : SAN→scope, **admin-gate** `ReloadConfig`/`Reset` ; ou retirer la revendication de scope du proto |
| `ADV-8` (medium) | **`gamma/alpha` hors-fil** (`step_with_pill` droppe `_gamma/_alpha`) ⟹ mix KM silencieusement incohérent, non couvert par le reçu | Mettre `alpha/gamma` dans `StepRequest` + rejet sur écart, ou lier le hash du manifeste au handshake (`DT-A3-STEP-PARAMS-ON-WIRE`) |
| `ADV-7`/`C3-F3` (medium) | La provenance scelle des hashes **positionnels** — aucune identité participant ⟹ contribution forgée re-vérifiable mais **non attribuable** | Sceller une **identité stable** (empreinte SAN/cert) par vote, ou re-libeller « contribution en position i » |
| `ADV-9` (medium) | **Bypass TLS loopback** ⟹ aucune ronde jamais exercée sur transport chiffré (mTLS testé sur duplex mémoire, jamais un `Iterate`) | e2e `Iterate`-sur-mTLS **non-loopback** avant toute affirmation « consensus sécurisé » |

---

## 7. Fédération inter-org (jalon de RECHERCHE, explicitement différé)

`grep federation|inter-region|cross-org` = **0 code**. Ce n'est **pas** un problème de transport mais de **modèle de confiance**, cassé simultanément sur **5 axes** :

1. **PKI** — pas de CA commune entre orgs ⟹ besoin SPIFFE / trust-bundle multi-racines (aujourd'hui : `StrictSanVerifier` exige une **CA partagée** + allowlist SAN intra-cluster).
2. **Agrégation** — `UniformMetaOp` (moyenne plaine) est **trivialement pilotable** par un vote adverse ⟹ besoin de l'agrégateur robuste-byzantin (§4, non livré).
3. **Certificat** — on ne peut pas croire la revendication H1 d'une org distante ⟹ besoin du certificat signé vérifiable (§5, aujourd'hui `Stub`).
4. **Provenance** — le sceau est **HMAC symétrique / clé-tenant partagée** ⟹ chaque org pourrait **forger** le sceau de l'autre ⟹ besoin de **signatures asymétriques par-org** (non-répudiation).
5. **Confidentialité** — l'agrégat exige les votes **EN CLAIR** (`StepResponse.new_state` = tenseur brut) ⟹ le vecteur latent d'une org est **exposé** au coordinateur ⟹ besoin d'agrégation sécurisée / MPC.

**Position Kovant** : la fédération est un **axe de recherche co-construit**, jamais une capacité existante. Le périmètre réel aujourd'hui est **mono-domaine coopératif** : un opérateur unique détient la clé, choisit les participants, agrège, scelle. **Ne jamais prétendre au consensus trustless inter-org.**

---

## 8. Jalons & effort (roadmap Couche 3)

```
M0 ──► M1 ──► M2 ──► M3 ──► M4 ──► M5 ──► (M6)
honnête  vrais  servi+  agrég.  certif  durciss.  fédération
docs   particip. reçu   robuste  réel   sécurité  (recherche)
 S       L       M       L       L       M          XL/différé
```

| Jalon | But | Effort | Sortie clé (binaire) |
|---|---|---|---|
| **M0 — Honnêteté** | corriger les 2 sur-ventes ; retirer `AttentionSoftmax` du schéma public (ou le faire échouer explicitement) ; bannir « convergence prouvée » ; ajouter le **gate Lipschitz** `build()` (fail-closed) | **S** | ~~`grep` des phrases bannies = 0~~ — **insatisfaisable** : les tables BANNIR/REMPLACER contiennent nécessairement les phrases qu'elles interdisent. Critère exécutable : **toute occurrence est une MENTION** (table, ligne « sur-vente », négation), **jamais une affirmation** ; un opérateur expansif ⟹ `Err` |
| **M1 — Vrais participants** ✅ **LIVRÉ** (`8fcb3dc` → `665ead0`) | câbler `core-integration` dans le binaire participant (`LocalParticipant`↔efe-core au lieu de `MockParticipant`) | **L** | une ronde coordonne de vrais essaims **latents efe-core** — ~~« EFE/SLM »~~, jamais SLM. Détail & DoD re-vérifié : `COUCHE3-M1-vrais-participants-ARCHITECTURE.md` |
| **M2 — Servi + reçu** | `run_iterate` via `iterate_with_provenance` ; `meta_provenance_root` dans `IterateResponse` (slot 8) ; route `/consensus` efe-server | **M** | `POST /consensus` → reçu re-vérifiable par `verify-meta-mini` |
| **M3 — Agrégateur robuste** | `FixedWeightMetaOp` + `CoordinateMedianMetaOp`, proptests miroir, câblés par config | **L** | vrai « weighted vote » + robustesse aux outliers, **non-expansifs** |
| **M4 — Certificat réel** | `NonExpansivenessCertificate` scellé + re-vérifié ; Ed25519-sur-root ; garde H1 qui peut échouer | **L** | borne de Lipschitz **auditable** (plus de `Stub`) |
| **M5 — Durcissement** | mTLS fail-closed **par défaut** ; authz par-RPC ; validation des états ; `gamma/alpha` sur le fil ; e2e Iterate-sur-mTLS | **M** | déploiement fail-closed, ReloadConfig admin-gated |
| **(M6) — Fédération** | le problème 5-axes (§7) | **XL** | **DIFFÉRÉ — recherche**, pas un livrable produit |

**Chemin critique honnêteté** = M0 (avant tout pitch). **Chemin critique produit** = M1→M2 (un consensus servi sur de vrais essaims avec reçu).

---

## 9. Ce qu'on NE promet PAS

- **Pas de robustesse byzantine** : le consensus est **contenu** dans la boîte, pas **d'accord** ; un participant malveillant le pilote (jusqu'à M3 + validation des états).
- **Pas de « convergence prouvée »** : la non-expansivité est **property-testée** ; le certificat runtime est un `Stub` (jusqu'à M4).
- **Pas de trustless / inter-org** : tamper-evident **détenteur-de-clé**, mono-domaine coopératif. La fédération est un **axe de recherche**.
- **Pas de streaming** : `IterateStream`/`StepStream` = `unimplemented!` (PR#21).
- **Pas de « vrais SLM »** — jamais, et M1 n'y change rien : depuis `c53a7f6` le transport coordonne de vrais **essaims latents efe-core** (fusion Mode1 numérique), ce qui n'est pas la même chose. Templates stub, aucun poids de modèle ni tokenizer sur le chemin du pas.
- Le reçu **atteste l'intégrité, pas la correction** ; il scelle des **sorties**, pas de quoi ré-exécuter.

---

## 10. Corrections vs les docs (traçabilité)

| Revendication doc | Réalité (preuve) | Correctif |
|---|---|---|
| « weighted vote + outlier detection » | seul `UniformMetaOp` câblé ; attention = stub expansif ; config parse-only (`iterate.rs:651`, `config.rs:89`) | §2.1 + §4 (agrégateur honnête + gate Lipschitz) |
| « convergence prouvée (cf. certification.rs) » | **partiellement corrigé (M1.c)** — ✅ la garde H1 n'est **plus** dishonest-green (`cert.accepts(strict)`, `Empirical` adversarial, gate de boot) ; ⚠ **restent vrais** : `BundleCertificate::Stub => true` inconditionnel, `efe-prove` fantôme, signature jamais vérifiée, et `RemoteParticipant` toujours `Stub` (= pas de certificat **sur le fil**, jalon M4) | §2.2 + §5 (certificat réel) |
| « projection de sûreté prouvée » | **seul `BoxSafety`** l'est ; Polyhedron/Learned non (`polyhedron.rs:89`, `learned.rs`) | §2.3 (ne certifier que Box) |
| « consensus distribué SLM » | ✅ M1 livré : participants = vrais essaims **latents efe-core** (`Engine::EfeCore`) — donc « distribué » et « réel » sont acquis, **« SLM » ne l'est pas** (fusion numérique Mode1, templates stub) | dire « consensus distribué sur essaims latents efe-core » |
| « re-preuve / replay du consensus » | provenance non servie (`iterate()` sans observateur) ; scelle sorties + hashes positionnels, pas l'opérateur ni les entrées | §3 + M2 + `C3-F6`/`C3-F3` |
| « transport mTLS fail-closed » | logique fail-closed mais **défaut fail-open** ; loopback bypass ; authz stub (`config/controller.rs:90`, `remote_participant.rs:183`, `auth/mtls.rs:12`) | §6 (`ADV-3`/`ADV-4`/`ADV-9`) |
| « fédéré / inter-région » | 0 code ; modèle mono-CA (`grep = 0`) | §7 (recherche différée) |

*Préparation Couche 3 : le moteur mathématique est réel et le moat re-prouvable est solide — mais avant de servir ce consensus, il faut **arrêter de mentir** (agrégateur uniforme, pas weighted+outlier ; property-testé, pas certifié), **câbler de vrais participants et un reçu**, et **cadrer honnêtement** un système mono-org, non-byzantin, non-fédéré. Le dowry n'est pas « le consensus latent distribué fédéré prouvé » — c'est « un point-fixe couplé non-expansif, re-prouvable byte-identique, servi honnêtement sur un domaine unique ». Ça, c'est vrai, et c'est déjà rare.*
