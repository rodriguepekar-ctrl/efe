// SPDX-License-Identifier: UNLICENSED
//! Tests `Decomposer`.

use efe_multi_llm::{Decomposer, DecomposerMode, MultiLlmError};
use efe_nano_app_coordinator::types::router::TaskClass;

#[test]
fn decomposer_splits_3_subtasks() {
    let d = Decomposer::default();
    let tasks = d
        .decompose("Translate this to Rust. Then explain it. Then write a haiku.")
        .unwrap();
    assert_eq!(tasks.len(), 3);
}

#[test]
fn classifies_code_translation_creative() {
    let d = Decomposer::default();
    let tasks = d
        .decompose("Translate to Rust. Explain semantics. Write a haiku.")
        .unwrap();
    assert_eq!(tasks[0].task_class, TaskClass::Translation);
    assert_eq!(tasks[1].task_class, TaskClass::Analysis);
    assert_eq!(tasks[2].task_class, TaskClass::Creative);
}

#[test]
fn empty_text_errors() {
    let d = Decomposer::default();
    assert!(matches!(d.decompose(""), Err(MultiLlmError::Config(_))));
}

#[test]
fn single_phrase_works() {
    let d = Decomposer::default();
    let t = d.decompose("Explain quantum entanglement.").unwrap();
    assert_eq!(t.len(), 1);
}

#[test]
fn ids_sequential_from_1() {
    let d = Decomposer::default();
    let tasks = d.decompose("First task. Second task. Third task.").unwrap();
    for (i, t) in tasks.iter().enumerate() {
        assert_eq!(t.id, (i + 1) as u32);
    }
}

#[test]
fn llm_backed_mode_falls_back_to_heuristic() {
    let d = Decomposer::new(DecomposerMode::LlmBacked);
    let t = d.decompose("Explain X. Build Y.").unwrap();
    assert!(!t.is_empty());
}

#[test]
fn capped_at_max_sub_tasks() {
    let d = Decomposer::default();
    let text = (0..20)
        .map(|i| format!("Task {i} does something specific."))
        .collect::<Vec<_>>()
        .join(" ");
    let tasks = d.decompose(&text).unwrap();
    assert!(tasks.len() <= 10);
}

#[test]
fn factual_qa_default_for_unmatched() {
    let d = Decomposer::default();
    let tasks = d.decompose("What is the capital of France?").unwrap();
    assert_eq!(tasks[0].task_class, TaskClass::FactualQa);
}
