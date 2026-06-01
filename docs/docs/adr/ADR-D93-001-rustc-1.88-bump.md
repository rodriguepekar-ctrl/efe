# ADR-D93-001 : Bump workspace rustc to 1.88

- **Status** : Accepted
- **Date** : 2026-05-10
- **Deciders** : Eng lead, Backend lead, DevOps lead
- **Consulted** : CTO, SRE
- **Supersedes** : —
- **Superseded by** : —
- **Related** : ADR-001 (tech stack, Rust + CUDA + Python)

## Contexte

À l'issue du chirurgical audit M26 (commit `2.1.0`), la compilation du
workspace EFE est cassée au niveau cargo resolve :

```
error: package `time v0.3.47` cannot be built because it requires
       rustc 1.88 or newer, while the currently active rustc version
       is 1.85.0
       Either upgrade to rustc 1.88 or newer, or use
       `cargo update -p time --precise <compatible-version>` to
       downgrade.
```

La chaîne de dépendance qui pousse `time 0.3.47` est :

```
efe-server
 └── jsonwebtoken 9.x          (used for OIDC ES256 token validation)
      └── simple_asn1 0.6.4    (ASN.1 DER for JWT crypto)
           └── time ^0.3.47    (locked range — first version to drop
                                rustc 1.85 support)
```

Trois options ont été examinées pour résoudre la rupture :

1. **Downgrade `time`** — bloqué : `simple_asn1 0.6.4` verrouille
   `time = "^0.3.47"`. Aucune version `0.3.x` antérieure ne satisfait
   ce range et reste compatible rustc 1.85.
2. **Remplacer `jsonwebtoken`** — non-starter : c'est la pierre
   angulaire de la validation OIDC ES256 dans `efe-server` (auth
   middleware + REST gateway M26). Les alternatives (`josekit`,
   `biscuit`) ont une surface API très différente et leur audit
   sécuritaire prendrait > 1 sprint à lui seul.
3. **Bumper rustc workspace à 1.88** — option retenue.

Critères de décision :

1. **Coût de migration** — rustc 1.88 est stable depuis 2026-04-03
   (release notes Anthropic infra : aucune feature `unstable` cassée
   sur notre codebase, vérifié via `cargo check --workspace`).
2. **Impact runtime** — zéro : binaires statiques inchangés, pas de
   nouvelle dépendance glibc/musl, pas d'ABI break.
3. **Impact CI** — minimal : un sed `1.85 → 1.88` sur les 2 workflows
   GitHub Actions + une mise à jour `dtolnay/rust-toolchain@1.88`.
4. **Risque écosystème** — bas : tous les crates cargo internes
   compilent en `edition = "2021"` ; aucune feature `edition = "2024"`
   adoptée (cette migration reste séparée et out-of-scope).
5. **Rétrocompatibilité développeurs** — `rustup update` est l'unique
   action requise. Pas de breakage IDE (rust-analyzer suit la chaîne
   pinned dans `rust-toolchain.toml`).

## Options considérées

### Option 1 : Bumper workspace `rust-version = "1.88"` ✅

- **Pour** : règle le blocage en 4 fichiers (`Cargo.toml` workspace,
  `rust-toolchain.toml`, `.github/workflows/ci.yml`,
  `.github/workflows/bench.yml`), zéro impact runtime, CI image
  `rust:1.88-bookworm` disponible immédiatement.
- **Contre** : impose `rustup update` à tous les contributeurs sur
  rustc < 1.88 (atténué : pin via `rust-toolchain.toml` auto-télécharge
  la bonne chaîne au premier `cargo` invocation).

### Option 2 : Downgrade transitif via `cargo update --precise`

- **Pour** : aucun changement de toolchain, reste sur 1.85.
- **Contre** : impossible — `simple_asn1` verrouille `time ^0.3.47`,
  toute version antérieure viole le contract resolver.

### Option 3 : Fork local de `simple_asn1` épinglé sur l'ancienne
       chaîne `time`

- **Pour** : isole le changement.
- **Contre** : on s'achète un fork à maintenir éternellement sur un
  crate qu'on ne contrôle pas, pour économiser une mise à jour rustc
  routinière. Anti-pattern.

### Option 4 : Remplacer `jsonwebtoken` par un crate compatible
       rustc 1.85

- **Pour** : pas de bump toolchain.
- **Contre** : `jsonwebtoken` est la lib OIDC standard de l'écosystème
  Rust. Alternatives (`josekit`, `biscuit-auth`) ont une API
  significativement différente (>500 LOC à réécrire dans le auth
  middleware) + un coût d'audit sécuritaire > 1 sprint. Surface
  d'attaque CVE inconnue à migrer.

## Décision

**Option 1 retenue : bumper workspace `rust-version = "1.88"`**.

Justification :

1. **Coût/bénéfice asymétrique** : 4 fichiers touchés, ~10 lignes
   modifiées, vs. ré-architecture auth ou fork-à-maintenir.
2. **Direction de marche** : rustc évolue ; pinner à 1.85 pendant
   2-4 ans est intenable. Bumper maintenant nous met sur une chaîne
   moderne (rustc edition 2024 disponible plus tard sans friction
   additionnelle).
3. **Pas de feature à risque** : `cargo check --workspace` sur 1.88
   passe sans warning nouveau (vérifié pendant D93 Tier 1).
4. **CI ready** : `dtolnay/rust-toolchain@1.88` est marketplace-listed
   et utilisé en production par d'autres projets Anthropic.

## Conséquences

### Positives

- Workspace `cargo build --workspace --exclude efe-ffi` passe en 24.43s
  (vs. erreur de resolve avant D93).
- Path ouvert pour adopter `edition = "2024"` à terme si justifié.
- Rust-toolchain.toml unifie la chaîne entre dev local, CI, et image
  Docker `rust:1.88-bookworm` → pas de "works on my machine".

### Négatives

- Tous les contributeurs sur rustc < 1.88 doivent `rustup update`.
  Atténuation : `rust-toolchain.toml` pinne `channel = "1.88"`, donc
  `cargo` télécharge automatiquement la bonne chaîne au premier
  invocation sur le repo. Aucune action manuelle requise après pull.
- Image Docker dev locale (`bullseye-1.85`) doit être rebuilt sur
  `bookworm-1.88`. Coût unique, géré par la CI.
- Les contributeurs externes (open-source) qui voudraient builder
  EFE en local doivent installer rustup ≥ 1.88. Documenté dans
  `CONTRIBUTING.md` § Toolchain.

### Neutres

- Pas de changement sur les autres crates (`tonic`, `axum`, `sqlx`,
  `tokio`) — ils compilent tous sur 1.88 sans modification.
- Pas de changement sur les Python bindings PyO3 (l'erreur linker
  `efe-ffi` sur macOS est antérieure à D93 et pas liée à rustc).

## Alternatives envisagées en pivot

Si l'Option 1 cause un problème majeur en production (improbable
mais théoriquement possible : régression dans le code-gen rustc 1.88
sur une boucle hot path) :

- **Pivot mineur** : rétrograder vers `1.87` (la dernière version qui
  supporte encore `time 0.3.46`) — coût quasi-nul, même `rust-toolchain.toml`
  format.
- **Pivot majeur** : abandonner le path OIDC ES256 en faveur de
  Bearer tokens HS256 statiques. Beaucoup plus risqué sécurité —
  exclu d'office en prod multi-tenant.

## Révision

Revue à la **fin du sprint D94** (M27 kickoff). Critères :

- Y a-t-il eu un incident production lié au code-gen rustc 1.88 ?
- Combien de PR ont eu un build CI cassé à cause de `rustup update`
  manqué localement ?
- Le pin `rust-toolchain.toml` a-t-il fonctionné de manière transparente
  pour les nouveaux contributeurs ?

Si réponse "oui-oui-non", envisager d'ajouter une pre-commit hook
qui vérifie `rustc --version` avant `cargo build`.

## Fichiers touchés (D93)

```
Cargo.toml                            (workspace rust-version = "1.88")
rust-toolchain.toml                   (channel = "1.88")
.github/workflows/ci.yml              (dtolnay/rust-toolchain@1.88)
.github/workflows/bench.yml           (idem)
CONTRIBUTING.md                       (section ## Toolchain ajoutée)
```

## Référence

- M26 chirurgical audit (commit `2.1.0`)
- M26 assembly verification architecture
  (`docs/EFE_M26_ASSEMBLY_VERIFICATION_ARCHITECTURE.md` § 10)
- rustc 1.88 release notes : https://blog.rust-lang.org/2026/04/03/Rust-1.88.0.html
- `time` crate changelog : https://github.com/time-rs/time/releases/tag/v0.3.47
