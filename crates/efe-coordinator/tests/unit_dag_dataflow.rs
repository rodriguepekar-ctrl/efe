// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration : data flow parent→enfant + per-node timeout.

use std::collections::HashMap;
use std::time::Duration;

use efe_coordinator::{DagExecutor, DagNode, NodeStatus};

#[tokio::test]
async fn child_reads_parent_output() {
    let exec = DagExecutor::default();
    let a = DagNode::root("a", || async { Ok(serde_json::json!({"intent": "FACTUAL"})) });
    let b = DagNode::with_parents_and_inputs("b", ["a"], |inputs| async move {
        let intent = inputs["a"]["intent"].as_str().unwrap().to_string();
        Ok(serde_json::json!({"echo": intent}))
    });
    let r = exec.run(vec![a, b]).await.unwrap();
    assert!(r.all_success());
    assert_eq!(r.output("b").unwrap()["echo"], "FACTUAL");
}

#[tokio::test]
async fn child_reads_multiple_parents() {
    let exec = DagExecutor::default();
    let a = DagNode::root("a", || async { Ok(serde_json::json!({"value": 10})) });
    let b = DagNode::root("b", || async { Ok(serde_json::json!({"value": 20})) });
    let c = DagNode::with_parents_and_inputs("c", ["a", "b"], |inputs| async move {
        let va = inputs["a"]["value"].as_i64().unwrap();
        let vb = inputs["b"]["value"].as_i64().unwrap();
        Ok(serde_json::json!({"sum": va + vb}))
    });
    let r = exec.run(vec![a, b, c]).await.unwrap();
    assert_eq!(r.output("c").unwrap()["sum"], 30);
}

#[tokio::test]
async fn skip_propagates_through_inputs_node() {
    let exec = DagExecutor::default();
    let bad = DagNode::root("bad", || async {
        Err::<serde_json::Value, _>(efe_coordinator::CoordError::Config("boom".into()))
    });
    let consumer = DagNode::with_parents_and_inputs("consumer", ["bad"], |_inputs| async move {
        // Cette closure ne doit JAMAIS être appelée.
        panic!("must not run when parent failed");
    });
    let r = exec.run(vec![bad, consumer]).await.unwrap();
    assert_eq!(r.status("bad"), Some(&NodeStatus::Failed));
    assert_eq!(r.status("consumer"), Some(&NodeStatus::Skipped));
}

#[tokio::test]
async fn node_timeout_marks_timed_out() {
    let exec = DagExecutor::default();
    let slow = DagNode::root("slow", || async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(serde_json::json!("never"))
    })
    .with_timeout(Duration::from_millis(50));
    let r = exec.run(vec![slow]).await.unwrap();
    assert_eq!(r.status("slow"), Some(&NodeStatus::TimedOut));
    assert!(r.any_failure);
    assert!(r.errors.get("slow").unwrap().contains("timeout"));
}

#[tokio::test]
async fn timeout_propagates_skip_to_children() {
    let exec = DagExecutor::default();
    let slow = DagNode::root("slow", || async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(serde_json::json!(0))
    })
    .with_timeout(Duration::from_millis(20));
    let child = DagNode::with_parents("child", ["slow"], || async { Ok(serde_json::json!(0)) });
    let r = exec.run(vec![slow, child]).await.unwrap();
    assert_eq!(r.status("slow"), Some(&NodeStatus::TimedOut));
    assert_eq!(r.status("child"), Some(&NodeStatus::Skipped));
}

#[tokio::test]
async fn node_finishes_under_timeout() {
    let exec = DagExecutor::default();
    let fast = DagNode::root("fast", || async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        Ok(serde_json::json!("ok"))
    })
    .with_timeout(Duration::from_millis(200));
    let r = exec.run(vec![fast]).await.unwrap();
    assert_eq!(r.status("fast"), Some(&NodeStatus::Success));
    assert!(r.all_success());
}

#[tokio::test]
async fn data_flow_with_timeout_combined() {
    let exec = DagExecutor::default();
    let a = DagNode::root("a", || async { Ok(serde_json::json!({"x": 42})) })
        .with_timeout(Duration::from_millis(500));
    let b = DagNode::with_parents_and_inputs("b", ["a"], |inputs| async move {
        let x = inputs["a"]["x"].as_i64().unwrap();
        Ok(serde_json::json!({"x_doubled": x * 2}))
    })
    .with_timeout(Duration::from_millis(500));
    let r = exec.run(vec![a, b]).await.unwrap();
    assert!(r.all_success());
    assert_eq!(r.output("b").unwrap()["x_doubled"], 84);
}

#[tokio::test]
async fn with_inputs_root_receives_empty_map() {
    let exec = DagExecutor::default();
    let root = DagNode::with_inputs("root", |inputs: HashMap<String, _>| async move {
        Ok(serde_json::json!({"parent_count": inputs.len()}))
    });
    let r = exec.run(vec![root]).await.unwrap();
    assert_eq!(r.output("root").unwrap()["parent_count"], 0);
}

#[tokio::test]
async fn diamond_data_flow_aggregates() {
    // a → {b, c} → d (d reçoit b.value et c.value).
    let exec = DagExecutor::default();
    let a = DagNode::root("a", || async { Ok(serde_json::json!({"base": 5})) });
    let b = DagNode::with_parents_and_inputs("b", ["a"], |inputs| async move {
        let base = inputs["a"]["base"].as_i64().unwrap();
        Ok(serde_json::json!({"value": base * 2}))
    });
    let c = DagNode::with_parents_and_inputs("c", ["a"], |inputs| async move {
        let base = inputs["a"]["base"].as_i64().unwrap();
        Ok(serde_json::json!({"value": base * 3}))
    });
    let d = DagNode::with_parents_and_inputs("d", ["b", "c"], |inputs| async move {
        let bv = inputs["b"]["value"].as_i64().unwrap();
        let cv = inputs["c"]["value"].as_i64().unwrap();
        Ok(serde_json::json!({"sum": bv + cv}))
    });
    let r = exec.run(vec![a, b, c, d]).await.unwrap();
    assert!(r.all_success());
    assert_eq!(r.output("d").unwrap()["sum"], 10 + 15);
}
