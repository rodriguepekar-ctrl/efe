//! State machines for erasure + portability.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErasureState {
    /// Initial : waiting for MFA email verification.
    MfaPending,
    /// MFA verified ; in 7-day grace window. Cancellable.
    Grace,
    /// Grace expired ; cascade worker running.
    InProgress,
    /// Cascade completed.
    Done,
    /// Cancelled during grace.
    Cancelled,
}

impl ErasureState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MfaPending => "mfa_pending",
            Self::Grace => "grace",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }
    /// Allowed transitions :
    ///   mfa_pending → (grace | cancelled)
    ///   grace → (in_progress | cancelled)
    ///   in_progress → (done)
    ///   done | cancelled = sink
    pub fn can_transition_to(self, next: Self) -> bool {
        use ErasureState::*;
        matches!(
            (self, next),
            (MfaPending, Grace)
                | (MfaPending, Cancelled)
                | (Grace, InProgress)
                | (Grace, Cancelled)
                | (InProgress, Done)
        )
    }
}

impl FromStr for ErasureState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mfa_pending" => Ok(Self::MfaPending),
            "grace" => Ok(Self::Grace),
            "in_progress" => Ok(Self::InProgress),
            "done" => Ok(Self::Done),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(format!("unknown erasure state : {}", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortabilityState {
    Queued,
    Building,
    Ready,
    Expired,
    Failed,
}

impl PortabilityState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Building => "building",
            Self::Ready => "ready",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Expired | Self::Failed)
    }
    pub fn can_transition_to(self, next: Self) -> bool {
        use PortabilityState::*;
        matches!(
            (self, next),
            (Queued, Building)
                | (Queued, Failed)
                | (Building, Ready)
                | (Building, Failed)
                | (Ready, Expired)
        )
    }
}

impl FromStr for PortabilityState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "queued" => Ok(Self::Queued),
            "building" => Ok(Self::Building),
            "ready" => Ok(Self::Ready),
            "expired" => Ok(Self::Expired),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown portability state : {}", other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erasure_transitions() {
        use ErasureState::*;
        assert!(MfaPending.can_transition_to(Grace));
        assert!(MfaPending.can_transition_to(Cancelled));
        assert!(Grace.can_transition_to(InProgress));
        assert!(Grace.can_transition_to(Cancelled));
        assert!(InProgress.can_transition_to(Done));
        // Forbidden :
        assert!(!MfaPending.can_transition_to(InProgress));
        assert!(!MfaPending.can_transition_to(Done));
        assert!(!Done.can_transition_to(Grace));
        assert!(!Cancelled.can_transition_to(Grace));
        assert!(!InProgress.can_transition_to(Cancelled)); // too late to cancel
    }

    #[test]
    fn portability_transitions() {
        use PortabilityState::*;
        assert!(Queued.can_transition_to(Building));
        assert!(Building.can_transition_to(Ready));
        assert!(Ready.can_transition_to(Expired));
        // Forbidden :
        assert!(!Queued.can_transition_to(Ready));
        assert!(!Building.can_transition_to(Expired));
        assert!(!Failed.can_transition_to(Building));
    }

    #[test]
    fn terminal_sets() {
        assert!(ErasureState::Done.is_terminal());
        assert!(ErasureState::Cancelled.is_terminal());
        assert!(!ErasureState::Grace.is_terminal());

        assert!(PortabilityState::Expired.is_terminal());
        assert!(PortabilityState::Failed.is_terminal());
        assert!(!PortabilityState::Ready.is_terminal());
    }

    #[test]
    fn from_str_roundtrip() {
        for s in &["mfa_pending", "grace", "in_progress", "done", "cancelled"] {
            let st: ErasureState = s.parse().unwrap();
            assert_eq!(st.as_str(), *s);
        }
        assert!("bogus".parse::<ErasureState>().is_err());
    }
}
