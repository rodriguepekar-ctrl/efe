//! BYOK rotation state machine.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationState {
    Pending,
    Preparing,
    Reencrypting,
    Committing,
    Done,
    Cancelled,
    Failed,
}

impl RotationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Preparing => "preparing",
            Self::Reencrypting => "reencrypting",
            Self::Committing => "committing",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::Failed)
    }
    /// Allowed transitions.
    pub fn can_transition_to(self, next: Self) -> bool {
        use RotationState::*;
        matches!(
            (self, next),
            (Pending, Preparing)
                | (Pending, Cancelled)
                | (Preparing, Reencrypting)
                | (Preparing, Failed)
                | (Preparing, Cancelled)
                | (Reencrypting, Committing)
                | (Reencrypting, Failed)
                | (Committing, Done)
                | (Committing, Failed)
        )
    }
    /// Returns true if the rotation can still be safely cancelled.
    /// Cancellation after `Committing` is unsafe (new key already
    /// active for some rows).
    pub fn is_cancellable(&self) -> bool {
        matches!(self, Self::Pending | Self::Preparing)
    }
}

impl FromStr for RotationState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "preparing" => Ok(Self::Preparing),
            "reencrypting" => Ok(Self::Reencrypting),
            "committing" => Ok(Self::Committing),
            "done" => Ok(Self::Done),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown rotation state : {}", other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions() {
        use RotationState::*;
        assert!(Pending.can_transition_to(Preparing));
        assert!(Pending.can_transition_to(Cancelled));
        assert!(Preparing.can_transition_to(Reencrypting));
        assert!(Preparing.can_transition_to(Failed));
        assert!(Reencrypting.can_transition_to(Committing));
        assert!(Committing.can_transition_to(Done));
        // Forbidden :
        assert!(!Pending.can_transition_to(Done));
        assert!(!Done.can_transition_to(Reencrypting));
        // No cancel after committing :
        assert!(!Committing.is_cancellable());
        assert!(Pending.is_cancellable());
        assert!(Preparing.is_cancellable());
    }

    #[test]
    fn from_str_roundtrip() {
        for s in &[
            "pending",
            "preparing",
            "reencrypting",
            "committing",
            "done",
            "cancelled",
            "failed",
        ] {
            let st: RotationState = s.parse().unwrap();
            assert_eq!(st.as_str(), *s);
        }
    }
}
