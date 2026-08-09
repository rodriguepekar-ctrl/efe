// SPDX-License-Identifier: UNLICENSED
//! # Couche 3 · M2 — **le reçu de consensus, re-prouvé par un tiers** (la DoD du jalon).
//!
//! Le jalon M2 se résume à une phrase : *une ronde de consensus servie rend un reçu qu'un tiers
//! re-prouve hors-ligne, sans faire confiance à ce serveur.* Ce fichier est la mise à l'épreuve de
//! cette phrase, et il est écrit pour **pouvoir échouer**.
//!
//! ## Pourquoi la re-preuve passe par `verify-meta-mini` et par lui seul
//!
//! Le vérifieur est en **dev-dépendance** : le binaire servi ne le lie pas. Et à l'intérieur du
//! test, la re-preuve n'emploie QUE ses primitives — son parseur JSON fait main, son SHA-256, son
//! HMAC, sa chaîne. Aucun crate du serveur, aucun `serde`. Si l'on relisait le bundle avec les
//! types qui l'ont produit, on ne prouverait qu'une chose : que le serveur est d'accord avec
//! lui-même. Le tiers de la DoD est sceptique, donc il apporte ses propres outils.
//!
//! ## Ce que ces tests couvrent, et pourquoi chacun est là
//!
//! | Test | Ce qu'il empêche |
//! |---|---|
//! | `served_receipt_is_reproved_by_a_skeptical_third_party` | la DoD elle-même : un bundle servi qui ne se re-prouve pas |
//! | `a_tampered_step_is_refused_and_localised` | un reçu qui « passe » quoi qu'on lui fasse (contrôle négatif) |
//! | `the_wire_root_and_the_manifest_root_agree` | un root sur le fil qui ne serait pas celui du bundle |
//! | `provenance_does_not_change_the_numerics` | que sceller altère le consensus servi |
//! | `without_provenance_the_root_is_empty_never_a_placeholder` | le « vert malhonnête » : un root décoratif |
//! | `the_bundle_is_opt_in_but_the_root_is_not` | payer le bundle sans l'avoir demandé, ou perdre le root |
//! | `the_served_receipt_names_its_participants_and_its_rule` | **v4** : un reçu qui ne sait pas nommer l'agent derrière une position |
//! | `a_third_party_rejects_a_receipt_whose_identities_were_rewritten` | **v4** : des noms falsifiés sous un root valide (contrôle négatif) |

mod common;

use efe_meta_swarm_proto::meta_swarm::IterateResponse;
use efe_meta_swarm_server::controller::provenance::ProvenanceConfig;
use efe_meta_swarm_server::controller::ControllerService;
use efe_meta_swarm_server::proto::MetaSwarmService;
use tonic::Request;

use efe_meta_swarm::provenance::{HmacKeySource, TenantId};

const DIM: usize = 8;

/// La clé de dev — **la même** que celle du serveur sous `test_key()`. Un tiers réel la recevrait
/// hors-bande de son détenteur ; ici on la connaît parce qu'on joue les deux rôles.
fn tenant_key() -> Vec<u8> {
    HmacKeySource::test_key().derive(TenantId::system()).expect("clé de dev dérivable")
}

/// Monte trois participants + un contrôleur, **avec** provenance active sous la clé de dev.
async fn serve_with_provenance(
) -> (ControllerService, Vec<(String, usize, f32, u64, std::net::SocketAddr)>) {
    let statics = spawn_three().await;
    let mut state = common::build_controller_state_for(&statics, 10, DIM);
    state.set_provenance(std::sync::Arc::new(ProvenanceConfig::new(
        HmacKeySource::test_key(),
        TenantId::system(),
        "test_key() (test)",
        true,
    )));
    (ControllerService::new(state), statics)
}

async fn spawn_three() -> Vec<(String, usize, f32, u64, std::net::SocketAddr)> {
    let (a1, h1) = common::spawn_participant_simple("supplier-scoring", DIM as u32, 0.6, 11).await;
    let (a2, h2) = common::spawn_participant_simple("contract-risk", DIM as u32, 0.7, 22).await;
    let (a3, h3) = common::spawn_participant_simple("spend-categorization", DIM as u32, 0.5, 33).await;
    // Les gardes de tâche doivent survivre à l'appel : on les fuit délibérément pour la durée du
    // test (le harnais fait de même) — les serveurs meurent avec le process de test.
    std::mem::forget((h1, h2, h3));
    vec![
        ("supplier-scoring".into(), DIM, 0.6_f32, 11_u64, a1),
        ("contract-risk".into(), DIM, 0.7_f32, 22_u64, a2),
        ("spend-categorization".into(), DIM, 0.5_f32, 33_u64, a3),
    ]
}

// =====================================================================
// LE TIERS SCEPTIQUE — n'emploie QUE les primitives de `verify-meta-mini`
// =====================================================================

/// Re-prouve un bundle exactement comme le ferait la commande
/// `verify_meta_mini meta_steps.jsonl meta_manifest.json --key-hex <64hex>`, mais en process.
///
/// Reproduit la séquence du binaire, y compris **le recoupement `n_steps`** : un manifeste qui
/// revendique un nombre de pas différent du fichier est refusé AVANT la re-preuve — c'est ce qui
/// attrape une troncature, dont la chaîne restante serait pourtant parfaitement intègre.
fn third_party_reproves(steps_jsonl: &str, manifest_json: &str, key: &[u8]) -> Result<String, String> {
    use verify_meta_mini::json::parse_object;
    use verify_meta_mini::verify::{from_hex32, to_hex, verify, Outcome, Step};

    let steps: Vec<Step> = steps_jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(Step::from_json_line)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("ligne de pas illisible : {e}"))?;

    let m = parse_object(manifest_json.trim()).map_err(|e| format!("manifeste illisible : {e}"))?;
    let meta_id = m.get_str("meta_id").map_err(|e| format!("meta_id : {e}"))?.to_string();
    let n_steps = m.get_u64("n_steps").map_err(|e| format!("n_steps : {e}"))? as usize;
    let root_hex = m.get_str("root").map_err(|e| format!("root : {e}"))?;
    let expected_root = from_hex32(root_hex).ok_or_else(|| "root : 64 hex attendus".to_string())?;

    if n_steps != steps.len() {
        return Err(format!("manifeste incohérent : n_steps={n_steps} ≠ {} lignes", steps.len()));
    }
    // **v4** — recouper la LIAISON avant la chaîne. Sans ce contrôle, un manifeste dont on aurait
    // réécrit les identités passerait la re-preuve (qui ne regarde que le condensé scellé) et
    // l'auditeur lirait des noms falsifiés sous un root valide.
    let binding = verify_meta_mini::verify::Binding::from_manifest(&m)
        .map_err(|e| format!("liaison illisible : {e}"))?;
    verify_meta_mini::verify::check_binding(&steps, binding.as_ref())
        .map_err(|e| format!("REFUSÉ (liaison) : {e}"))?;
    match verify(&steps, key, &expected_root) {
        Outcome::Ok { root } => {
            assert!(!meta_id.is_empty(), "un bundle sans meta_id n'identifie rien");
            Ok(to_hex(&root))
        }
        Outcome::Broken { at, reason } => Err(format!("REFUSÉ au pas {at} : {reason}")),
    }
}

/// Extrait le bundle d'une réponse servie, en échouant fort s'il manque — un test qui « passerait »
/// sur un reçu absent ne prouverait rien.
fn bundle_of(resp: &IterateResponse) -> (&str, &str) {
    let b = resp
        .meta_provenance_bundle
        .as_ref()
        .expect("le bundle a été DEMANDÉ : son absence est un échec, pas un cas limite");
    (&b.steps_jsonl, &b.manifest_json)
}

// =====================================================================
// LA DoD
// =====================================================================

/// **LE test du jalon M2.** Une ronde servie rend un reçu qu'un tiers re-prouve avec ses propres
/// maths, et le root re-calculé est celui annoncé sur le fil.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn served_receipt_is_reproved_by_a_skeptical_third_party() {
    let (svc, _statics) = serve_with_provenance().await;

    let resp = svc
        .iterate(Request::new(common::iterate_request_with_receipt(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("l'Iterate servi réussit")
        .into_inner();

    // Le reçu n'a de sens que sur un vrai consensus : on exige d'abord la convergence.
    assert_eq!(
        resp.stop_reason,
        efe_meta_swarm_proto::meta_swarm::StopReason::Converged as i32,
        "reçu attendu sur une ronde CONVERGÉE : {resp:?}",
    );

    let (steps_jsonl, manifest_json) = bundle_of(&resp);
    assert!(!steps_jsonl.is_empty(), "un bundle sans pas ne re-prouve rien");

    let recomputed = third_party_reproves(steps_jsonl, manifest_json, &tenant_key())
        .expect("le tiers re-prouve le reçu servi");

    assert_eq!(
        recomputed, resp.meta_provenance_root,
        "le root RE-CALCULÉ par le tiers doit être celui servi sur le fil",
    );
    assert_eq!(recomputed.len(), 64, "root = SHA-256 en hex");
}

/// **Le contrôle négatif — sans lui, le test précédent ne vaut rien.** Un reçu dont on modifie un
/// seul pas doit être REFUSÉ, et le refus doit être *localisé* : un vérifieur qui dirait seulement
/// « non » laisserait l'auditeur sans prise.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tampered_step_is_refused_and_localised() {
    let (svc, _statics) = serve_with_provenance().await;
    let resp = svc
        .iterate(Request::new(common::iterate_request_with_receipt(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("iterate")
        .into_inner();
    let (steps_jsonl, manifest_json) = bundle_of(&resp);
    let key = tenant_key();

    // Sanité : intact ⟹ passe. (Si ce préalable tombait, le reste du test serait vide de sens.)
    third_party_reproves(steps_jsonl, manifest_json, &key).expect("intact ⟹ intègre");

    // (a) Falsifier le CONTENU d'un pas — le sceau ne correspond plus au contenu re-haché.
    //     L'altération est ASSERTÉE : sans cela, un changement du format JSON (une espace après
    //     le deux-points) ferait silencieusement échouer le `replacen`, le test passerait sans
    //     rien falsifier, et ce contrôle négatif deviendrait décoratif.
    let tampered = steps_jsonl.replacen("\"residual\":", "\"residual\": 0.0, \"_ignored\":", 1);
    assert_ne!(tampered, *steps_jsonl, "l'altération DOIT mordre — sinon ce test ne teste rien");
    let err = third_party_reproves(&tampered, manifest_json, &key)
        .expect_err("un pas falsifié doit être REFUSÉ");
    assert!(err.contains("REFUSÉ") || err.contains("illisible"), "refus explicite : {err}");

    // (b) TRONQUER la chaîne — le cas vicieux : le préfixe restant est PARFAITEMENT intègre en
    //     tant que chaîne. Seul le recoupement avec le manifeste (n_steps + root) le démasque.
    //     C'est exactement l'attaque que le Fil P a traitée côté pile A/B.
    let lines: Vec<&str> = steps_jsonl.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(lines.len() >= 2, "il faut au moins deux pas pour qu'une troncature ait un sens");
    let truncated = lines[..lines.len() - 1].join("\n") + "\n";
    let err = third_party_reproves(&truncated, manifest_json, &key)
        .expect_err("une chaîne TRONQUÉE doit être refusée");
    assert!(
        err.contains("n_steps") || err.contains("REFUSÉ"),
        "la troncature est démasquée par le manifeste, pas par la chaîne : {err}",
    );

    // (c) Mauvaise clé — un tiers qui n'a pas la clé n'obtient pas un faux « OK ».
    let wrong = vec![0x00_u8; 32];
    let err = third_party_reproves(steps_jsonl, manifest_json, &wrong)
        .expect_err("une clé fausse ne doit jamais rendre un verdict positif");
    assert!(err.contains("REFUSÉ"), "refus sur mauvaise clé : {err}");
}

/// Le root du fil et celui du manifeste sont **le même** — sans quoi un consommateur qui fait
/// confiance au fil et un auditeur qui lit le bundle croiraient deux choses différentes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_wire_root_and_the_manifest_root_agree() {
    let (svc, _statics) = serve_with_provenance().await;
    let resp = svc
        .iterate(Request::new(common::iterate_request_with_receipt(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("iterate")
        .into_inner();
    let (_, manifest_json) = bundle_of(&resp);

    let m = verify_meta_mini::json::parse_object(manifest_json.trim()).expect("manifeste lisible");
    assert_eq!(
        m.get_str("root").expect("root"),
        resp.meta_provenance_root,
        "le root du manifeste EST celui du fil",
    );
    assert!(m.get_u64("n_steps").expect("n_steps") > 0, "au moins un pas scellé");
}

/// **Non-interférence, au niveau SERVI.** La bibliothèque teste déjà que le sink n'altère pas le
/// numérique ; ce test-ci teste le *câblage d'ici* — que la branche ajoutée dans `run_iterate` n'a
/// pas changé le consensus rendu. Une provenance qui déplacerait le résultat d'un ULP serait pire
/// qu'une provenance absente : elle rendrait le reçu authentique d'un consensus qui n'est pas
/// celui qu'on aurait servi sans elle.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn provenance_does_not_change_the_numerics() {
    // DEUX JEUX DE PARTICIPANTS DISTINCTS, aux mêmes (kappa, seed).
    //
    // Les participants sont **stateful d'un Iterate à l'autre** — c'est pour cela que `Reset`
    // existe. Partager un seul jeu entre les deux rondes ferait démarrer la seconde depuis le
    // point fixe laissé par la première : 28 itérations contre 1, et une différence qu'on aurait
    // imputée au scellement. (Ce n'est pas une hypothèse : la première version de ce test est
    // tombée exactement là.) Des états initiaux identiques viennent des seeds identiques, donc
    // deux jeux frais sont numériquement comparables sans être couplés.
    let statics_plain = spawn_three().await;
    let statics_sealed = spawn_three().await;

    let plain = ControllerService::new(common::build_controller_state_for(&statics_plain, 10, DIM));
    let sealed = {
        let mut st = common::build_controller_state_for(&statics_sealed, 10, DIM);
        st.set_provenance(std::sync::Arc::new(ProvenanceConfig::new(
            HmacKeySource::test_key(),
            TenantId::system(),
            "test_key() (test)",
            true,
        )));
        ControllerService::new(st)
    };

    let a = plain
        .iterate(Request::new(common::iterate_request(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("sans provenance")
        .into_inner();
    let b = sealed
        .iterate(Request::new(common::iterate_request(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("avec provenance")
        .into_inner();

    assert_eq!(a.iterations, b.iterations, "même nombre d'itérations");
    assert_eq!(a.stop_reason, b.stop_reason, "même raison d'arrêt");
    assert_eq!(
        a.final_residual.to_bits(),
        b.final_residual.to_bits(),
        "résidu final BIT-À-BIT identique",
    );
    let (ca, cb) = (a.meta_consensus.expect("consensus"), b.meta_consensus.expect("consensus"));
    assert_eq!(
        ca.data.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        cb.data.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "consensus BIT-À-BIT identique avec et sans scellement",
    );
}

/// Sans provenance configurée, le fil porte un root **vide** — pas un placeholder, pas un zéro
/// hexadécimal plausible. L'absence de reçu doit se lire comme une absence.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_provenance_the_root_is_empty_never_a_placeholder() {
    let statics = spawn_three().await;
    let svc = ControllerService::new(common::build_controller_state_for(&statics, 10, DIM));

    let resp = svc
        .iterate(Request::new(common::iterate_request_with_receipt(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("iterate")
        .into_inner();

    assert!(
        resp.meta_provenance_root.is_empty(),
        "root VIDE attendu, pas un placeholder : {:?}",
        resp.meta_provenance_root,
    );
    assert!(
        resp.meta_provenance_bundle.is_none(),
        "on ne fabrique pas un bundle sans clé, même s'il est demandé",
    );
}

/// Le root est **toujours** là quand la provenance est active (il ne coûte rien) ; le bundle ne
/// vient que sur demande (il coûte O(itérations × dim)). Les deux moitiés comptent : oublier la
/// première priverait tout appelant de la revendication, oublier la seconde ferait payer le reçu
/// complet à qui ne le re-prouvera jamais.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bundle_is_opt_in_but_the_root_is_not() {
    let (svc, _statics) = serve_with_provenance().await;

    let resp = svc
        .iterate(Request::new(common::iterate_request(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("iterate")
        .into_inner();

    assert_eq!(resp.meta_provenance_root.len(), 64, "le root voyage TOUJOURS quand la provenance est active");
    assert!(resp.meta_provenance_bundle.is_none(), "…mais le bundle n'est PAS envoyé sans demande");
}

// =====================================================================
// Couche 3 v4 — le reçu NOMME ses acteurs et sa règle
// =====================================================================

/// **Le test du jalon v4.** Le reçu servi ne dit plus « la position 1 a contribué *h* » : il dit
/// **quel agent**, sous **quelle configuration déclarée**, et sous **quelle règle**. C'est la
/// différence entre une trace mathématique et un enregistrement qu'un auditeur procurement peut
/// exploiter.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_served_receipt_names_its_participants_and_its_rule() {
    let (svc, _statics) = serve_with_provenance().await;
    let resp = svc
        .iterate(Request::new(common::iterate_request_with_receipt(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("iterate")
        .into_inner();
    let (steps_jsonl, manifest_json) = bundle_of(&resp);

    let m = verify_meta_mini::json::parse_object(manifest_json.trim()).expect("manifeste lisible");
    assert_eq!(m.get_u64("schema_version").expect("schema"), 4, "une liaison ⟹ schéma 4");

    let b = verify_meta_mini::verify::Binding::from_manifest(&m)
        .expect("liaison lisible")
        .expect("le manifeste servi PORTE la liaison");

    // Les trois participants sont NOMMÉS, dans l'ordre où ils votent.
    assert_eq!(
        b.participant_ids,
        vec!["supplier-scoring".to_string(), "contract-risk".into(), "spend-categorization".into()],
        "le reçu nomme ses acteurs, en ordre",
    );
    // La règle d'agrégation est scellée — elle ne peut plus changer en silence d'une ronde à l'autre.
    assert_eq!(b.operator, "uniform");
    // Les participants sont des mocks ici : aucune configuration à attester, et cela se LIT
    // (colonne vide) au lieu d'être deviné.
    assert!(b.declared_config_hashes.iter().all(String::is_empty));
    assert_eq!(b.declared_config_hashes.len(), b.participant_ids.len(), "colonnes alignées");

    // Et le tiers re-prouve l'ensemble — chaîne ET liaison.
    third_party_reproves(steps_jsonl, manifest_json, &tenant_key()).expect("re-preuve complète");
}

/// **Le contrôle négatif du jalon.** Réécrire une identité — ou permuter deux agents — dans le
/// manifeste CASSE la re-preuve chez le tiers. Sans cela, nommer les acteurs serait décoratif :
/// un auditeur lirait des noms falsifiés sous un root parfaitement valide.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_third_party_rejects_a_receipt_whose_identities_were_rewritten() {
    let (svc, _statics) = serve_with_provenance().await;
    let resp = svc
        .iterate(Request::new(common::iterate_request_with_receipt(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("iterate")
        .into_inner();
    let (steps_jsonl, manifest_json) = bundle_of(&resp);
    let key = tenant_key();

    // Préalable : intact ⟹ passe. Si ce préalable tombait, le reste serait vide de sens.
    third_party_reproves(steps_jsonl, manifest_json, &key).expect("intact ⟹ intègre");

    // (a) RENOMMER un agent.
    let renamed = manifest_json.replacen("supplier-scoring", "supplier-scoring-EVIL", 1);
    assert_ne!(renamed, *manifest_json, "l'altération DOIT mordre — sinon ce test ne teste rien");
    let err = third_party_reproves(steps_jsonl, &renamed, &key).expect_err("renommage refusé");
    assert!(err.contains("liaison"), "le refus nomme la liaison : {err}");

    // (b) Changer la RÈGLE d'agrégation — le cas qu'aucune trace positionnelle ne pourrait voir.
    let rule = manifest_json.replacen("\"uniform\"", "\"attention\"", 1);
    assert_ne!(rule, *manifest_json, "l'altération DOIT mordre");
    third_party_reproves(steps_jsonl, &rule, &key).expect_err("changement de règle refusé");

    // (c) RETIRER la liaison en gardant les pas — l'effacement doit être aussi visible que la
    //     falsification, sinon il suffirait de supprimer les noms pour redevenir « intègre ».
    let stripped: String = manifest_json
        .lines()
        .filter(|l| !l.contains("binding_"))
        .collect::<Vec<_>>()
        .join("\n");
    third_party_reproves(steps_jsonl, &stripped, &key).expect_err("effacer la liaison est refusé");
}

/// **Le test que mon propre commentaire promettait, et qui manquait.** Le nom scellé dans la
/// liaison doit être celui de l'opérateur RÉELLEMENT appliqué — sinon le reçu attesterait une
/// règle d'agrégation qui n'est pas celle qui a combiné les votes, ce qui est précisément le
/// mensonge que ce champ existe pour empêcher.
///
/// La dérive est désormais structurellement impossible (`wired_meta_operator` donne à l'opérateur
/// le nom scellé), mais on l'épingle quand même : c'est cette propriété-là qui compte, pas la
/// façon dont elle est obtenue aujourd'hui.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sealed_rule_is_the_one_actually_applied() {
    let (svc, _statics) = serve_with_provenance().await;
    let resp = svc
        .iterate(Request::new(common::iterate_request_with_receipt(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("iterate")
        .into_inner();
    let (_, manifest_json) = bundle_of(&resp);

    let m = verify_meta_mini::json::parse_object(manifest_json.trim()).expect("manifeste lisible");
    let b = verify_meta_mini::verify::Binding::from_manifest(&m).expect("lisible").expect("présente");

    // L'opérateur effectivement câblé par le contrôleur porte CE nom.
    let wired = efe_meta_swarm::prelude::UniformMetaOp::with_name("uniform", DIM);
    let applied = efe_meta_swarm::prelude::MetaOperator::name(&wired).to_string();
    assert_eq!(
        b.operator, applied,
        "le nom SCELLÉ doit être celui de l'opérateur APPLIQUÉ, pas une étiquette parallèle",
    );
}
