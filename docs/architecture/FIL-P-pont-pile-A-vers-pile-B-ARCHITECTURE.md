# FIL-P — LE PONT : brancher le reçu servi (pile A) sur le vérifieur auditable (pile B)

**Statut :** **P.0 → P.4 IMPLÉMENTÉS ET VÉRIFIÉS** (commits `8bceaf3`, `06c5235`, `e3c00e5`,
`c51c538`, `e8817b8`). P.5 reste optionnel. Ce document a été **amendé au réel** après
implémentation : les deux corrections d'architecture prises en cours de route sont signalées à
leur place (§4 pour `RunView`, §8/P.4 pour le placement de l'ancre) plutôt que réécrites en silence.
**Date :** 2026-08-05, amendé 2026-08-06 · **Base :** monorepo `efe` @ HEAD `02eccdb`.
**Auteur :** R. Pekar — avec Claude Code (Opus 5).
**Filiation :** issu du classement par potentiel (panel à 5 lentilles, 2026-08-05), dont les cinq juges
ont convergé indépendamment sur ce geste comme le plus fort rendement du dépôt.
**Règle cardinale :** NE JAMAIS SUR-VENDRE. Chaque affirmation sur l'existant est ancrée
`fichier:ligne` ; chaque niveau de garantie est énoncé séparément au §6, et aucun n'emprunte à un autre.

---

## 0 · Le but exact (et ce que le Pont EST / n'est PAS)

**Le constat de départ, en une phrase.** Le dépôt possède **deux moitiés de moat qui ne se touchent
jamais** : la **pile A** scelle les missions réellement servies avec un contenu riche (quel modèle a
réécrit quoi, quels microcentimes sont sortis, quel hop était local) mais **sa route de vérification ne
compare même pas la racine** ; la **pile B** possède un vérifieur dont l'arborescence de dépendances est
littéralement vide, la machinerie de re-preuve contre une racine publiée, et un format de bundle — mais
**n'a rien à vérifier** et **ne publie aucune racine**. Chacune porte exactement le défaut que l'autre
corrigerait.

**Le Pont EST :** (a) fermer d'abord le **défaut de racine** de la pile A, sans quoi tout le reste
scellerait un mensonge ; (b) une **dérivation pure et déterministe** d'un run de la pile A vers un DAG
de la pile B ; (c) un **bundle exportable** re-prouvable par le vérifieur auditable ; (d) une
**signature ed25519 de la racine** ; (e) un **ancrage publié HORS du bundle**.

**Le Pont N'EST PAS :** (a) une réécriture de l'une ou l'autre pile — les deux modèles restent intacts ;
(b) **un second producteur de vérité** — le DAG de la pile B est **dérivé** de la pile A, jamais
concurrent, jamais autoritatif ; (c) un système *trustless* — voir le tableau des niveaux au §6, dont
aucune ligne n'emprunte à la suivante ; (d) une preuve de **correction** — le reçu prouve ce qui a
tourné, jamais que c'est vrai ; (e) un changement du chemin servi par défaut — sans opt-in explicite,
**pas un octet ne bouge**.

---

## 1 · Vérité-terrain vérifiée (ce qui existe aujourd'hui)

### 1.1 · La pile A — ce qui scelle réellement les missions servies

Le record scellé (`efe-swarm-provenance/src/record.rs:370`) :

```rust
pub struct AgentCallRecord {
    pub record_id: Uuid,        pub swarm_id: SwarmId,     pub tenant_id: TenantId,
    pub agent_id: AgentId,      pub agent_role: AgentRole, pub agent_kind: AgentKind,
    pub input_hash: Blake3Hash, pub reads: Vec<CellRef>,
    pub output_hash: Blake3Hash, pub writes: Vec<CellRef>,
    pub variant: Option<u64>,   pub causal_parents: Vec<Uuid>,
    pub llm_provenance_id: Option<ProvenanceId>,
    pub wall_at: SystemTime,    pub seq: u64,
    pub chain_parent_hmac: Option<[u8; 32]>,
    pub payload_hash: [u8; 32], pub hmac: [u8; 32],
}
```

**Le fait structurel décisif pour le Pont** (`hash.rs`, module entier) : la pile A utilise **deux
familles de hash délibérément distinctes**, et **les deux sont sans clé** —

- **BLAKE3-256** pour le **contenu** (`input_hash`, `output_hash`), sur forme canonique : deux valeurs
  logiquement égales produisent le même hash (`blake3_value`, test
  `blake3_value_is_canonical_insensitive_to_key_order`) ;
- **SHA-256** pour le **`payload_hash`** d'un enregistrement, également sur forme canonique
  (`payload_hash_of`, test `payload_hash_of_is_key_order_invariant`).

**Seul le chaînage est HMAC.** Autrement dit : *n'importe qui peut déjà recalculer les empreintes de
contenu de la pile A ; le secret n'est requis que pour relier les maillons entre eux.* C'est ce qui rend
la dérivation vers la pile B possible **sans jamais exposer la clé**.

Autres éléments réutilisables, vérifiés :

| Brique | Emplacement | Rôle pour le Pont |
|---|---|---|
| `pub fn deterministic_order(records) -> Vec<AgentCallRecord>` | `replay.rs:82` | **l'ordre canonique** — à réutiliser, jamais à recopier |
| `fn check_dag(ordered) -> (bool, Option<Uuid>)` | `verify.rs:120` | la cohérence causale déjà vérifiée côté A |
| `StoredRun { root: Option<String>, n17_root: Option<String>, records, llm_records }` | `store.rs:24-36` | **la racine scellée EST persistée** |
| `ed25519-dalek` + `PinnedNode` / `NodeRegistry` / `cosign_tuple` / `verify_cosignature` | `cosign.rs` (776 l.) | la primitive asymétrique **est déjà une dépendance** |
| Discipline de domaine versionné pour les signatures | `cosign.rs:101` | le patron à réutiliser pour la signature de racine |

> **Nuance à ne pas confondre.** `cosign.rs` implémente la **co-signature R3 d'un participant sur sa
> propre computation** (« le nœud N, dont tu as pinné la clé, jure avoir produit X »). Ce **n'est pas**
> une signature de la **racine du run par l'orchestrateur**. La primitive est là ; l'usage est neuf.

### 1.2 · La pile B — ce qui vérifie, et ce qu'elle attend

Le modèle de nœud (`provenance-dag/src/node.rs:37`) : un enum à quatre variantes —
`AgentAct` / `Edge` / `Evidence` / `Human` — sérialisé en JSONL interne-taggé, **mais scellé sur un
encodage canonique BINAIRE** préfixé en longueur, à ordre de champ figé (`node.rs:5-8, :108-155`). Le
JSON ne sert qu'au stockage et à l'inspection humaine ; la re-preuve **recompose** l'encodage canonique
depuis les champs. C'est plus strict que la pile A, et c'est un point d'attention du mapping (§2c).

La causalité est **imposée** (`node.rs:176-183`) : les identifiants référencés par `refs()` doivent
exister **avant** le nœud qui les cite ; une référence pendante est rejetée à l'ajout comme à la
re-preuve. `Evidence` est la seule source pure (récupérée avant d'être consommée).

Le DAG scellé (`dag.rs`) :

```rust
pub struct SealedDag { /* records, */ chain: [u8; 32] /* = le manifest-root */ }
impl SealedDag {
    pub fn new() -> Self;
    pub fn from_records(records: Vec<Record>) -> Self;
    pub fn append(&mut self, key: &TenantKey, node: Node) -> Result<&Record, DagError>;  // :83
    pub fn manifest_root(&self) -> String;                                              // :107
    pub fn to_jsonl(&self) -> String;
    pub fn records_from_jsonl(s: &str) -> Result<Vec<Record>, DagError>;
}
pub fn reprove_dag(records: &[Record], key: &TenantKey) -> Verdict;                      // :167
pub fn verify_against_root(records: &[Record], key: &TenantKey,
                           expected_root: &str) -> Verdict;                              // :198
```

**Point capital et sous-exploité : `verify_against_root` existe déjà et est testé**
(`verify_against_published_root`, `dag.rs:386`) — la machinerie de comparaison à une racine **publiée**
est écrite. Il ne lui manque qu'une racine publiée à comparer.

La clé (`key.rs`) : `TenantKey::derive(root_secret, tenant_id)` avec le contexte versionné
`b"efe-prov-dag:v1:tenant:"`. **La pile B est donc, elle aussi, symétrique.** Sa doctrine, énoncée dans
son propre en-tête : *« Le client détient sa clé — lui seul (ou l'auditeur à qui il la confie) peut
re-prouver — la confiance n'exige PAS de faire confiance à l'éditeur. »* C'est exact, et c'est **plus
faible** que « n'importe qui vérifie ». Le §6 en tire toutes les conséquences.

Les convertisseurs (`provenance-capture/src/tap.rs`) : **quatre adaptateurs purs** —
`LlmCall` / `MessageEvent` / `ToolCall` / `HumanReview`, chacun exposant `to_node()`, avec hachage des
entrées (minimisation RGPD) et la convention d'identifiant `{cas}::{étape}` (`tap.rs:26`).

### 1.3 · Les quatre trous que le Pont bouche

**Trou 1 — la pile A ne compare pas sa racine.** `routes/verify.rs` appelle

```rust
let v = verify_run_offline(&stored.records, &stored.llm_records,
                           &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
```

puis renvoie `root: v.swarm.root_hex()` — **la racine recalculée** — et `verified: v.fully_proven`.
**`stored.root` n'est jamais lu.** L'asymétrie est frappante : la racine **N17 est passée** en paramètre
pour être comparée, la racine **essaim ne l'est pas**. Et la comparaison correcte existe déjà un fichier
plus loin, dans le replay R0 : `let root_reproduced = reproduced_root == stored.root;`
(`routes/replay.rs:79`). **Conséquence directe et testable : un tiers disposant de l'écriture en base et
d'aucune clé peut supprimer les derniers enregistrements d'un run et obtenir `verified: true`.**

**Trou 2 — la pile B n'a aucun producteur.** Une recherche des sites de construction
(`SealedDag::new()` suivi de `.append(`) hors tests ne retourne que le **doc-exemple** de
`reg-evidence/src/lib.rs:49-50` et des **tests** (`selector.rs:147`, `engine.rs:190`). Les quatre taps
sont des convertisseurs `to_node()` sans appelant réel.

**Trou 3 — aucun ancrage n'est produit.** `verify_against_root` attend un `expected_root` ; rien dans le
dépôt ne publie de racine. La racine de référence **voyage donc dans le bundle** qu'on tend à
l'auditeur, ce qui ne prouve la cohérence que contre soi-même.

**Trou 4 — aucune signature de racine.** `cosign.rs` signe des tuples de participants, jamais la racine
d'un run.

---

## 2 · Le problème central — quatre écarts de modèle

**(a) Les modèles de données sont disjoints.** La pile A produit un **record plat** portant des
empreintes et une parenté causale par `Uuid` ; la pile B attend un **enum typé à quatre variantes** dont
la causalité passe par des identifiants de chaîne. Rien ne se mappe automatiquement : il faut un
**mapping figé, documenté et testé par golden**.

**(b) La pile A n'a pas les sorties, seulement leurs empreintes.** Le champ `AgentAct.output` de la pile
B est une `String` documentée comme « la sortie attestée — gardée pour l'audit » (`node.rs:47-48`). La
pile A, par minimisation, ne détient que `output_hash`. **La dérivation porte donc l'empreinte, pas la
sortie** — et doit le dire explicitement, dans le champ comme dans la documentation servie, faute de
quoi un lecteur croirait le bundle plus riche qu'il n'est.

**(c) Identité, unicité et ordre.** La pile B rejette les identifiants dupliqués (`DagError::DuplicateId`)
et exige que les références précèdent leurs citants. La pile A dispose de trois candidats — `record_id`
(un `Uuid`, unique mais illisible), `agent_id` (lisible mais **pas garanti unique dans un run**) et
`seq` (unique et ordonnant). Il faut une convention qui soit **à la fois** unique, déterministe et
lisible par un auditeur humain.

**(d) Deux dérivations de clé, à ne surtout pas confondre.** La pile A dérive sa clé par HKDF côté
serveur (`state.derive_key(tenant)`) ; la pile B par `HMAC-SHA256` avec le contexte
`"efe-prov-dag:v1:tenant:"`. Ce sont deux fonctions différentes. **Décision : ne JAMAIS réutiliser les
octets de la clé A comme clé B.** La séparation de domaine est un invariant de sécurité, pas une
commodité : réutiliser une clé à travers deux schémas de scellement différents est précisément la classe
d'erreur qui casse les deux à la fois.

---

## 3 · La décision de conception

### 3.1 · Direction : la pile B est **dérivée** de la pile A, jamais concurrente

Deux options ont été pesées. **(i) La double écriture** — au moment du scellement de la mission,
alimenter aussi le DAG de la pile B. **(ii) La dérivation à la demande** — reconstruire le DAG de la
pile B depuis un `StoredRun` déjà scellé.

**Retenu : (ii), sans hésitation.** Quatre raisons, dans l'ordre de leur poids :

1. **L'invariant byte-identique tient gratuitement.** Le chemin de scellement de la pile A ne change pas
   d'un octet ; il n'y a littéralement rien à re-tester sur le chemin servi.
2. **Il n'y a jamais deux vérités.** Une double écriture crée deux sources qui peuvent diverger, et la
   divergence serait invisible. Une dérivation ne peut pas diverger : elle est une **fonction** de la
   pile A.
3. **La dérivation est idempotente et rejouable.** On peut la relancer sur d'anciens runs, comparer, et
   détecter une régression de mapping par un simple golden.
4. **L'échec est sans conséquence.** Si la dérivation échoue, le reçu de la pile A est intact et la
   mission a déjà réussi. Aucun chemin de production ne dépend du Pont.

> Le bundle de la pile B est donc une **vue** du reçu de la pile A. C'est une phrase à répéter : il
> n'ajoute aucune information, il **change le vérifieur** — et le vérifieur est précisément ce qui
> manquait.

### 3.2 · La dérivation : une fonction pure

```rust
pub fn nodes_from_run(stored: &StoredRun) -> Result<Vec<Node>, DeriveError>;
```

Contraintes non négociables :

- **Déterministe** : aucune horloge, aucun aléa, aucun identifiant frais. Même `StoredRun` ⟹ mêmes
  nœuds, à l'octet — donc **même manifest-root**.
- **Ordre canonique réutilisé, jamais recopié** : l'ordre est celui de
  `efe_swarm_provenance::replay::deterministic_order` (`replay.rs:82`), déjà testé indépendant de
  l'ordre d'entrée. *La leçon du garde de souveraineté dupliqué s'applique : une seule source de
  vérité.*
- **Topologiquement sûre** : les parents sont émis avant leurs enfants. Une référence pendante ou un
  cycle ⟹ `Err` (fail-closed), jamais un DAG partiel.

### 3.3 · Le mapping figé (le cœur du Pont)

| Pile A | Pile B | Décision et justification |
|---|---|---|
| `AgentCallRecord` (tout `agent_kind`) | `Node::AgentAct` | un record = un acte |
| `record_id` + `seq` + `agent_id` | `node_id` = `{run_id}::{seq:06}-{agent_id}` | **unique** (par `seq`), **ordonnant** (lexicographique), **lisible** ; respecte la convention `{cas}::{étape}` de `tap.rs:26` |
| `agent_role` | `agent_role` | tag stable, jamais renuméroté |
| `input_hash` (BLAKE3, sans clé) | `inputs_hash` (hex) | transport direct |
| `output_hash` (BLAKE3, sans clé) | `output` (hex, **préfixé `blake3:`**) | **c'est une EMPREINTE, pas la sortie.** Le préfixe rend l'ambiguïté impossible à l'œil |
| `agent_kind` (+ identité modèle si Mode-C) | `model` | ex. `mode-a`, `mode-c:claude-sonnet-4-6` |
| `CANONICAL_SCHEMA_VERSION` du record | `version` | figé, versionne le mapping |
| `wall_at: SystemTime` | `timestamp: u64` | **millisecondes depuis l'époque**, conversion figée ; un `SystemTime` antérieur à l'époque ⟹ `Err` |
| chaque `causal_parents[i]` | un `Node::Edge` | `from` = nœud du parent, `to` = nœud de l'enfant, `artifact_hash` = `output_hash` **du parent**, `artifact_type` = `agent_role` du parent |
| `llm_provenance_id` présent | un `Node::Evidence` cité par l'acte | `tool` = fournisseur, `query_hash` = empreinte du prompt, `content_hash` = empreinte de la sortie, `consumed_by_node` = l'acte ; l'acte le référence dans `evidence_refs` |
| *(rien)* | `Node::Human` | **délibérément laissé libre** — voir ci-dessous |

**La variante `Human` laissée libre est un choix d'architecture, pas un oubli.** Le DAG dérivé devient
l'endroit où la **supervision humaine** (article 14) vient s'attacher à une mission machine : la console
d'audit peut y ajouter ses revues avec la même primitive, dans le même bundle, sous le même sceau. C'est
aussi le seul chemin par lequel la décision humaine cesse d'être perdue au redémarrage — elle devient un
nœud d'un DAG persisté, pas un état de console.

### 3.4 · L'ancrage — ce qui doit sortir du bundle

Sans ancrage, `verify_against_root` n'a pas d'`expected_root` indépendant, et le bundle ne prouve sa
cohérence que contre lui-même. L'ancre est donc un enregistrement **volontairement minuscule**, publié
**ailleurs** que le bundle :

```rust
pub struct AnchorRecord {
    pub schema: u16,             // 1
    pub run_id: Uuid,
    pub tenant_id: String,
    pub manifest_root: String,   // hex — le seul champ qui compte
    pub node_count: u32,         // borne l'attaque par troncature
    pub derived_at: u64,         // millis, informatif
}
```

`node_count` n'est pas décoratif : il **borne la troncature de queue**, la classe d'attaque que la pile
B documente elle-même comme non détectable par un DAG nu (`dag.rs`, et le test
`suffix_truncation_reproves_but_published_root_catches_it`). Un bundle amputé re-prouve comme un DAG
authentique plus court ; comparer *et* le root *et* le compte l'attrape deux fois.

**Où publier ?** Le Pont ne tranche pas le support (journal append-only interne, horodatage RFC-3161,
journal de transparence). Il impose seulement le **contrat** : l'ancre est écrite **hors du bundle**,
elle est **append-only**, et le vérifieur la reçoit **par un canal distinct** de celui du bundle. Le
support est une question ouverte assumée (§11.2).

### 3.5 · La signature ed25519 de la racine

```rust
pub struct RootAttestation {
    pub schema: u16, pub run_id: Uuid, pub tenant_id: String,
    pub manifest_root: String, pub node_count: u32,
    pub key_id: String,        // quelle clé publique
    pub signed_at: u64,
    pub sig: String,           // ed25519, hex 64 octets
}
```

La signature couvre `blake3( DOMAINE_V1 ‖ len(run_id) ‖ run_id ‖ len(tenant) ‖ tenant ‖ root ‖ node_count )`,
sur le patron **exact** de `cosign_tuple` (`cosign.rs:90-101`), avec un **domaine versionné distinct** —
la discipline est déjà dans le code (`cosign.rs:101` : *« Le domaine versionné empêche qu'une
co-signature d'un usage soit re-interprétée comme l'autre »*), il suffit de la suivre.

**Ce que la signature achète, exactement, et rien de plus :** un tiers qui ne détient **que la clé
publique** peut confirmer que **cette racine-là** a bien été émise par le détenteur de la clé privée, et
qu'elle n'a pas été fabriquée après coup. Il ne peut toujours **pas** re-prouver le contenu sans la clé
tenant. C'est un gain réel et borné — le §6 le range à sa place.

### 3.6 · Le wiring servi (opt-in, fail-closed, jamais monté par défaut)

Une route de lecture seule, derrière un drapeau dédié :

```
GET /v1/swarm/bundle/:id     ->  { bundle_jsonl, manifest_root, node_count, attestation? }
```

Gardes, dans l'ordre : rôle `auditor` exigé ; drapeau absent ⟹ **501**, route non montée ; run inconnu
ou d'un autre tenant ⟹ **404** (isolation, comme `verify`) ; dérivation en échec ⟹ **500 sans bundle
partiel**. Aucun chemin ne renvoie un bundle incomplet : un bundle partiel est indistinguable d'un
bundle tronqué, ce qui est exactement l'attaque qu'on cherche à rendre visible.

---

## 4 · Contrats d'interface (signatures exactes proposées)

> **Correction apportée à l'implémentation de P.1 (2026-08-05).** Ce paragraphe annonçait
> `nodes_from_run(stored: &StoredRun)`. C'était une **erreur** : `StoredRun` vit dans
> `efe-swarm-gateway` (`store.rs:24`), pas dans `efe-swarm-provenance`. Le prendre en entrée aurait
> fait dépendre une crate de dérivation *pure* de tout le serveur HTTP (axum, tower, sqlx…), et
> inversé le sens des dépendances que le fil s'impose. Le pont définit donc sa propre vue minimale,
> `RunView`, que l'appelant construit. Les signatures ci-dessous sont **celles du code livré**.

```rust
// crates/provenance-bridge/src/  (crate NEUVE — voir §11.1)

/// La vue minimale d'un run scellé : des EMPRUNTS, aucune possession — la pureté par les types.
pub struct RunView<'a> {
    pub run_id: Uuid,
    pub tenant: &'a str,
    pub records: &'a [AgentCallRecord],
    pub llm_records: &'a [LlmProvenanceRecord],
    pub sealed_root: Option<&'a str>,   // sa présence ARME le contrôle de complétude
}

/// Dérivation pure : un run scellé de la pile A -> les nœuds de la pile B.
/// La clé est celle de la pile A : le pont re-prouve LUI-MÊME la complétude plutôt que de croire
/// son appelant sur parole.
pub fn nodes_from_run(view: &RunView<'_>, key: &[u8]) -> Result<Vec<Node>, DeriveError>;

pub enum DeriveError {
    PendingRef { node: String, missing: Uuid },          // référence causale pendante
    DuplicateNodeId(String),                             // violerait DagError::DuplicateId
    TimestampBeforeEpoch { record: Uuid },
    MissingRoot,                                         // complétude indécidable : on refuse
    RootMismatch { sealed: String, recomputed: Option<String> },  // `None` = chaîne rompue ou run vidé
}

// ── P.2 à P.4 — CODÉS (signatures réelles ; anchor_of vit dans le pont, AnchorRecord dans
//    la pile B : voir la correction au §8/P.4) ───────────────────────────────────────────
pub fn seal_bundle(nodes: Vec<Node>, key: &TenantKey)
    -> Result<(SealedDag, String /* manifest_root */), DagError>;
pub fn anchor_of(run_id: Uuid, tenant: &str, root: &str, n: u32, at: u64) -> AnchorRecord;
pub fn attest_root(anchor: &AnchorRecord, sk: &SigningKey, key_id: &str) -> RootAttestation;
pub fn verify_attestation(att: &RootAttestation, pk: &VerifyingKey) -> Result<(), AttestError>;
```

> **Note de dépendance (vérifiée au build).** `provenance-bridge` dépend de `efe-swarm-provenance`
> (pile A), `provenance-dag` (pile B), `efe-nano-sdk` et `uuid`. **Aucune des deux piles ne dépend du
> pont** : `cargo tree -p provenance-dag` ne le mentionne pas, et surtout
> `cargo tree -p verify-mini` reste **littéralement vide** — l'invariant qui rend la pile B
> auditable est préservé.

---

## 5 · Fail-closed et dégradation

| Condition | Comportement | Ce qui est dit |
|---|---|---|
| Drapeau absent | route non montée, **501** | rien ne change, défaut inchangé |
| `StoredRun.root` absent | `Err(MissingRoot)` — **on refuse de dériver** | on ne dérive jamais d'un run non scellé |
| Racine recalculée ≠ `stored.root` | `Err(RootMismatch)` — **aucun bundle produit** | le run est déjà compromis ; produire un bundle le blanchirait |
| Référence causale pendante / cycle | `Err(PendingRef)` | jamais de DAG partiel |
| Identifiant dupliqué | `Err(DuplicateNodeId)` | remonte le défaut de la pile A plutôt que de le masquer |
| Horodatage antérieur à l'époque | `Err(TimestampBeforeEpoch)` | pas de conversion silencieuse |
| Clé d'attestation absente | bundle produit **sans** `attestation` | le champ manque, il n'est jamais faux ni vide-mais-présent |
| Ancre non publiée | `verify_against_root` **non appelable** | le vérifieur dit « racine publiée absente », jamais « vérifié » |

**Jamais fail-open** : aucune erreur de dérivation ne produit un bundle. **Jamais silencieux** : chaque
refus porte sa cause typée. **Jamais de blanchiment** : un run qui ne se re-prouve pas dans la pile A ne
peut pas obtenir un bundle de la pile B — ce serait offrir un vernis de vérifiabilité à un reçu cassé,
c'est-à-dire l'exact inverse du but.

---

## 6 · Provenance et honnêteté — **le tableau des niveaux**

C'est le cœur intellectuel de ce document. Chaque niveau achète **une** chose précise, et **n'emprunte
rien** au suivant. Toute phrase publique doit pouvoir se ranger dans une ligne, et une seule.

| Niveau | Ce qu'un tiers peut conclure | Ce qu'il doit détenir | État |
|---|---|---|---|
| **N0** — aujourd'hui | **rien** : la route renvoie `verified:true` sur une chaîne tronquée | — | **le défaut** |
| **N1** — racine comparée *(P.0)* | le run n'a **pas été tronqué ni altéré** depuis son scellement | la clé tenant A | XS-S |
| **N2** — bundle dérivé *(P.1-P.2)* | idem N1, **mais re-prouvé par un vérifieur dont il peut lire toute la source** | la clé tenant B + le binaire | S-M |
| **N3** — racine signée *(P.3)* | **cette racine a bien été émise** par le détenteur de la clé, et n'a pas été fabriquée après coup | la clé **publique** seulement | S |
| **N4** — ancre publiée *(P.4)* | le run **existait à la date T** avec ce compte de nœuds ; une troncature est **détectable sans la clé** | l'ancre publiée | M |

**Ce que même N4 ne donne pas** — et qui doit être dit à chaque fois : la **correction**. Le reçu prouve
*ce qui a tourné*, jamais que le résultat est vrai. Un modèle honnête-mais-faux produit un bundle
parfaitement valide. L'intégrité n'est pas la vérité.

**Trois revendications interdites**, tant que les jalons correspondants ne sont pas faits :

1. « **Vérifiable par n'importe qui** » — faux jusqu'à N3, et même à N3 cela ne couvre que
   l'authenticité de la **racine**, pas la re-preuve du **contenu**, qui reste détenteur-de-clé.
2. « **La pile B rend le reçu trustless** » — faux : la pile B est **elle aussi symétrique**
   (`key.rs`). Ce qu'elle apporte est l'**auditabilité du vérifieur**, ce qui est différent et déjà
   précieux.
3. « **Le bundle contient les sorties** » — faux : il contient leurs **empreintes** (§2b), d'où le
   préfixe `blake3:` imposé au §3.3.

---

## 7 · Change-set précis (fichiers et fonctions)

| Jalon | Fichier | Changement |
|---|---|---|
| **P.0** | `efe-swarm-gateway/src/routes/verify.rs` | lire `stored.root` ; ajouter `root_matches: Option<bool>` à `VerifyResponse` ; **combiner au verdict** (`verified &&= root_matches != Some(false)`) |
| **P.0** | `efe-swarm-gateway/src/routes/verify.rs` | racine absente ⟹ `root_matches: None` + verdict inchangé — **aucune accusation rétroactive** sur un run antérieur au champ |
| **P.1** | `crates/provenance-bridge/` (neuf) | `nodes_from_run`, le mapping figé, `DeriveError`, golden du mapping |
| **P.1** | `efe-swarm-provenance/src/replay.rs` | *(rien)* — `deterministic_order` est déjà `pub` |
| **P.2** | `crates/provenance-bridge/src/lib.rs` | `seal_bundle` + export JSONL |
| **P.2** | `efe-swarm-gateway/src/routes/bundle.rs` (neuf) | `GET /v1/swarm/bundle/:id`, rôle `auditor`, drapeau, 501/404/500 |
| **P.2** | `efe-swarm-gateway/src/state.rs` | dérivation de la **clé B** depuis le maître, contexte propre — **jamais les octets de A** |
| **P.3** | `crates/provenance-bridge/src/attest.rs` (neuf) | `RootAttestation`, `attest_root`, `verify_attestation`, domaine versionné |
| **P.4** | `crates/provenance-bridge/src/anchor.rs` (neuf) | `AnchorRecord`, journal append-only, canal de publication |
| **P.4** | `provenance-verify` (CLI) | option `--anchor <fichier>` ⟹ appelle `verify_against_root` |
| **P.5** | `audit-console` | attacher les revues humaines au DAG dérivé (`Node::Human`) + persistance |

---

## 8 · Jalons (chacun sûr seul, mergeable seul, testable)

- **P.0 — Fermer le défaut de racine.** *La pile A seule ; aucun rapport avec la pile B, et pourtant
  c'est le prérequis absolu du Pont.* Comparer la racine recalculée à `stored.root`, exposer
  `root_matches`, l'intégrer au verdict. **DoD :** un run dont on supprime le dernier enregistrement en
  base renvoie désormais `verified: false` avec `root_matches: false` ; un run antérieur au champ
  renvoie `None` et **n'est pas accusé** ; les tests existants de `verify` restent verts. **Taille XS-S.**
  ✅ **FAIT** (`8bceaf3`) — 4 tests, dont `verify_detects_tail_truncation` qui épingle que
  `chain_intact` reste `true` : l'attaque est invisible à l'intégrité seule.
  *Tant que ce jalon n'est pas fait, tout le reste scellerait un mensonge — et le Pont donnerait à ce
  mensonge un vérifieur auditable, ce qui serait pire que de ne rien faire.*

- **P.1 — La dérivation pure.** `nodes_from_run` + le mapping figé + `DeriveError`. Aucune surface
  servie, aucun effet de bord. **DoD :** un golden du mapping (run fixé ⟹ nœuds attendus, à l'octet) ;
  déterminisme prouvé par double dérivation (y compris à entrée mélangée) ; les variantes d'erreur
  épinglées — **quatre par une entrée fautive construite, la cinquième (`DuplicateNodeId`) par la
  propriété qui la rend inatteignable** (cf. §9) ; **la re-preuve du bundle dérivé passe**
  (`reprove_dag`). **Taille S-M.** ✅ **FAIT** (`06c5235`, golden ajouté ensuite) — 43 tests.

- **P.2 — Le bundle servi.** Scellement, export JSONL, route opt-in, dérivation de la clé B.
  **DoD :** sans drapeau, **byte-identique** — aucune route montée, aucune réponse modifiée ; avec
  drapeau, un bundle exporté est re-prouvé **par le binaire `verify-mini`** ; falsifier un nœud du
  bundle casse la re-preuve, localisée. **Taille M.** ✅ **FAIT** (`e3c00e5`) — la route n'est pas
  montée sans opt-in (404, pas 501 : une surface désactivée ne trahit pas son existence).

- **P.3 — L'attestation de racine.** ed25519 sur le patron de `cosign_tuple`, domaine versionné distinct.
  **DoD :** une attestation vérifie avec la seule clé publique ; une racine modifiée, un `run_id`
  substitué ou un `node_count` gonflé la font échouer ; l'absence de clé produit un bundle **sans**
  champ d'attestation, jamais un champ vide. **Taille S** *(+ M pour la garde et la rotation des clés,
  qui est un chantier d'exploitation et non de cryptographie)*. ✅ **FAIT** (`c51c538`) — la clé
  publique **ne sort jamais** par la route (seule son empreinte) : la servir à côté du bundle
  rendrait la vérification circulaire.

- **P.4 — L'ancre publiée.** `AnchorRecord`, journal append-only hors bundle, option CLI.
  **DoD :** `verify_against_root` est appelé avec une ancre **reçue par un canal distinct** ; un bundle
  tronqué de son dernier nœud échoue **sur le root ET sur le compte** ; sans ancre, le vérifieur dit
  « racine publiée absente » et **jamais** « vérifié ». **Taille M.** ✅ **FAIT** (`e8817b8`).
  **Correction d'architecture assumée** : le change-set plaçait `anchor.rs` dans le pont, mais
  `provenance-verify` ne dépend que de `provenance-dag` — y loger l'ancre aurait inversé
  l'invariant « pont → piles ». Le **format et la vérification** vivent donc dans la pile B
  (`provenance-dag/anchor.rs`), **seule l'écriture** dans le pont. Trouvaille au passage : le CLI
  **sur-revendiquait** — sans référence, il imprimait « ✓ VÉRIFIÉ » alors qu'un bundle tronqué
  produit ce verdict ; corrigé en trois verdicts distincts.

- **P.5 — La supervision humaine attachée** *(optionnel, ouvre l'article 14).* La console attache ses
  revues au DAG dérivé via `Node::Human`, persistées. **DoD :** une revue survit au redémarrage ; elle
  est scellée dans le même bundle ; la falsifier casse la re-preuve. **Taille M.**

**Ordre imposé :** P.0 (indispensable) → P.1 (pur) → P.2 (servi) → P.3 (le saut de niveau) → P.4
(l'opposabilité) → P.5 (l'ouverture).

---

## 9 · Stratégie de test

- **Golden de mapping** (P.1) : un `StoredRun` figé en fixture ⟹ la liste de nœuds attendue, comparée à
  l'octet. C'est le test qui empêche une dérive silencieuse du mapping — la classe de régression la plus
  probable de tout le Pont.
- **Déterminisme** : dériver deux fois le même run ⟹ même `manifest_root`. Et dériver le même run avec
  les records **mélangés en entrée** ⟹ même racine (c'est ce que garantit `deterministic_order`, déjà
  testé côté A par `deterministic_order_is_input_order_independent`).
- **Fail-closed** : les **cinq** variantes de `DeriveError`. Quatre sont épinglées par un test qui
  construit l'entrée fautive (`RootMismatch`, `MissingRoot`, `TimestampBeforeEpoch`, `PendingRef`).
  La cinquième, `DuplicateNodeId`, est **inatteignable par construction** et il faut le dire plutôt
  que d'écrire un test contorsionné : l'identité d'un acte est `{run}::{seq:06}-{agent}`, or la
  chaîne de la pile A refuse un `seq` non strictement croissant (`BreakReason::NonMonotonicSeq`) —
  deux records de même `seq` cassent la racine, et la dérivation refuse avant d'y arriver. Elle
  reste en défense en profondeur ; le test
  `node_ids_are_injective_which_is_why_duplicates_are_unreachable` épingle la **propriété** qui la
  rend inatteignable, et sert d'alarme si elle disparaît.
- **Re-preuve croisée** (P.2, le test-roi) : sceller une mission dans la pile A, dériver, exporter, puis
  **re-prouver le bundle par `verify-mini`** — le vérifieur dont l'arborescence de dépendances est
  vide. Ce test est la démonstration entière du Pont. *Précision d'honnêteté : le test appelle
  `verify_mini::run`, c'est-à-dire la MÊME fonction que le binaire enveloppe, via une
  **dev-dépendance** (l'arbre normal reste propre des deux côtés). Il exerce donc la logique de
  vérification, pas l'analyse d'arguments ni la lecture de fichiers du binaire — celles-ci sont
  couvertes par les tests propres de `verify-mini`.*
- **Falsification** : muter un nœud du bundle ⟹ re-preuve cassée et **localisée** ; muter la racine
  attestée ⟹ signature invalide ; retirer le dernier nœud ⟹ échec **sur le root et sur le compte**.
- **Non-accusation rétroactive** (P.0) : un run scellé avant l'introduction du champ renvoie `None`,
  jamais `false`. *La leçon de B.3, reprise telle quelle.*
- **Isolation tenant** : dériver un run du tenant A avec la clé du tenant B ⟹ re-preuve refusée.
- **Non-régression** : la batterie des deux piles, exécutée isolément (flakiness de workspace connue).

---

## 10 · Modèle d'honnêteté (à énoncer avant tout usage externe)

- Le bundle de la pile B est une **vue dérivée** du reçu de la pile A. Il n'ajoute **aucune
  information** ; il change le **vérifieur**.
- Les deux piles sont **symétriques**. Le Pont ne les rend pas *trustless* ; il rend la **re-preuve
  délégable à un auditeur** avec un vérifieur dont il peut lire toute la source, et — à partir de P.3 —
  la **racine** vérifiable sans aucune clé secrète.
- Le bundle contient des **empreintes**, pas les sorties. Le préfixe `blake3:` le rend impossible à
  confondre.
- **Intégrité n'est pas correction.** Un modèle honnête-mais-faux produit un bundle valide.
- La **troncature de queue** n'est détectable qu'à partir de P.4, et seulement contre une ancre reçue
  **hors** du bundle. Avant P.4, ne jamais dire « inviolable ».
- Le compte de nœuds et la racine sont **deux gardes distincts** contre la même attaque ; citer les deux
  ou aucun.

---

## 11 · Risques et questions ouvertes

1. **Crate neuve ou module ?** Le Pont vit-il dans une crate `provenance-bridge` dédiée, ou dans un
   module de `efe-swarm-gateway` ? **Recommandé : une crate dédiée**, pour que le sens des dépendances
   reste strictement `pont → piles`, et pour qu'aucune des deux piles ne puisse être polluée. À
   revérifier au build.
2. **Le support de l'ancre.** Journal append-only interne (simple, mais l'éditeur reste l'auteur),
   horodatage RFC-3161 (opposable, dépendance externe), journal de transparence (le plus fort, le plus
   lourd). **Non tranché** — P.4 impose le contrat, pas le support.
3. **La garde des clés d'attestation.** La cryptographie est de taille S ; la **rotation** et la
   **révocation** sont de taille M et relèvent de l'exploitation. Un `key_id` est prévu dans
   `RootAttestation` pour ne pas se fermer la porte, mais aucune politique n'est décidée ici.
4. **Que faire des runs déjà scellés ?** La dérivation est rétroactive par construction. Faut-il la
   lancer en lot sur l'historique, ou seulement à la demande ? *Recommandé : à la demande d'abord* — un
   lot masquerait les échecs de mapping dans le volume.
5. **Le `model` des records Mode-A.** Un nœud interne n'a pas de modèle. Écrire `mode-a` est honnête
   mais pauvre ; écrire le nom du composant serait plus utile et plus fragile. **À trancher en P.1**,
   sur pièces.
6. **La pile B accepte-t-elle un DAG large ?** Une mission produit peu de nœuds (quelques dizaines) ;
   un run d'essaim peut en produire beaucoup plus. Aucune borne n'est documentée côté `SealedDag`. **À
   mesurer avant P.2**, et à borner explicitement si nécessaire.

---

## 12 · Definition of Done (le Pont entier)

- Sans drapeau : les réponses servies sont **byte-identiques** à HEAD `02eccdb`, à la seule exception du
  champ additif `root_matches` de P.0, qui vaut `None` sur tout run antérieur.
- Un run tronqué en base renvoie `verified: false` — **le défaut N0 est fermé**.
- Un bundle exporté est re-prouvé par **`verify-mini`**, binaire à arborescence de dépendances vide, et
  la falsification d'un nœud casse la re-preuve de façon localisée.
- Une attestation de racine vérifie **avec la seule clé publique** ; sans clé, le champ est **absent**.
- Une ancre publiée hors bundle rend la **troncature détectable sans la clé tenant**.
- La documentation servie range chaque revendication dans **une seule ligne** du tableau du §6, et
  aucune ne cite un niveau non atteint.

---

### Annexe — le chemin de la preuve, après le Pont

```
   MISSION SERVIE                          (pile A — inchangée)
        │  AgentCallRecord × N  ──HMAC tenant A──►  provenance_root  ──► StoredRun.root
        │
        ├─ P.0 ─►  GET /v1/swarm/verify/:id   compare root recalculé ≟ stored.root
        │                                      ⟹ une troncature devient VISIBLE
        ▼
   DÉRIVATION PURE                          (le Pont — nouveau)
        │  deterministic_order ─► mapping figé ─► Vec<Node>   [échec ⟹ AUCUN bundle]
        ▼
   BUNDLE SCELLÉ                            (pile B — inchangée)
        │  append(clé tenant B) × N  ──►  manifest_root  ──►  bundle.jsonl
        │
        ├─ P.3 ─►  RootAttestation (ed25519)     ─► vérifiable avec la SEULE clé publique
        └─ P.4 ─►  AnchorRecord {root, n}        ─► publié HORS du bundle
                                    │
                                    ▼
                          L'AUDITEUR, chez lui, hors ligne :
                          verify_against_root(bundle, clé_tenant_B, ancre.root)
                          avec un binaire dont il peut lire TOUTE la source.
```

Le reçu ne change pas de contenu : il change de **témoin**. C'est tout ce que le Pont fait — et c'est
précisément ce qui manquait.
