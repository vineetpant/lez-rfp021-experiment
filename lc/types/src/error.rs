//! Every refusal is one of these codes, the same in the program, the client's dry run and the
//! docs (U.5).

use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "helios")]
use helios_consensus_core::errors::ConsensusError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum Error {
    Malformed = 1,
    NotFinalized = 2,
    BadSignature = 3,
    HandoffNotVerified = 4,
    BelowParticipation = 5,
    CheckpointStale = 6,
    BadInclusion = 7,
    PredicateFalse = 8,
    ChainMismatch = 9,
    AlreadyApplied = 10,
    BadExecutionBranch = 11,
    BadCommitteeBranch = 12,
    CommitteeMismatch = 13,
    CheckpointMismatch = 14,
}

#[cfg(feature = "helios")]
impl Error {
    /// The code for a refusal from Helios's consensus checks.
    pub fn from_helios(e: &eyre::Report) -> Self {
        match e.downcast_ref::<ConsensusError>() {
            Some(ConsensusError::InsufficientParticipation) => Self::BelowParticipation,
            Some(ConsensusError::InvalidPeriod) => Self::HandoffNotVerified,
            Some(ConsensusError::NotRelevant) => Self::AlreadyApplied,
            Some(ConsensusError::InvalidFinalityProof) => Self::NotFinalized,
            Some(ConsensusError::InvalidNextSyncCommitteeProof | ConsensusError::InvalidCurrentSyncCommitteeProof) => {
                Self::BadCommitteeBranch
            }
            Some(ConsensusError::InvalidExecutionPayloadProof) => Self::BadExecutionBranch,
            Some(ConsensusError::InvalidSignature) => Self::BadSignature,
            Some(ConsensusError::InvalidHeaderHash(..)) => Self::CheckpointMismatch,
            Some(ConsensusError::CheckpointTooOld) => Self::CheckpointStale,
            _ => Self::Malformed,
        }
    }
}
