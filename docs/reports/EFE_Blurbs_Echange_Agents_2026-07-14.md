# EFE — Notices d'accompagnement / Companion blurbs (2026-07-14)

Cadrage : échange d'informations entre agents — grands modèles (Claude/Anthropic, Gemini, GPT)
et essaims de nano-robots ; nano-robot → essaim → méta-essaim. Règle : ne jamais sur-vendre.

---

## Introduction du concept (4 phrases)

**FR**
1. Faire coopérer plusieurs agents d'IA — de grands modèles comme Claude (Anthropic) et Gemini, mais aussi des essaims de « nano-robots » (de minuscules agents latents) — suppose qu'ils échangent de l'information et convergent vers une décision commune, sans qu'un seul domine et sans devoir croire une boîte noire sur parole.
2. EFE organise cet échange sur plusieurs échelles : des nano-robots (chacun une ligne d'un tenseur partagé) se replient en la réponse d'un essaim ; des essaims entiers deviennent des opérateurs que l'on couple en un méta-essaim ; et, en parallèle, un panel de modèles frontière (Claude, Gemini, GPT) échange du texte sous un reçu re-vérifiable.
3. À chaque échelle, la même discipline : agréger par repli vers un consensus, projeter sur des limites de sécurité, et refuser de servir (fail-closed) plutôt que rendre une réponse non vérifiée.
4. Règle cardinale, sans laquelle rien ne tient : décrire chaque mécanisme à sa maturité réelle — ce qui est servi par défaut, ce qui est réel mais opt-in, ce qui reste un stub — jamais plus que ce qui existe.

**EN**
1. Getting several AI agents to cooperate — large models like Claude (Anthropic) and Gemini, but also swarms of "nano-robots" (tiny latent agents) — means having them exchange information and converge on a shared decision, without any one of them dominating and without trusting a black box on faith.
2. EFE organizes that exchange across scales: nano-robots (each a row of a shared tensor) fold into a swarm's answer; whole swarms become operators that couple into a meta-swarm; and, in parallel, a frontier-model panel (Claude, Gemini, GPT) exchanges text under a re-verifiable receipt.
3. At every scale the same discipline holds: aggregate by folding toward a consensus, project onto safety limits, and refuse to serve (fail-closed) rather than return an unverified answer.
4. The cardinal rule, without which none of it holds: describe each mechanism at its real maturity — what is served by default, what is real but opt-in, what is still a stub — never as more than what ships.

---

## 1 · Rapport « Échange d'informations entre agents » (les 4 couches)
`EFE_Echange_Agents_Rapport_2026-07-14.pdf` / `EFE_Agent_Info_Exchange_Report_2026-07-14_EN.pdf`

**FR**
1. Ce rapport cartographie les quatre façons dont EFE fait échanger de l'information entre agents, chacune sur son propre binaire indépendant.
2. Couche 1 : à l'intérieur d'un essaim, des nano-agents (lignes d'un tenseur latent) fusionnent par attention et se replient en un centroïde — la seule couche active par défaut.
3. Couche 2 : un panel frontière éclate un prompt vers Claude, Gemini et GPT, fusionne leurs réponses (vote/Borda, et un vrai échange d'arguments en « debate ») et scelle un reçu re-vérifiable — réel et testé, mais désactivé par défaut.
4. Couche 3 : des essaims-opérateurs sont couplés sur gRPC en un méta-essaim (le cœur de notre travail M1), servi avec un participant mock par défaut, le vrai essaim efe-core restant opt-in.
5. Couche 4 : un blackboard agentique où des nœuds de DAG appellent le vrai Claude — mais qui répond « en dur » sans clé ; et le rapport dit à chaque couche ce qui est servi, réel-mais-off, ou stub, sans jamais sur-vendre.

**EN**
1. This report maps the four ways EFE makes information flow between agents, each on its own independent binary.
2. Layer 1: inside a swarm, nano-agents (rows of a latent tensor) fuse by attention and fold into a centroid — the only layer on by default.
3. Layer 2: a frontier panel fans a prompt out to Claude, Gemini and GPT, fuses their answers (vote/Borda, plus a genuine argument exchange in "debate") and seals a re-verifiable receipt — real and tested, but off by default.
4. Layer 3: swarm-operators are coupled over gRPC into a meta-swarm (the heart of our M1 work), served with a mock participant by default, the real efe-core swarm staying opt-in.
5. Layer 4: an agentic blackboard where DAG nodes call the real Claude — but it answers with canned text without a key; and the report states, layer by layer, what is served, real-but-off, or a stub, never over-selling.

---

## 2 · Rapport « Nano-robots, essaim, méta-essaim » (le vertical)
`EFE_Nano_Essaim_MetaEssaim_Rapport_2026-07-14.pdf` / `EFE_Nano_Swarm_MetaSwarm_Report_2026-07-14_EN.pdf`

**FR**
1. Ce rapport suit une seule idée verticale : comment l'information remonte du nano-robot à l'essaim, puis au méta-essaim, par replis successifs.
2. Un nano-robot est concrètement une ligne d'un tenseur latent (pas un robot autonome) ; sa « voix » est un vecteur, lu par tous les autres via une attention N×N.
3. Un essaim est un contrôleur qui enchaîne genèse → fusion itérée → détecteur de consensus → porte d'alignement (qui avorte si la sécurité est violée) → repli en un centroïde, sa réponse.
4. Un méta-essaim expose chaque essaim comme un opérateur Φ, couple M participants par un point fixe de Krasnoselskii–Mann, et renvoie le méta-consensus comme « tirage ».
5. À chaque étage, le document sépare la mécanique réelle du contenu qui remonte (mince par défaut : priors vides, participants mock) et rappelle qu'aucune non-expansivité n'est prouvée — seulement falsifiée par une sonde.

**EN**
1. This report follows one vertical idea: how information rises from the nano-robot to the swarm, then to the meta-swarm, through successive folds.
2. A nano-robot is concretely a row of a latent tensor (not an autonomous robot); its "voice" is a vector, read by all the others through an N×N attention.
3. A swarm is a controller that chains genesis → iterated fusion → consensus detector → alignment gate (which aborts if safety is breached) → fold into a centroid, its answer.
4. A meta-swarm exposes each swarm as an operator Φ, couples M participants through a Krasnoselskii–Mann fixed point, and sends the meta-consensus back as the "pull".
5. At every stage the document separates the real machinery from the content that actually rises (thin by default: empty priors, mock participants) and reminds that no non-expansiveness is proven — only falsified by a probe.

---

## 3 · Livret des 8 schémas conceptuels
`EFE_Concepts_Echange_Agents_2026-07-14.pdf` / `EFE_Agent_Exchange_Concepts_2026-07-14_EN.pdf`

**FR**
1. Ce livret réunit huit schémas conceptuels, un par idée-clé, pour expliquer l'échange inter-agents sans le code.
2. On y trouve les quatre médias d'échange, le repli vertical (population → vecteur), « la voix d'un agent est un vecteur », et le point fixe couplé qui relie les essaims.
3. Trois schémas expliquent les garde-fous : la sonde H1 (qui falsifie, jamais ne prouve), l'élimination de la discontinuité par topologie fixe, et le principe fail-closed (refuser plutôt que faire semblant).
4. Le dernier illustre le panel frontière : Claude, Gemini et GPT échangent du texte, seul « debate » faisant circuler de vrais arguments, le tout sous reçu re-vérifiable.
5. Chaque image porte son pied d'honnêteté (mock par défaut, borne inférieure, Borda heuristique…) pour rester juste même sortie de son contexte.

**EN**
1. This booklet gathers eight conceptual diagrams, one per key idea, to explain inter-agent exchange without the code.
2. It covers the four exchange media, the vertical fold (population → vector), "an agent's voice is a vector", and the coupled fixed point that links the swarms.
3. Three diagrams explain the guardrails: the H1 probe (which falsifies, never proves), removing the discontinuity via a fixed topology, and the fail-closed principle (refuse rather than pretend).
4. The last illustrates the frontier panel: Claude, Gemini and GPT exchange text, only "debate" circulating real arguments, all under a re-verifiable receipt.
5. Every image carries its honesty footer (mock by default, lower bound, Borda heuristic…) so it stays accurate even out of context.

---

## 4 · Schéma « Méta-essaim : avant / après »
`EFE_Schema_MetaSwarm_BeforeAfter_2026-07-09_EN.pdf` (+ version FR d'origine)

**FR**
1. Ce schéma illustre, en une image, la même décision à trois parties (par ex. acheteur, fournisseur, auditeur) « sans » puis « avec » le méta-essaim.
2. Sans : soit on centralise tout chez une partie (refusé), soit on réconcilie à la main (lent, invérifiable) — pas de décision commune fiable.
3. Avec : chaque organisation garde ses données et son équipe d'agents chez elle ; seule sa « position » (un vecteur) circule vers un espace d'accord commun.
4. Les positions se rapprochent pas à pas jusqu'à un consensus, projeté sur des limites fixées d'avance, d'où sort une décision commune accompagnée d'un reçu.
5. À présenter comme un concept cible : le reçu est re-vérifiable par le détenteur de clé (tamper-evident), pas par un tiers sans confiance, et le méta-essaim servi tourne par défaut sur un participant mock.

**EN**
1. This diagram illustrates, in a single image, the same three-party decision (e.g. buyer, supplier, auditor) "without" then "with" the meta-swarm.
2. Without: either everything is centralized at one party (refused), or reconciled by hand (slow, unverifiable) — no reliable joint decision.
3. With: each organization keeps its data and its team of agents in-house; only its "position" (a vector) travels to a shared agreement space.
4. The positions move closer step by step to a consensus, projected onto limits set in advance, out of which a joint decision comes with a receipt.
5. Present it as a target concept: the receipt is re-verifiable by the key-holder (tamper-evident), not by an untrusting third party, and the served meta-swarm runs a mock participant by default.

---

## 5 · Schéma « Essaim : avant / après » (tableau partagé d'agents)
`EFE_Schema_Swarm_BeforeAfter_2026-07-09_EN.pdf` (+ version FR d'origine)

**FR**
1. Ce schéma montre une tâche « sans » puis « avec » l'essaim, à l'échelle d'une équipe d'agents qui se partagent un même tableau.
2. Sans : les étapes s'enchaînent en file (un agent, puis l'autre), personne ne travaille en même temps, et rien ne trace qui a contribué quoi.
3. Avec : plusieurs agents — Claude, Gemini, un petit modèle, du code — lisent et écrivent le même tableau partagé et avancent en parallèle.
4. Chacun garde ses données ; seules les contributions déposées sur le tableau circulent, et l'ensemble reste re-vérifiable.
5. À présenter comme concept : ce blackboard agentique est réel dans le code mais répond « en dur » sans clé API, et la coordination multi-fournisseurs est désactivée par défaut.

**EN**
1. This diagram shows a task "without" then "with" the swarm, at the scale of a team of agents sharing a single board.
2. Without: steps run in a queue (one agent, then the next), no one works at the same time, and nothing traces who contributed what.
3. With: several agents — Claude, Gemini, a small model, code — read and write the same shared board and progress in parallel.
4. Each keeps its own data; only the contributions posted to the board travel, and the whole stays re-verifiable.
5. Present it as a concept: this agentic blackboard is real in the code but answers with canned text without an API key, and multi-vendor coordination is off by default.

---

## 6 · Schéma « Nano-robots : avant / après »
`EFE_Schema_NanoRobots_BeforeAfter_2026-07-09_EN.pdf` (+ version FR d'origine)

**FR**
1. Ce schéma illustre une tâche concrète (p. ex. traiter une facture) « sans » puis « avec » des nano-robots.
2. Sans : une seule étape monolithique, lente et opaque, dont on ne peut rien re-vérifier.
3. Avec : plusieurs petits agents spécialisés travaillent en parallèle, chacun sur sa partie, autour d'un tableau commun.
4. Chaque contribution est scellée et chaînée, de sorte qu'une modification après coup se repère et que le tout se re-vérifie.
5. À présenter comme concept cible : dans le code servi, les comportements par gabarit ne sont pas encore câblés (la spécialisation est illustrative) et la re-exécution à l'identique n'est prouvée que pour le chemin latent Φ.

**EN**
1. This diagram illustrates a concrete task (e.g. processing an invoice) "without" then "with" nano-robots.
2. Without: a single monolithic step, slow and opaque, from which nothing can be re-verified.
3. With: several small specialized agents work in parallel, each on its part, around a shared board.
4. Each contribution is sealed and chained, so a later change is spotted and the whole can be re-verified.
5. Present it as a target concept: in the served code the per-template behaviours are not yet wired (the specialization is illustrative), and identical replay is proven only for the latent Φ path.
