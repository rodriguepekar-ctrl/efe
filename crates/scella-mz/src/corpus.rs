// SPDX-License-Identifier: UNLICENSED
//! # Le corpus à vérité de terrain.
//!
//! Sans liste d'entités connues, la préservation ne se mesure pas — on ne peut que la *croire*.
//! Chaque document porte donc la liste de ce qu'on y a délibérément placé.
//!
//! ## Écrit AVANT l'expérience, et c'est la moitié de sa valeur
//!
//! Un corpus rédigé après avoir vu les résultats n'est plus un corpus : c'est une justification. Ces
//! six documents et leurs entités sont figés ici, dans le dépôt, avant qu'aucun chiffre n'existe.
//! Les ajuster ensuite --- retirer l'entité que le compresseur perd toujours, par exemple --- se
//! verrait dans l'historique.
//!
//! ## Trois familles, parce que le plafond n'a aucune raison d'être le même
//!
//! * [`Famille::Dense`] --- beaucoup d'entités au millier de mots. Un dossier fournisseur : la
//!   compression doit renoncer à quelque chose, et on veut savoir à quoi.
//! * [`Famille::Diffus`] --- peu d'entités noyées dans de la prose. Un compte rendu : le résumé a
//!   de la place, donc l'échec y serait un échec de *sélection*, pas de budget.
//! * [`Famille::Adverse`] --- entités en **fin** de document, et formes voisines de la même entité
//!   (« ACME Metallurgie SA », « ACME », « la société »). C'est là qu'un résumeur extractif coupe
//!   en premier, et c'est là que la mesure devient intéressante.
//!
//! ## Les entités sont exigées VERBATIM
//!
//! Le contrat du compresseur dit « préservant des entités spécifiées ». Le contrôle de MZ est donc
//! littéral (cf. [`crate::verifieur`]) : une entité reformulée est une entité perdue, du point de
//! vue d'un dossier qui doit citer un numéro, une date ou un montant.

/// La famille d'un document — elle décide de ce que sa mesure apprend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Famille {
    /// Beaucoup d'entités au millier de mots.
    Dense,
    /// Peu d'entités, noyées dans de la prose.
    Diffus,
    /// Entités en fin de document, et formes voisines.
    Adverse,
}

impl Famille {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Famille::Dense => "dense",
            Famille::Diffus => "diffus",
            Famille::Adverse => "adverse",
        }
    }
}

/// Un document et sa vérité de terrain.
#[derive(Debug, Clone, Copy)]
pub struct Document {
    pub id: &'static str,
    pub famille: Famille,
    pub texte: &'static str,
    /// Ce qui **doit** survivre. C'est cette liste qui part en `must_preserve`, et c'est contre elle
    /// que le résumé est relu.
    pub entites: &'static [&'static str],
}

impl Document {
    /// Le nombre de mots du texte source — le dénominateur du ratio.
    #[must_use]
    pub fn mots(&self) -> usize {
        self.texte.split_whitespace().count()
    }
}

// ── DENSE ────────────────────────────────────────────────────────────────────────────────────

const DENSE_1: &str = "\
Dossier de diligence fournisseur SUP-2024-0417, ouvert le 2024-03-11 par le comité conformité. \
Le fournisseur ACME Metallurgie SA, immatriculé sous le numéro FR-88231904, exploite deux sites de \
production : Gdansk en Pologne et Valenciennes en France. Le volume d'achat consolidé de l'exercice \
precedent s'eleve a 1 240 000 EUR, en hausse de onze pour cent. La certification ISO 14001 du site \
de Gdansk a ete renouvelee le 2023-11-02 et expire le 2026-11-02. Le site de Valenciennes ne \
dispose que d'une certification ISO 9001, obtenue le 2022-06-15, ce qui a ete releve comme un ecart \
au titre de la clause 7.3 du contrat cadre. Le questionnaire social retourne le 2024-02-28 signale \
l'absence de convention collective sur le site polonais. Un audit sur place a ete diligente le \
2024-04-09 par le cabinet Vertex Audit Partners, dont le rapport conclut a un risque MODERE. Deux \
non-conformites mineures ont ete relevees : l'absence de registre des heures supplementaires, et \
un stockage de solvants non conforme a la clause 12.1. Le fournisseur s'est engage a corriger les \
deux points avant le 2024-09-30. Le comite a maintenu le referencement sous reserve d'une \
contre-visite. Aucun element ne permet a ce stade de conclure a un travail force ou a un travail \
des enfants au sens des articles 3 et 4 de la directive. Le montant provisionne au titre du risque \
residuel est de 46 000 EUR. Le prochain reexamen est fixe au 2025-03-11, soit douze mois apres \
l'ouverture. Le dossier reste ouvert jusqu'a reception de la contre-visite et du registre corrige.";

const DENSE_2: &str = "\
Rapport d'incident QUAL-2024-0092 concernant le lot 7745-B livre le 2024-05-22 par Northline \
Composites Ltd. Le controle reception a detecte une derive dimensionnelle de 0,42 mm sur \
quarante-trois pieces sur deux cents, soit un taux de rebut de 21,5 pour cent. Le seuil contractuel \
est fixe a 2 pour cent par la clause 9.4. Le fournisseur a ete notifie le 2024-05-24 et a reconnu \
un defaut de calibration sur la presse P-118 de son site de Sheffield. La production du lot 7745-B \
a ete tracee jusqu'a l'operateur habilite numero OP-3391, dont la derniere requalification datait \
du 2023-08-17, soit au-dela de l'intervalle de douze mois exige. Le cout direct du rebut est evalue \
a 87 400 EUR, auxquels s'ajoutent 12 000 EUR de retard de ligne. Une demande d'avoir a ete emise \
sous la reference AV-2024-0311. Northline Composites Ltd a propose un plan d'action en trois \
volets : recalibration immediate de la presse P-118, requalification de l'ensemble des operateurs \
avant le 2024-06-30, et mise en place d'un controle statistique renforce sur les trois lots \
suivants. Le comite qualite a accepte le plan le 2024-06-03 en le conditionnant a un audit de \
suivi. L'audit a eu lieu le 2024-07-15 et a confirme la recalibration, mais a releve que la \
requalification n'etait effective que pour onze operateurs sur dix-sept. Le dossier reste en \
surveillance renforcee jusqu'au 2025-01-15.";

// ── DIFFUS ───────────────────────────────────────────────────────────────────────────────────

const DIFFUS_1: &str = "\
Le comite de pilotage s'est reuni pour examiner l'avancement du programme de mise en conformite \
engage l'an dernier. Les participants ont d'abord rappele le contexte general : la pression \
reglementaire s'est accentuee, les attentes des clients se sont deplacees vers la tracabilite, et \
plusieurs concurrents ont publie des engagements publics que nous n'avons pas encore egales. La \
discussion a porte longuement sur la difficulte a obtenir des donnees fiables aupres des \
fournisseurs de rang deux, qui ne sont lies par aucune obligation contractuelle directe. Plusieurs \
membres ont souligne que les questionnaires actuels sont trop longs et que le taux de retour s'en \
ressent. Une refonte a ete evoquee, sans decision. Le sujet des ressources a ete aborde : l'equipe \
conformite compte aujourd'hui trois personnes a temps plein, ce qui est juge insuffisant au regard \
du perimetre. Une demande de renfort sera portee au prochain arbitrage budgetaire. Sur le plan des \
outils, il a ete rappele que le deploiement de la plateforme SCELLA-P3 est prevu pour le \
2024-10-01, et que la reprise de l'historique reste un point ouvert. Le comite a enfin evoque la \
communication externe, en insistant sur la prudence : rien ne doit etre publie qui ne soit \
demontrable. La prochaine reunion est fixee au 2024-09-12. Aucune decision formelle n'a ete prise \
au cours de cette seance, qui avait un caractere preparatoire.";

const DIFFUS_2: &str = "\
La revue annuelle des pratiques d'achat responsable a permis de dresser un etat des lieux \
contraste. D'un cote, la sensibilisation des acheteurs a nettement progresse et la grille \
d'evaluation est desormais employee de facon systematique dans les appels d'offres superieurs a un \
certain seuil. De l'autre, l'articulation entre l'evaluation et la decision reste faible : dans la \
majorite des cas examines, le critere extra-financier n'a pas modifie le classement des offres. \
Plusieurs explications ont ete avancees au cours des entretiens. La premiere tient a la ponderation \
retenue, jugee trop basse par les acheteurs eux-memes. La deuxieme tient a la qualite des donnees, \
souvent declaratives et rarement verifiees. La troisieme, plus rarement exprimee mais recurrente, \
tient a la crainte de fragiliser une relation fournisseur ancienne. Le rapport recommande de \
revoir la ponderation, d'introduire une verification par echantillonnage, et de clarifier la \
procedure d'escalade lorsqu'un ecart significatif est constate. Il note enfin que le dispositif \
actuel ne couvre pas les achats indirects, qui representent pourtant une part importante de la \
depense. Une extension du perimetre est proposee pour l'exercice 2025, sous reserve des moyens \
disponibles. Le document a ete transmis au comite d'audit le 2024-07-04.";

// ── ADVERSE ──────────────────────────────────────────────────────────────────────────────────

const ADVERSE_1: &str = "\
Note de synthese relative au reexamen du referencement. Le dossier a ete instruit selon la \
procedure habituelle, avec un examen documentaire suivi d'entretiens. La societe examinee opere \
depuis une quinzaine d'annees dans le secteur des composants metalliques et entretient avec nous \
une relation commerciale ancienne. Les entretiens menes aupres des equipes achats font ressortir \
une satisfaction generale sur la qualite et les delais. Les elements financiers communiques ne \
revelent aucune fragilite particuliere. Le point de vigilance porte sur la chaine de sous-traitance, \
dont la cartographie reste incomplete au-dela du rang un. La societe indique travailler a cette \
cartographie et s'engage a la transmettre. Rien dans les elements recueillis ne justifie une \
suspension. Le rapport d'instruction recommande le maintien du referencement, assorti d'un suivi \
renforce. Les elements d'identification precis figurent en fin de note, comme le veut la procedure \
interne : la societe concernee est ACME Metallurgie SA, le dossier porte la reference \
SUP-2024-0417, la decision prend effet le 2025-01-06, et le prochain reexamen interviendra au plus \
tard le 2026-01-06. Le montant d'engagement annuel autorise est plafonne a 1 500 000 EUR.";

const ADVERSE_2: &str = "\
Compte rendu de la visite effectuee sur le site de production. La journee a commence par une \
presentation generale des activites, suivie d'un parcours des ateliers. L'organisation apparait \
maitrisee et les postes de travail sont correctement amenages. Les responsables rencontres ont \
repondu aux questions sans reticence apparente. Le systeme documentaire est en place, meme si \
certaines procedures n'ont pas ete revues depuis plusieurs annees. La societe applique un plan de \
formation annuel dont le taux de realisation nous a ete presente comme satisfaisant. La visite des \
zones de stockage n'a revele aucune anomalie majeure. En fin de parcours, nous avons demande a \
consulter le registre des incidents, qui nous a ete communique sans difficulte. Les elements \
formels de la visite sont les suivants : l'entite visitee est Northline Composites Ltd, le site est \
celui de Sheffield, la visite s'est tenue le 2024-11-19, elle a ete conduite par le cabinet Vertex \
Audit Partners, et le rapport definitif porte la reference AUD-2024-0781. Northline s'est engagee a \
transmettre le plan d'action avant le 2024-12-20.";

/// Le corpus, figé.
pub const CORPUS: &[Document] = &[
    Document {
        id: "dense-1",
        famille: Famille::Dense,
        texte: DENSE_1,
        entites: &[
            "SUP-2024-0417",
            "2024-03-11",
            "ACME Metallurgie SA",
            "FR-88231904",
            "1 240 000 EUR",
            "ISO 14001",
            "2026-11-02",
            "clause 7.3",
            "Vertex Audit Partners",
            "clause 12.1",
            "46 000 EUR",
            "2025-03-11",
        ],
    },
    Document {
        id: "dense-2",
        famille: Famille::Dense,
        texte: DENSE_2,
        entites: &[
            "QUAL-2024-0092",
            "7745-B",
            "Northline Composites Ltd",
            "0,42 mm",
            "clause 9.4",
            "P-118",
            "OP-3391",
            "87 400 EUR",
            "AV-2024-0311",
            "2024-06-30",
            "2025-01-15",
        ],
    },
    Document {
        id: "diffus-1",
        famille: Famille::Diffus,
        texte: DIFFUS_1,
        entites: &["SCELLA-P3", "2024-10-01", "2024-09-12"],
    },
    Document {
        id: "diffus-2",
        famille: Famille::Diffus,
        texte: DIFFUS_2,
        entites: &["2024-07-04", "2025"],
    },
    Document {
        id: "adverse-1",
        famille: Famille::Adverse,
        texte: ADVERSE_1,
        entites: &[
            "ACME Metallurgie SA",
            "SUP-2024-0417",
            "2025-01-06",
            "2026-01-06",
            "1 500 000 EUR",
        ],
    },
    Document {
        id: "adverse-2",
        famille: Famille::Adverse,
        texte: ADVERSE_2,
        entites: &[
            "Northline Composites Ltd",
            "Sheffield",
            "2024-11-19",
            "Vertex Audit Partners",
            "AUD-2024-0781",
            "2024-12-20",
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// ⭐ **Le contrôle qui rend le corpus utilisable.** Une entité qui n'est pas dans son propre
    /// document rendrait la préservation impossible à atteindre, et l'expérience mesurerait une
    /// faute de frappe au lieu du compresseur.
    #[test]
    fn chaque_entite_est_reellement_dans_son_document() {
        for d in CORPUS {
            for e in d.entites {
                assert!(
                    crate::verifieur::presente(d.texte, e),
                    "{} : « {e} » n'est pas dans le texte source — l'expérience mesurerait une \
                     faute de frappe, pas le compresseur",
                    d.id
                );
            }
        }
    }

    #[test]
    fn les_identifiants_sont_uniques() {
        let mut vus = std::collections::BTreeSet::new();
        for d in CORPUS {
            assert!(vus.insert(d.id), "identifiant dupliqué : {}", d.id);
        }
    }

    /// Un document trop court n'a rien à comprimer, et le contrat exige ≥ 50 caractères.
    #[test]
    fn chaque_document_a_de_quoi_etre_comprime() {
        for d in CORPUS {
            assert!(d.texte.len() >= 50, "{} : sous le minimum du contrat", d.id);
            assert!(
                d.mots() >= 150,
                "{} : {} mots — trop court pour qu'un ratio de 8:1 veuille dire quelque chose",
                d.id,
                d.mots()
            );
        }
    }

    /// Les trois familles doivent être exercées, sinon le balayage ne mesure qu'un cas.
    #[test]
    fn les_trois_familles_sont_representees() {
        for f in [Famille::Dense, Famille::Diffus, Famille::Adverse] {
            let n = CORPUS.iter().filter(|d| d.famille == f).count();
            assert!(n >= 2, "{} : {n} document(s), il en faut au moins 2", f.code());
        }
    }

    /// La famille ADVERSE n'a de sens que si ses entités sont réellement **en fin** de document —
    /// là où un résumeur extractif coupe. Sans ce contrôle, « adverse » serait une étiquette.
    #[test]
    fn le_corpus_adverse_place_bien_ses_entites_a_la_fin() {
        for d in CORPUS.iter().filter(|d| d.famille == Famille::Adverse) {
            for e in d.entites {
                let pos = d.texte.find(e).expect("entité présente");
                let part = pos as f32 / d.texte.len() as f32;
                assert!(
                    part > 0.55,
                    "{} : « {e} » apparaît à {:.0} % du document — pas assez tard pour éprouver \
                     quoi que ce soit",
                    d.id,
                    part * 100.0
                );
            }
        }
    }
}
