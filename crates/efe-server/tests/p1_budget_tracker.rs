// SPDX-License-Identifier: UNLICENSED
//! Sprint **D19 (P1-Foundation)** — tests d'intégration
//! [`BudgetTracker`].
//!
//! Cf. `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 7.2 (deadline
//! propagation) et § 6.4 (tests d'invariance).
//!
//! Les tests inline du module (`src/middleware/budget_tracker.rs`)
//! couvrent l'API basique. Ce fichier-ci pin **les comportements
//! d'invariance contractuels** qui sont opposables : ce sont eux
//! qui transforment le tracker en garantie produit.

use std::thread::sleep;
use std::time::{Duration, Instant};

use efe_server::middleware::{BudgetConfig, BudgetError, BudgetTracker};
use efe_telemetry::metrics::p1_instruments::Stage;

// ====================================================================
// Invariants contractuels (§ 6.4)
// ====================================================================

#[test]
fn p1_solo_request_does_not_wait_for_batch() {
    // Pin contractuel : une requête isolée (queue vide chez le
    // coalescer) ne paye PAS la fenêtre `max_wait_micros`. Ce test
    // émule le comportement attendu côté tracker — l'étage 5 doit
    // pouvoir consommer < 1 ms si l'inférence est immédiate.
    let mut t = BudgetTracker::new(BudgetConfig {
        emit_overruns: false,
        ..BudgetConfig::default()
    });
    // Simule les étages 1-4 légers
    for s in [
        Stage::GrpcRecv,
        Stage::GrpcDecode,
        Stage::Interceptors,
        Stage::Tokenize,
    ] {
        sleep(Duration::from_micros(100));
        let d = t.checkpoint(s).expect("ok");
        assert!(
            d < Duration::from_millis(5),
            "{:?} took too long: {:?}",
            s,
            d
        );
    }
    // Pas d'attente batch → étage 5 immédiat
    let d = t.checkpoint_inference(1).expect("ok");
    assert!(
        d < Duration::from_millis(2),
        "solo inference should not wait for batch (got {:?})",
        d
    );
}

#[test]
fn p1_warm_path_zero_alloc_in_steady_state_contract() {
    // Ce test ne vérifie PAS l'absence d'allocations (ça c'est le
    // job de `tests/p1_hot_path_lint.rs`). Il vérifie le contrat
    // de surface du tracker : aucune méthode publique du
    // `BudgetTracker` ne doit allouer sur le hot-path. On le pin
    // en s'assurant qu'on peut faire 1000 checkpoints back-to-back
    // sans dépasser 5 ms total (largement compatible avec 0 alloc).
    let mut total = Duration::ZERO;
    for _ in 0..50 {
        let mut t = BudgetTracker::new(BudgetConfig {
            emit_overruns: false,
            ..BudgetConfig::default()
        });
        let start = Instant::now();
        // 50 itérations × les 12 étages = 600 checkpoints. Si une
        // alloc se cache dans le `checkpoint`, ça apparaîtra.
        // (Le bitset .visited réinitialise par tracker, donc on
        // ne dépasse pas 12 par tracker.)
        for s in *Stage::all() {
            let _ = t.checkpoint(s);
        }
        total += start.elapsed();
        t.finish();
    }
    // 50 trackers × 12 checkpoints = 600 ops totales.
    // Marge généreuse : 50 ms (= ~83 µs / op) ; suffit largement
    // si pas d'alloc.
    assert!(
        total < Duration::from_millis(50),
        "50 trackers × 12 checkpoints took {:?} (> 50 ms suspect alloc)",
        total
    );
}

#[test]
fn p1_cancel_propagates_within_5ms() {
    // Pin contractuel : si la deadline est dépassée, le prochain
    // `allocate_for_stage` doit retourner DeadlineExceeded
    // **immédiatement** (< 5 ms).
    let cfg = BudgetConfig {
        hard_deadline: Duration::from_millis(10),
        ..BudgetConfig::default()
    };
    let t = BudgetTracker::new(cfg);
    sleep(Duration::from_millis(15));
    let start = Instant::now();
    let err = t.allocate_for_stage(Stage::Fusion).unwrap_err();
    let elapsed = start.elapsed();
    assert!(matches!(err, BudgetError::DeadlineExceeded { .. }));
    assert!(
        elapsed < Duration::from_millis(5),
        "cancel took {:?} (> 5 ms contract)",
        elapsed
    );
}

#[test]
fn p1_handshake_excluded_from_budget_contract() {
    // Pin contractuel : `BudgetTracker::new` démarre le timer
    // **maintenant** ; tout ce qui se passe avant (handshake mTLS,
    // par exemple) est **hors** budget. C'est crucial pour
    // l'opposabilité juridique : on ne facture pas au client les
    // 40-60 ms du handshake initial.
    // On le pin en simulant un handshake d'entrée puis en
    // mesurant que le tracker a bien commencé après.
    sleep(Duration::from_millis(50)); // simulation handshake
    let t = BudgetTracker::new(BudgetConfig::default());
    let r0 = t.remaining();
    // r0 doit être proche de 250 ms (la totalité du budget), pas
    // 250 - 50 = 200 ms.
    assert!(
        r0 > Duration::from_millis(248),
        "tracker should start AFTER handshake, got remaining={:?}",
        r0
    );
}

// ====================================================================
// Tests d'ordre canonique
// ====================================================================

#[test]
fn checkpoint_in_canonical_order_succeeds() {
    let mut t = BudgetTracker::new(BudgetConfig {
        emit_overruns: false,
        hard_deadline: Duration::from_secs(60), // pas de pression deadline
        ..BudgetConfig::default()
    });
    for s in *Stage::all() {
        t.checkpoint(s)
            .expect(&format!("stage {} should succeed", s.as_label()));
    }
    assert_eq!(t.visited_count(), 12);
    t.finish();
}

#[test]
fn out_of_order_checkpoints_are_allowed_but_visited_set_is_correct() {
    // Le tracker accepte un ordre quelconque (= un bug d'ordre
    // côté handler ne crash pas le service). Mais le bitset
    // `visited` reste correct.
    let mut t = BudgetTracker::new(BudgetConfig {
        emit_overruns: false,
        ..BudgetConfig::default()
    });
    t.checkpoint(Stage::Fusion).expect("ok");
    t.checkpoint(Stage::Genesis).expect("ok");
    t.checkpoint(Stage::Tokenize).expect("ok");
    assert_eq!(t.visited_count(), 3);
}

#[test]
fn double_checkpoint_at_any_stage_returns_error() {
    for stage in *Stage::all() {
        let mut t = BudgetTracker::new(BudgetConfig {
            emit_overruns: false,
            ..BudgetConfig::default()
        });
        t.checkpoint(stage).expect("first ok");
        let err = t.checkpoint(stage).unwrap_err();
        match err {
            BudgetError::DoubleCheckpoint { stage: s } => {
                assert_eq!(s, stage.as_label());
            }
            other => panic!("expected DoubleCheckpoint for {:?}, got {:?}", stage, other),
        }
    }
}

// ====================================================================
// Tests de propagation
// ====================================================================

#[test]
fn snapshot_remaining_stays_consistent_across_threads() {
    use std::sync::Arc;
    use std::thread;
    let t = BudgetTracker::new(BudgetConfig::default());
    let snap = Arc::new(t.snapshot());
    let snap1 = Arc::clone(&snap);
    let h = thread::spawn(move || {
        sleep(Duration::from_millis(5));
        snap1.remaining()
    });
    let local_r = snap.remaining();
    let thread_r = h.join().unwrap();
    // Le snapshot doit voir le temps avancer (le restant doit avoir
    // décrémenté d'au moins 5 ms).
    let diff = local_r.as_secs_f64() - thread_r.as_secs_f64();
    assert!(
        diff >= 0.004,
        "snapshot didn't see time advance: local={:?}, thread={:?}",
        local_r,
        thread_r
    );
    t.finish();
}

#[test]
fn with_deadline_truncates_when_client_is_stricter() {
    let server_cfg = BudgetConfig {
        hard_deadline: Duration::from_millis(250),
        ..BudgetConfig::default()
    };
    let client_dl = Instant::now() + Duration::from_millis(50);
    let t = BudgetTracker::with_deadline(server_cfg, client_dl);
    let r = t.remaining();
    assert!(r <= Duration::from_millis(50));
    assert!(r > Duration::from_millis(45));
    t.finish();
}

// ====================================================================
// Tests de mode multi-3
// ====================================================================

#[test]
fn mode2_tracker_has_1800ms_budget() {
    let t = BudgetTracker::new(BudgetConfig::mode2());
    assert_eq!(t.mode(), "mode2");
    assert!(t.remaining() <= Duration::from_millis(1800));
    assert!(t.remaining() > Duration::from_millis(1795));
    t.finish();
}

#[test]
fn mode3_tracker_has_700ms_budget() {
    let t = BudgetTracker::new(BudgetConfig::mode3());
    assert_eq!(t.mode(), "mode3");
    assert!(t.remaining() <= Duration::from_millis(700));
    assert!(t.remaining() > Duration::from_millis(695));
    t.finish();
}

// ====================================================================
// Tests de scénario réaliste (12 étages)
// ====================================================================

#[test]
fn realistic_p1_request_consumes_budget_proportionally() {
    // Simule un roundtrip P1 sain : chaque étage prend ~50 % de
    // son budget canonique. On vérifie qu'à la fin :
    //   - chaque `checkpoint` retourne une durée ≥ target sleep
    //     (le `sleep` ne peut pas finir plus tôt — invariant kernel)
    //   - chaque `checkpoint` retourne une durée ≤ target × 2 + 10 ms
    //     (tolérance large pour absorber le jitter scheduler sur CI
    //     qui peut sleeper 2× le target en charge ; on bound
    //     défensivement)
    //   - `visited_count == 12`
    //   - `remaining > 150 ms` (= 250 - 153/2 ≈ 173 ms - jitter)
    let mut t = BudgetTracker::new(BudgetConfig {
        emit_overruns: false,
        ..BudgetConfig::default()
    });
    for s in *Stage::all() {
        let budget = s.budget();
        let target = budget / 2;
        sleep(target);
        let observed = t
            .checkpoint(s)
            .expect(&format!("stage {} ok", s.as_label()));
        assert!(
            observed >= target,
            "stage {} observed={:?} should be ≥ target={:?}",
            s.as_label(),
            observed,
            target
        );
        // Bound supérieur défensif : 2× target + 10 ms de marge fixe
        // pour absorber les machines lentes / CI préemptive.
        let max_acceptable = target * 2 + Duration::from_millis(10);
        assert!(
            observed <= max_acceptable,
            "stage {} observed={:?} >> max_acceptable={:?} (CI super préemptive ?)",
            s.as_label(),
            observed,
            max_acceptable
        );
    }
    assert_eq!(t.visited_count(), 12);
    t.finish();
}
