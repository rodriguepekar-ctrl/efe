// SPDX-License-Identifier: UNLICENSED
//! Workflows multi-LLM Phase 3.

mod cascade;
mod debate;
mod relay;
mod specialist;
mod vote;

pub use cascade::run_cascade;
pub use debate::run_debate;
pub use relay::run_relay;
pub use specialist::run_specialist;
pub use vote::run_vote_3llm;
