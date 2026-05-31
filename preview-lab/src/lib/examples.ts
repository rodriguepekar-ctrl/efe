// SPDX-License-Identifier: UNLICENSED
//
// P5.6 — Prompts d'exemple chargeables en un clic dans le `MissionComposer`.
//
// But : rendre le pré-mâchage VISIBLE dans la démo. Le défaut court ne déclenche
// pas de compression notable (cible proportionnelle plancher → `savings_ratio`
// ≈ 0 → `SavingsGauge` plat). Le prompt « long » est calibré pour le *sweet
// spot* du compresseur extractif : assez long et redondant pour que l'extraction
// coupe le remplissage et fasse bouger la jauge, sans atteindre la zone où le
// modèle 1B dégénère (repli sur le prompt brut → savings nuls,
// cf. `final_prompt_from_compression`). Les savings n'apparaissent qu'en LIVE
// (`EFE_LIVE_SLM=1`, `compressor ✓`) ; en mock pur le prompt tourne sans
// compression (dégradation gracieuse).

/** Un prompt d'exemple chargeable dans le composer. */
export interface ExamplePrompt {
  /** Identifiant stable (clé React + tests). */
  id: string;
  /** Libellé affiché sur la chip. */
  label: string;
  /** Texte injecté dans `user_text`. */
  text: string;
  /**
   * Sources de fact-check à pré-remplir (UNE par ligne) — réveille le verifier
   * → événements `VERDICT_READY` (P5.5). Absent → la chip vide le champ sources.
   */
  sources?: string;
}

/**
 * Seuil heuristique (caractères) au-delà duquel le pré-mâchage devient
 * visiblement payant. **Indicatif UI uniquement** — le vrai seuil est moteur
 * (cible proportionnelle en mots, bornée [30, 500]).
 */
export const PRECHEW_HINT_CHARS = 500;

/** Prompt court — démos rapides A / B / speculative (compression ≈ nulle). */
const SHORT_PROMPT =
  "Quelle est la capitale de la France ? Réponds en une phrase courte.";

/**
 * Prompt long (~760 car.) : une vraie question factuelle (synchrone vs
 * asynchrone) NOYÉE dans du remplissage conversationnel redondant. L'extractif
 * garde la question saillante et jette le bruit → `savings_ratio` lisiblement
 * > 0, sans franchir la zone de dégénérescence du 1B.
 */
const LONG_PROMPT =
  "Bonjour, alors voilà, je me posais une question depuis un moment et je " +
  "voulais vraiment avoir ton avis, parce que franchement je ne sais plus trop " +
  "quoi en penser. En gros j'ai lu plein de choses contradictoires un peu " +
  "partout — des forums, des articles, des vidéos — et chacun raconte un peu ce " +
  "qu'il veut, donc honnêtement c'est difficile de s'y retrouver. Pour te " +
  "donner un minimum de contexte : je bricole un petit projet perso le " +
  "week-end, rien de très sérieux, mais j'aimerais bien comprendre le fond du " +
  "sujet avant de me lancer pour de bon. Bref, tout ça pour dire que ma vraie " +
  "question, au final, c'est celle-ci : quelle est la différence concrète entre " +
  "la programmation synchrone et asynchrone, et dans quels cas précis vaut-il " +
  "mieux utiliser l'une plutôt que l'autre ? Merci d'avance, et désolé pour le " +
  "pavé.";

/** Prompt court factuel + 2 sources concordantes → démo fact-check en un clic
 *  (charge le texte ET les sources → `VERDICT_READY: SUPPORTED` en live). */
const FACTCHECK_PROMPT =
  "Quelle est la capitale de la France ? Réponds en une phrase courte.";
const FACTCHECK_SOURCES = [
  "Paris est la capitale de la France.",
  "La capitale administrative de la France est la ville de Paris.",
].join("\n");

/**
 * Jeu de prompts d'exemple. Le 1er (court) est le défaut du composer ; le 2nd
 * (long) montre le pré-mâchage (compression) en direct ; le 3e (fact-check)
 * pré-remplit des sources → réveille le verifier et affiche des Verdicts.
 */
export const EXAMPLE_PROMPTS: ExamplePrompt[] = [
  { id: "short", label: "Court", text: SHORT_PROMPT },
  { id: "long", label: "Long · pré-mâchage", text: LONG_PROMPT },
  {
    id: "factcheck",
    label: "Fact-check · verdicts",
    text: FACTCHECK_PROMPT,
    sources: FACTCHECK_SOURCES,
  },
];

/** Prompt par défaut du composer (= exemple court). */
export const DEFAULT_PROMPT = SHORT_PROMPT;
