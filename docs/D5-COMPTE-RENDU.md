# D5 — le compte rendu de la personne qui n'est pas l'auteur

> **Ceci n'est pas un questionnaire de satisfaction.** On ne demande pas si c'était
> agréable, ni si le produit plaît. On demande **ce qui a bloqué**, où exactement,
> et ce qu'il a fallu demander. Chaque question posée à l'auteur est un défaut de
> documentation ; c'est elle qu'on cherche, pas l'avis.

## Pourquoi cette page existe

Tout le reste de ce dépôt se mesure : seize contrôles, trois portes de qualité,
3 900 tests, une CI qui joue les portes à chaque `push`. **D5 est la seule étape
pour laquelle il n'existe pas d'`exit 0`.** Elle demande une personne, une machine
qui n'est pas celle de l'auteur, et ce compte rendu.

Un formulaire vide n'est pas un succès. C'est une étape **non jouée**.

---

## Avant de commencer — à noter, sans réfléchir

| | |
|---|---|
| Date | |
| Qui (prénom suffit) | |
| Système (macOS / Linux / Windows+WSL, version) | |
| `docker --version` | |
| A déjà utilisé Docker ? (oui / un peu / non) | |
| Connaît ce projet ? (rien / le nom / en a parlé avec l'auteur) | |

⚠ **Si vous avez déjà parlé du produit avec l'auteur, dites-le.** Une personne
briefée ne mesure pas la même chose qu'une personne neuve, et les deux comptes
rendus sont utiles — à condition de savoir lequel on lit.

---

## Le parcours, dans l'ordre

### 1 · La démonstration

```bash
docker compose up
```

| | |
|---|---|
| Heure de départ | |
| Ça a démarré ? (oui / non / après un ajustement) | |
| Si non ou « après » : **le message exact**, recopié | |
| Combien de temps avant de voir quelque chose ? | |
| **Avez-vous su quand c'était fini ?** (oui / non) | |
| Avez-vous su comment l'arrêter ? (oui / non) | |

### 2 · Le premier reçu opposable

```bash
bash scripts/mon-premier-recu.sh
```

| | |
|---|---|
| Heure de départ | |
| Est-il allé au bout ? (oui / non) | |
| Si non : **à quelle section il s'est arrêté**, et le message exact | |
| Avez-vous compris ce que vous aviez obtenu ? (oui / à peu près / non) | |
| Avez-vous **rejoué** la commande de re-preuve qu'il imprime ? (oui / non) | |
| Si oui : qu'a-t-elle répondu ? | |

### 3 · La question qui compte le plus

> **À quel moment avez-vous eu envie de demander quelque chose à l'auteur ?**
> Notez-le même si vous ne l'avez pas demandé, même si vous avez trouvé tout seul
> trente secondes après. C'est la mesure la plus utile de cette page.

```
(écrire ici, une ligne par fois)



```

---

## Ce que vous avez lu, et ce que vous n'avez pas lu

Cochez ce que vous avez **réellement ouvert** — pas ce que vous auriez dû.

- [ ] `README.md`
- [ ] la sortie du terminal, en entier
- [ ] la sortie du terminal, en diagonale
- [ ] `SECURITY.md`
- [ ] `CONTRIBUTING.md`
- [ ] `docs/reports/Scella_Le_Seuil_En_Une_Commande_*.tex` (la page qu'on tend)
- [ ] rien du tout, j'ai tapé la commande

---

## Trois questions de fond, à répondre en une phrase

1. **En vos mots, qu'est-ce que ce produit fait ?**
   ```

   ```

2. **Le reçu que vous avez obtenu — à quoi servirait-il, et contre qui ?**
   ```

   ```

3. **Qu'est-ce qui vous a paru faux, exagéré, ou trop beau ?**
   ```

   ```

---

## Ce qui a DÉJÀ bloqué, avant vous

Ces deux défauts ont été trouvés le 6 septembre 2026 en marchant le chemin
d'un inconnu — et **seulement** par là. Ils sont corrigés ; ils sont listés ici
pour que vous sachiez à quoi ressemble ce qu'on cherche.

- **Le profil de production n'avait jamais produit de bundle dans Docker.** Le
  volume `/var/lib/scella` naissait `root:root` alors que le conteneur tourne en
  uid 65532 : la publication de l'ancre échouait en `Permission denied`, et le
  reçu portable rendait `500`. `p1-check.sh` ne pouvait pas le voir — il lance le
  binaire de l'HÔTE, qui possède son répertoire.
- **Un échec laissait la pile debout**, et la tentative suivante butait sur « le
  port est déjà pris » — en accusant la machine, alors que le coupable était
  l'essai précédent.

Aucun test, aucune porte, aucune CI ne les avait vus. C'est ce que cette étape
existe pour trouver, et c'est pourquoi un formulaire vide ne vaut rien.

---

## Pour l'auteur, quand ce formulaire revient

1. **Ne pas expliquer.** Chaque phrase commençant par « en fait, il faut… » est un
   défaut de documentation déguisé en malentendu.
2. **Classer chaque blocage** : documentation absente · documentation présente mais
   non trouvée · message d'erreur muet · vrai défaut de code.
3. **Le troisième cas est le plus courant et le plus rentable** — un message qui ne
   nomme pas son remède coûte une personne entière.
4. Reporter les corrections ici, datées, comme le fait la section précédente.
