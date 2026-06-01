//! Individual recovery strategies implementing [`crate::collapse::CollapseStrategy`].

pub mod abort;
pub mod centroid_project;
pub mod noise;
pub mod partial_respawn;
pub mod rollback;
pub mod stub;

pub use abort::FullAbortStrategy;
pub use centroid_project::CentroidProjectionStrategy;
pub use noise::NoiseInjectionStrategy;
pub use partial_respawn::PartialRespawnStrategy;
pub use rollback::SoftRollbackStrategy;
pub use stub::{StubBehavior, StubStrategy};
