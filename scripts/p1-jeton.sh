#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# P1.d — forger un JWT (ES256 ou RS256) pour appeler une passerelle en profil `production`.
#
#   scripts/p1-jeton.sh <clé-privée.pem> [tenant] [rôles] [aud] [ttl_s]
#
# ── Pourquoi ce script existe ─────────────────────────────────────────────────
#
#  Activer l'OIDC ferme AUSSI le chemin souverain : `POST /v1/swarm/run` sans
#  jeton répond 401. Un opérateur qui adopte le profil de production découvre
#  donc que TOUT exige un JWT — et jusqu'ici aucune page ne lui disait comment en
#  obtenir un. Une passerelle que personne ne sait appeler n'est pas déployée,
#  elle est installée.
#
# ── Ce que ce script N'EST PAS ────────────────────────────────────────────────
#
#  Ce n'est pas un IdP, et il ne remplace pas le vôtre. En production, le jeton
#  vient de votre fournisseur d'identité ; ce script existe pour la mise au point
#  et pour la recette d'acceptation (`p1-check.sh`).
#
#  Ce n'est pas non plus de la cryptographie maison — le dépôt s'y refuse. La
#  SIGNATURE est faite par `openssl`. Ce script ne fait que du RÉ-ENCODAGE :
#  openssl rend une signature ECDSA au format DER (`SEQUENCE { INTEGER r,
#  INTEGER s }`), alors qu'un JWT ES256 exige les 64 octets bruts `r‖s`. Convertir
#  l'un en l'autre est de la lecture d'ASN.1, pas du calcul.
#
# ── Ce que votre IdP doit émettre ─────────────────────────────────────────────
#
#    alg     DÉDUIT DE LA CLÉ : EC P-256 → ES256, RSA → RS256. Ce bloc disait
#            « ES256, et RIEN D'AUTRE […] une clé RSA est refusée — or RS256 est
#            le défaut de Keycloak et d'Auth0 ». C'était vrai, et c'était le
#            blocage d'adoption que `p1-check.sh` nommait dans « ce qu'il ne
#            prouve pas ». Levé le 6 septembre 2026 : la passerelle déduit
#            l'algorithme de la clé (`oidc::cle_et_algorithme`), et ce script fait
#            la MÊME déduction sur la MÊME entrée. Jamais depuis l'en-tête du
#            jeton : ce serait la confusion d'algorithmes.
#    aud     `efe-llm` pour la frontière (EFE_OIDC_LLM_AUDIENCE) ; pour le chemin
#            souverain, la validation d'audience est DÉSACTIVÉE tant que
#            EFE_OIDC_AUDIENCE n'est pas posée.
#    tenant  ou `tid`. Absent ⟹ le tenant système.
#    roles   ou `realm_access.roles`. C'est ce qui décide l'accès aux routes
#            d'audit (`auditor`) et d'action.
#
set -euo pipefail

CLE="${1:-}"
TENANT="${2:-00000000-0000-0000-0000-000000000001}"
ROLES="${3:-operator,auditor}"
AUD="${4:-efe-llm}"
TTL="${5:-3600}"

if [ -z "$CLE" ] || [ ! -f "$CLE" ]; then
  cat >&2 <<'FIN'
usage : scripts/p1-jeton.sh <clé-privée.pem> [tenant] [rôles] [aud] [ttl_s]

Pour fabriquer une paire de test (la publique va dans EFE_OIDC_PUBLIC_KEY_PEM) :

    openssl ecparam -name prime256v1 -genkey -noout -out cle.pem
    openssl ec -in cle.pem -pubout -out cle.pub

⚠ Une paire de TEST ne prouve rien : quiconque la détient forge un jeton. En
production, la clé publique est celle de votre IdP, et la privée ne vous
appartient pas.
FIN
  exit 2
fi

# ── De QUELLE famille est la clé ? ───────────────────────────────────────────
#
#  L'algorithme se déduit de la CLÉ, exactement comme le fait la passerelle
#  (`oidc::cle_et_algorithme`). Deux endroits qui déduisent la même chose de la
#  même entrée : si l'un changeait sans l'autre, le jeton serait forgé sous un
#  algorithme que le vérifieur n'attend pas, et l'opérateur chercherait le défaut
#  du mauvais côté — c'est ce que la version précédente de ce bloc évitait déjà,
#  en refusant simplement RSA.
if openssl ec -in "$CLE" -noout -text 2>/dev/null | grep -q 'prime256v1\|NIST CURVE: P-256'; then
  ALG=ES256
elif openssl rsa -in "$CLE" -noout -check >/dev/null 2>&1; then
  ALG=RS256
else
  echo "ERREUR : $CLE n'est ni une EC P-256 (ES256) ni une RSA (RS256)." >&2
  echo "         Ce sont les deux seules familles que la passerelle accepte." >&2
  exit 2
fi

python3 - "$CLE" "$TENANT" "$ROLES" "$AUD" "$TTL" "$ALG" <<'PY'
import base64, json, subprocess, sys, time

cle, tenant, roles, aud, ttl, alg = sys.argv[1:7]

def b64u(b: bytes) -> str:
    """base64url sans remplissage — l'encodage que le JWT impose."""
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()

entete = {"alg": alg, "typ": "JWT"}
maintenant = int(time.time())
charge = {
    "sub": "p1-jeton",
    "tenant": tenant,
    "roles": [r for r in roles.split(",") if r],
    "aud": aud,
    "iat": maintenant,
    "exp": maintenant + int(ttl),
}
signe = f"{b64u(json.dumps(entete, separators=(',', ':')).encode())}." \
        f"{b64u(json.dumps(charge, separators=(',', ':')).encode())}"

# La SIGNATURE vient d'openssl. Ce script ne calcule rien.
brut = subprocess.run(
    ["openssl", "dgst", "-sha256", "-sign", cle],
    input=signe.encode(), capture_output=True, check=True,
).stdout

def der_vers_brut(d: bytes) -> bytes:
    """DER `SEQUENCE { INTEGER r, INTEGER s }` → 64 octets `r‖s`.

    Lecture d'ASN.1, pas de cryptographie. Les entiers DER sont signés : un `r`
    dont l'octet de poids fort dépasse 0x7F porte un 0x00 de tête qu'il faut
    retirer, et un `r` court doit être rembourré à gauche jusqu'à 32 octets. Se
    tromper sur l'un ou l'autre produit un jeton d'apparence valide que la
    passerelle refuse — le genre d'erreur qu'on cherche du mauvais côté.
    """
    assert d[0] == 0x30, "la signature DER doit commencer par SEQUENCE"
    # Longueur de la séquence : forme courte (< 0x80) ou longue.
    i = 2 if d[1] < 0x80 else 2 + (d[1] & 0x7F)
    ent = []
    for _ in range(2):
        assert d[i] == 0x02, "chaque composante doit être un INTEGER"
        n = d[i + 1]
        v = d[i + 2 : i + 2 + n]
        i += 2 + n
        v = v.lstrip(b"\x00")               # retire le 0x00 de signe
        assert len(v) <= 32, "composante trop longue pour P-256"
        ent.append(v.rjust(32, b"\x00"))    # rembourre à gauche
    return b"".join(ent)

# RS256 : `openssl dgst -sign` rend DÉJÀ les octets PKCS#1 v1.5 attendus par le JWT — rien à
# convertir. ES256 est le cas compliqué, pas RS256 : ECDSA sort en DER, et c'est cette conversion
# (ci-dessus) qui demandait tout le soin. La branche RSA tient donc en une ligne, et c'est normal.
print(f"{signe}.{b64u(brut if alg == 'RS256' else der_vers_brut(brut))}")
PY
