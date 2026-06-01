# ADR-M32-006 : Collection slugs tenant-scoped `[a-z0-9_-]{1,32}`

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M32_VECTOR_SEARCH_ARCHITECTURE.md § 4

## Décision

Le nom de collection est validé contre `^[a-z0-9_-]{1,32}$` et
contraint à être unique par `(tenant_id, name)`. La clé naturelle
de l'API REST est le tuple `(tenant, name)`.

## Justification

1. **URL-safe** — collection name apparaît directement dans
   l'URL `/v1/vector/collections/{name}` ; pas d'encoding requis.
2. **Tenant isolation by construction** — impossible pour tenant
   A de référencer une collection de tenant B via name : le bearer
   contraint le tenant_id.
3. **Cap 32 chars** — limite SQL : `VARCHAR(32)` reste compatible
   avec un index B-tree compact + cache CPU.
4. **Pas d'UTF-8 / espaces** — évite les classes de bugs (encoding,
   tokenization SQL injection-adjacent, normalization NFC vs NFD).

## Conséquences

### Positives
- Pas de risque d'XSS via le slug.
- Pas de collision PG entre tenants (UNIQUE (tenant_id, name)).

### Négatives
- Migrations de l'extérieur (Pinecone → EFE) doivent renommer si
  les noms contiennent espaces / majuscules.
