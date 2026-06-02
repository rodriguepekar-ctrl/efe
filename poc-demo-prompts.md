# PoC EFE — 3 prompts de démo (copier-coller dans l'UI)

> Ces 3 prompts racontent en live le verdict mesuré (campagnes H0·a / H1·a).
> Colle-les dans le champ de l'UI Preview Lab (http://localhost:8700) et observe
> l'événement `PRE_CHEWING_METRICS` (tokens entrée → sortie) + la réponse.
>
> **Version exécutable** : `./poc.sh demo` joue ces 3 scénarios en headless et
> imprime un tableau de contraste (mêmes prompts, source : `scripts/poc/demo.py`).
>
> Rappel : en mode **keyless** (défaut) le LLM est simulé, mais le **compresseur
> SLM est réel** → tu vois la compression réelle (entrée). Le gain de **sortie**
> (réponse plus concise) et le gain **coût/latence** se voient en mode `--real`.
> Le gate de compression est ~250 tokens (~1000 car) : les scénarios 1 et 3 sont
> calibrés **au-dessus**, le 2 **en dessous**.

---

## 1) CHEWER — prompt redondant/verbeux (le compresseur agit)

> ~1500 car (~375 tokens, au-dessus du gate). Attendu avec SLM :
> `input_tokens_raw` ≫ `input_tokens_compressed`. En réel : réponse correcte
> **et concise**, coût + latence en baisse.

```
Alors voilà, honnêtement je ne sais plus trop par où commencer, j'ai lu énormément de choses un peu partout — des forums, des articles, des vidéos — et chacun raconte un peu sa version, donc c'est difficile de s'y retrouver. Alors voilà, honnêtement je ne sais plus trop par où commencer, j'ai lu énormément de choses un peu partout — des forums, des articles, des vidéos — et chacun raconte un peu sa version, donc c'est difficile de s'y retrouver. Alors voilà, honnêtement je ne sais plus trop par où commencer, j'ai lu énormément de choses un peu partout — des forums, des articles, des vidéos — et chacun raconte un peu sa version, donc c'est difficile de s'y retrouver. Alors voilà, honnêtement je ne sais plus trop par où commencer, j'ai lu énormément de choses un peu partout — des forums, des articles, des vidéos — et chacun raconte un peu sa version, donc c'est difficile de s'y retrouver. Alors voilà, honnêtement je ne sais plus trop par où commencer, j'ai lu énormément de choses un peu partout. Bref, ma vraie question : quelle est la différence concrète entre la programmation synchrone et asynchrone, et quand préférer l'une à l'autre ? Réponds simplement.
```

---

## 2) BYPASS — prompt court (sous le gate, aucun pré-mâchage)

> ~60 car (~15 tokens, sous le gate). Attendu : `input_tokens_raw ==
> input_tokens_compressed` (compression sautée). Zéro overhead : le système ne se
> sabote pas.

```
Quelle est la capitale de la France ? Réponds en une phrase.
```

---

## 3) COMPROMIS — prompt détaillé (brièveté vs complétude)

> ~1300 car (~320 tokens, au-dessus du gate). Le compresseur réduit fort (mesuré
> H1·a : ~326→130 tokens, ratio ~0,4), mais la réponse devient plus concise — au
> risque de **perdre de la complétude** quand la question exige de l'exhaustivité.
> C'est la limite assumée du pré-mâchage (verdict H1·a : justesse ~4/5 mais
> complétude ~2/5 sur ce type de prompt — visible en mode `--real`).
>
> ⚠️ Tous les prompts denses ne chewent pas : le compresseur 1B en **dégénère**
> certains de façon déterministe (ex. un prompt sur l'idempotence → no-op). C'est
> la fiabilité limitée du modèle, documentée en H1·a.

```
Je dois concevoir la limitation de débit de notre passerelle d'API et je veux comparer rigoureusement les algorithmes token bucket et leaky bucket avant de trancher. Le besoin : autoriser des rafales courtes légitimes, par exemple un client qui synchronise plusieurs ressources d'un coup au réveil de l'application, tout en bornant le débit moyen sur la durée et en protégeant le backend d'une surcharge. Pour le token bucket, je voudrais comprendre comment le seau se remplit à taux constant jusqu'à une capacité maximale, comment chaque requête consomme un jeton, et pourquoi cette capacité accumulée pendant les périodes calmes permet d'absorber une rafale tant que des jetons restent disponibles. Pour le leaky bucket, je veux saisir comment il impose au contraire un débit de sortie parfaitement régulier en faisant fuir les requêtes à taux fixe, et pourquoi il lisse les rafales au prix d'une latence d'attente ou d'un rejet quand la file est pleine. Je veux aussi comparer l'état à maintenir par client, le comportement exact aux limites, et l'interaction avec plusieurs instances de passerelle partageant un compteur central. Au final, indique lequel des deux tu choisirais pour autoriser des pics courts tout en garantissant un débit moyen borné, et le raisonnement complet derrière ce choix.
```

---

## Variante headless (sans UI)

Le plus simple — le driver intégré (bridge en marche) :

```bash
./poc.sh demo          # joue les 3 scénarios + tableau de contraste
```

Ou un seul prompt via curl :

```bash
curl -N -s http://localhost:8700/v1/missions/a \
  -H 'content-type: application/json' \
  -d '{"user_text":"Quelle est la capitale de la France ? Réponds en une phrase."}'
```

Tu verras les frames `data: {...}` défiler, dont une ligne
`"type":"PRE_CHEWING_METRICS"` avec `input_tokens_raw` / `input_tokens_compressed`.
