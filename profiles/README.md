# Profils — M0 · le profil servi

Trois configurations nommées. Chacune déclare **tout sauf les secrets**.

## Pourquoi aucun secret ici

Un profil qui embarquerait `EFE_NANO_AUDIT_HMAC_MASTER` ferait dire au rapport
« provenance : SERVIE » alors que la clé serait **publiquement connue** — un
*vert malhonnête* de manuel. Les secrets s'injectent au déploiement :

```bash
export EFE_NANO_AUDIT_HMAC_MASTER=$(openssl rand -hex 32)   # ≥ 32 octets
```

Sans lui, le binaire **démarre quand même** et le rapport dit `DÉGRADÉE — repli
sur la clé de TEST PUBLIQUE`. C'est le comportement voulu : jamais de silence.

## Usage

```bash
set -a && . ./profiles/souverain.env && set +a
export EFE_NANO_AUDIT_HMAC_MASTER=$(openssl rand -hex 32)
cargo run -p efe-swarm-gateway -- --profile-report | jq '.capabilities[] | select(.capability=="provenance")'
```

## Vérification

`crates/efe-swarm-gateway/tests/profile_files.rs` compare chaque `.env` à son
`.expected.json`. Modifier un profil sans mettre à jour son attendu casse le test.
