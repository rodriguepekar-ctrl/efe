// SPDX-License-Identifier: UNLICENSED
//! # `scella-conservation` — P2 : la conservation **opérable**.
//!
//! Le dépôt savait sceller, servir, exporter et re-prouver. Il ne savait pas **remettre en place**.
//! `grep -rni backup` sur les trois crates de service rendait zéro ; `rotate|rotation`, une seule
//! occurrence, et c'était un commentaire qui renvoyait **hors** du produit. Un registre censé tenir
//! des années, dont personne n'a écrit comment on le remonte après une perte de disque, est
//! incomplet d'une manière qu'un DPO voit à sa première question.
//!
//! Trois gestes manquaient, et un seul est difficile.
//!
//! 1. [`archive`] — **sauvegarder** : assembler, à côté des records, le peu de contexte sans lequel
//!    une restauration ne peut rien affirmer.
//! 2. [`restauration`] — **restaurer** : et surtout, quand ça rate, dire **laquelle** des cinq
//!    choses différentes vient de rater.
//! 3. [`generations`] — **faire tourner la clé tenant** sans casser un seul reçu déjà scellé.
//!
//! ## La contrainte qui a dessiné tout le reste
//!
//! Une rotation a besoin de savoir **quelle clé a scellé quoi**. Le réflexe est d'ajouter un champ
//! `generation` au nœud. Il faut résister, et la raison n'est pas esthétique :
//!
//! > le sceau porte sur `canonical(nœud)`. Ajouter un champ change l'encodage canonique, donc
//! > **tous** les sceaux, donc **toutes** les chaînes — y compris celles déjà remises à des tiers —
//! > et casse `verify-mini`, dont l'arbre de dépendances vide n'a de valeur que parce qu'il calcule
//! > exactement ce que S0 scelle.
//!
//! La génération vit donc dans le **manifeste de l'archive**, jamais dans le record. C'est une
//! propriété du *contenant*, pas du fait scellé. Conséquence heureuse : les archives produites par
//! ce crate se re-prouvent avec le `verify-mini` **existant**, sans un octet de changement.
//!
//! ## Ce que l'archive ne porte pas, et pourquoi
//!
//! * **La clé tenant.** Jamais. C'est l'invariant structurel du bundle forensic (`I-S4-SANS-CLÉ`),
//!   et il n'y a aucune raison qu'une sauvegarde soit plus bavarde qu'un export : une archive qui
//!   contiendrait sa propre clé serait un coffre avec la combinaison peinte dessus.
//! * **Le trousseau des clés de cas.** Il a son cycle de vie propre — `audit-console` le rend déjà
//!   durable, avec ses pierres tombales — et surtout il **est un secret**, quand l'archive ne l'est
//!   pas. L'archive n'en garde que l'**empreinte**, ce qui suffit à dire « vous restaurez ces
//!   records avec un autre trousseau que celui d'origine » sans rien révéler.
//!
//! ## La rotation est le broyage de M6, un étage plus haut — à un état près
//!
//! M6 donne à une clé de **cas** trois états : vivante, broyée (avec sa date), inconnue. Une clé de
//! **tenant** en demande **quatre**, et le quatrième est exactement ce que la rotation ajoute :
//!
//! | état | scelle du neuf | re-prouve l'ancien |
//! |---|---|---|
//! | [`EtatGeneration::Vivante`]  | oui | oui |
//! | [`EtatGeneration::Retiree`]  | **non** | **oui** |
//! | [`EtatGeneration::Detruite`] | non | non — et c'est **daté** |
//! | [`EtatGeneration::Inconnue`] | non | non — et ce n'est **pas** une affirmation d'inexistence |
//!
//! C'est la ligne `Retiree` qui fait toute la valeur : **une rotation ne détruit rien**. On cesse de
//! sceller avec une clé, on continue de lire ce qu'elle a scellé, et le jour où la conservation
//! légale expire, on la détruit — ce qui est alors le crypto-shredding de M6 appliqué à la
//! génération entière.
//!
//! Et la distinction que M6 défend au niveau du cas se défend ici mot pour mot : un reçu scellé sous
//! une génération détruite doit se lire **« détruite le 12 mars »**, jamais « invalide ». « Invalide »
//! accuse quelqu'un d'avoir altéré la preuve. C'est une accusation, et elle serait fausse.
//!
//! ## La limite, énoncée avant qu'on la découvre
//!
//! Une archive porte sa propre racine. Elle prouve donc qu'elle n'a pas bougé **depuis qu'elle a été
//! écrite** — pas qu'elle était complète à ce moment-là. Un producteur qui tronque *avant*
//! d'archiver produit une archive parfaitement cohérente. C'est exactement la limite du bundle
//! forensic, et elle a exactement le même remède, qui n'est pas dans ce crate : l'**ancre**, publiée
//! ailleurs, par un canal que le porteur de l'archive ne contrôle pas. Le manifeste réserve un champ
//! pour l'y rattacher ; il ne peut pas la remplacer.

pub mod archive;
pub mod generations;
pub mod manifeste;
pub mod restauration;

pub use archive::{sauvegarder, ArchiveError, Sauvegarde};
pub use generations::{EtatGeneration, Generation, TrousseauMaitre};
pub use manifeste::{Manifeste, ManifesteError, FORMAT};
pub use restauration::{restaurer, restaurer_avec, Issue, Restauration};
