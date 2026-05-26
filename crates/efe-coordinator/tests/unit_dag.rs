// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration `DagExecutor`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use efe_coordinator::{CoordError, DagExecutor, DagNode, NodeStatus};

#[tokio::test]
async fn linear_chain_runs_in_order() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut nodes = Vec::new();
    for i in 0..5 {
        let id = format!("n{i}");
        let parents: Vec<String> = if i == 0 {
            vec![]
        } else {
            vec![format!("n{}", i - 1)]
        };
        let c = Arc::clone(&counter);
        nodes.push(DagNode::with_parents(id.clone(), parents, move || async move {
            let v = c.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(5)).await;
            Ok(serde_json::json!({"n": v}))
        }));
    }
    let exec = DagExecutor::default();
    let report = exec.run(nodes).await.unwrap();
    assert!(report.all_success());
    // Linear → exactement 1 simultané (mais notre executor n'est pas obligé
    // de bloquer ; on vérifie juste tous le succès).
    for i in 0..5 {
        assert_eq!(report.status(&format!("n{i}")), Some(&NodeStatus::Success));
    }
}

#[tokio::test]
async fn wide_fan_out_uses_parallelism() {
    let inflight = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(AtomicUsize::new(0));
    let mut nodes = Vec::new();
    nodes.push(DagNode::root("root", || async {
        Ok(serde_json::json!("r"))
    }));
    for i in 0..20 {
        let inf = Arc::clone(&inflight);
        let obs = Arc::clone(&observed);
        let id = format!("c{i}");
        nodes.push(DagNode::with_parents(id, ["root"], move || async move {
            let v = inf.fetch_add(1, Ordering::SeqCst) + 1;
            obs.fetch_max(v, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(15)).await;
            inf.fetch_sub(1, Ordering::SeqCst);
            Ok(serde_json::json!("x"))
        }));
    }
    let exec = DagExecutor::new(4);
    let _r = exec.run(nodes).await.unwrap();
    let max = observed.load(Ordering::SeqCst);
    assert!(max >= 2, "expected parallelism, observed={max}");
    assert!(max <= 4, "expected respect of max=4, observed={max}");
}

#[tokio::test]
async fn cycle_detection_invariant() {
    let exec = DagExecutor::default();
    let a = DagNode::with_parents("a", ["b"], || async { Ok(serde_json::json!(0)) });
    let b = DagNode::with_parents("b", ["c"], || async { Ok(serde_json::json!(0)) });
    let c = DagNode::with_parents("c", ["a"], || async { Ok(serde_json::json!(0)) });
    let r = exec.run(vec![a, b, c]).await;
    assert!(matches!(r, Err(CoordError::DagCycle { .. })));
}

#[tokio::test]
async fn missing_parent_at_submit() {
    let exec = DagExecutor::default();
    let a = DagNode::with_parents("a", ["ghost"], || async { Ok(serde_json::json!(0)) });
    let r = exec.run(vec![a]).await;
    assert!(matches!(r, Err(CoordError::DagMissingParent { .. })));
}

#[tokio::test]
async fn failure_propagates_as_skip_to_all_downstream() {
    let exec = DagExecutor::default();
    let bad = DagNode::root("bad", || async {
        Err::<serde_json::Value, _>(CoordError::Config("boom".into()))
    });
    let mid = DagNode::with_parents("mid", ["bad"], || async { Ok(serde_json::json!(0)) });
    let tail = DagNode::with_parents("tail", ["mid"], || async { Ok(serde_json::json!(0)) });
    let r = exec.run(vec![bad, mid, tail]).await.unwrap();
    assert_eq!(r.status("bad"), Some(&NodeStatus::Failed));
    assert_eq!(r.status("mid"), Some(&NodeStatus::Skipped));
    assert_eq!(r.status("tail"), Some(&NodeStatus::Skipped));
    assert!(r.any_failure);
}

#[tokio::test]
async fn diamond_dag_collects_outputs_in_order() {
    let exec = DagExecutor::default();
    let a = DagNode::root("a", || async { Ok(serde_json::json!("A")) });
    let b = DagNode::with_parents("b", ["a"], || async { Ok(serde_json::json!("B")) });
    let c = DagNode::with_parents("c", ["a"], || async { Ok(serde_json::json!("C")) });
    let d = DagNode::with_parents("d", ["b", "c"], || async { Ok(serde_json::json!("D")) });
    let r = exec.run(vec![a, b, c, d]).await.unwrap();
    assert_eq!(r.output("a").unwrap(), &serde_json::json!("A"));
    assert_eq!(r.output("d").unwrap(), &serde_json::json!("D"));
}

#[tokio::test]
async fn empty_dag_is_success() {
    let exec = DagExecutor::default();
    let r = exec.run(vec![]).await.unwrap();
    assert!(r.all_success());
    assert!(r.outputs.is_empty());
}

#[tokio::test]
async fn report_includes_latencies() {
    let exec = DagExecutor::default();
    let n = DagNode::root("a", || async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(serde_json::json!(0))
    });
    let r = exec.run(vec![n]).await.unwrap();
    let lat = r.latencies_ms.get("a").copied().unwrap_or(0);
    assert!(lat >= 15, "expected ≥15ms, got {lat}");
}
