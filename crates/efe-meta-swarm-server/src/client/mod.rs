//! gRPC client adapters used by the controller to talk to
//! participants.
//!
//! PR #8 ships [`RemoteParticipant`], a thin wrapper around
//! `MetaParticipantServiceClient` that implements the Phase-1
//! [`SwarmParticipant`] trait so the existing in-process algorithm
//! can run unchanged over the network.
//!
//! [`SwarmParticipant`]: efe_meta_swarm::participants::SwarmParticipant

mod remote_participant;

pub use remote_participant::{proto_to_tensor, tensor_to_proto, RemoteParticipant};
