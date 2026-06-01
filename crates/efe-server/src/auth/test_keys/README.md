# `test_keys/` — fixtures de TEST (throwaway)

> ⚠️ **Clés de TEST jetables. Aucune valeur de production. Ne protègent rien.**

Ce dossier contient une paire RSA-2048 **déterministe et publique**, utilisée
exclusivement par le **test unitaire OIDC** (`../oidc.rs`) qui :

1. signe un JWT avec la clé privée (`rsa_2048.pem`) ;
2. le vérifie via le modulus public (`rsa_2048.n.b64url`, partie **publique**).

| Fichier | Rôle |
|---|---|
| `rsa_2048.pem` | clé **privée** RSA-2048 — fixture de test uniquement |
| `rsa_2048.n.b64url` | modulus public `n` (base64url) — non secret |

## Pourquoi c'est committé (et sans danger)

- Chargée par **`include_str!`** (compile-time) dans `oidc.rs` → sans le fichier,
  **le crate ne compile pas**. La générer au runtime n'est donc pas possible sans
  refactor (build script). C'est une **fixture de test standard** (les libs JWT/JOSE
  committent leurs clés de test de la même façon).
- **Throwaway** : générée une fois pour ce test, ne signe **aucun** jeton réel,
  ne déverrouille **rien**.
- Allowlistée dans **`/.gitleaks.toml`** (par chemin) pour éviter les faux positifs
  des scanners de secrets — l'allowlist ne masque **aucun** secret réel.

## Si une politique « aucune clé en git » est requise

Refactorer `oidc.rs` pour **générer une paire déterministe au build** (`build.rs`
+ crate `rsa`) au lieu de `include_str!`, puis supprimer ces fichiers et leur
entrée d'allowlist. (Plus de travail ; non requis tant que la fixture reste un
test throwaway local.)
