// SPDX-License-Identifier: UNLICENSED
//! Tests `Aggregator`.

use efe_multi_llm::aggregate::aggregator::AggregateInput;
use efe_multi_llm::{Aggregator, AggregatorMode, MultiLlmError};

#[test]
fn aggregate_single_input() {
    let a = Aggregator::default();
    let out = a
        .aggregate(&[AggregateInput {
            llm_source: "claude".into(),
            heading: "x".into(),
            body: "Hello.".into(),
        }])
        .unwrap();
    assert_eq!(out.summary, "Hello.");
    assert_eq!(out.sections.len(), 1);
}

#[test]
fn aggregate_three_inputs_combines() {
    let a = Aggregator::default();
    let out = a
        .aggregate(&[
            AggregateInput {
                llm_source: "claude".into(),
                heading: "Translation".into(),
                body: "fn main() {}".into(),
            },
            AggregateInput {
                llm_source: "gemini".into(),
                heading: "Explanation".into(),
                body: "Rust main function.".into(),
            },
            AggregateInput {
                llm_source: "gpt".into(),
                heading: "Haiku".into(),
                body: "Code flows ...".into(),
            },
        ])
        .unwrap();
    assert_eq!(out.sections.len(), 3);
    assert_eq!(out.citations.len(), 3);
    assert!(out.summary.contains("3 sub-results"));
}

#[test]
fn divergent_outputs_get_uncertainty() {
    let a = Aggregator::default();
    let out = a
        .aggregate(&[
            AggregateInput {
                llm_source: "a".into(),
                heading: "h".into(),
                body: "alpha beta gamma".into(),
            },
            AggregateInput {
                llm_source: "b".into(),
                heading: "h".into(),
                body: "epsilon zeta eta".into(),
            },
        ])
        .unwrap();
    assert!(out.uncertainty.contains("diverge"));
}

#[test]
fn convergent_outputs_no_uncertainty() {
    let a = Aggregator::default();
    let out = a
        .aggregate(&[
            AggregateInput {
                llm_source: "a".into(),
                heading: "h".into(),
                body: "the same answer".into(),
            },
            AggregateInput {
                llm_source: "b".into(),
                heading: "h".into(),
                body: "the same answer".into(),
            },
        ])
        .unwrap();
    assert!(out.uncertainty.is_empty());
}

#[test]
fn empty_input_errors() {
    let a = Aggregator::default();
    assert!(matches!(a.aggregate(&[]), Err(MultiLlmError::Config(_))));
}

#[test]
fn citations_preserve_llm_source_exactly() {
    let a = Aggregator::default();
    let out = a
        .aggregate(&[AggregateInput {
            llm_source: "gemini_2_5_pro".into(),
            heading: "h".into(),
            body: "b".into(),
        }])
        .unwrap();
    assert_eq!(out.citations[0].llm_source, "gemini_2_5_pro");
}

#[test]
fn mode_llm_backed_falls_back_to_concat() {
    let a = Aggregator::new(AggregatorMode::LlmBacked);
    let out = a
        .aggregate(&[AggregateInput {
            llm_source: "x".into(),
            heading: "h".into(),
            body: "body".into(),
        }])
        .unwrap();
    assert_eq!(out.summary, "body");
}
