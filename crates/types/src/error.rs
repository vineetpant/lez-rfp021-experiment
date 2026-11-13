//! Every refusal is one of these codes, the same in the program, the client's dry run and the
//! docs. They follow the failure modes RFP-021's U.5 lists, grouped by where they arise: an
//! update or bootstrap (1xx), an attestation (2xx), or the input itself.

use borsh::{BorshDeserialize, BorshSerialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[borsh(use_discriminant = true)]
#[repr(u16)]
pub enum Refusal {
    /// The input doesn't decode, or asks for something it doesn't carry.
    MalformedInput = 1,

    /// The finalized header isn't proven under the attested header's state.
    UnfinalizedHeader = 100,
    /// The sync-committee signature doesn't verify.
    InvalidSignature = 101,
    /// Fewer than 2/3 of the committee signed.
    InsufficientSigners = 102,
    /// The update is signed by, or hands off to, a committee the store didn't verify.
    UnverifiedHandoff = 103,
    /// The committee keys supplied don't hash to the root the store holds.
    CommitteeKeysMismatch = 104,
    /// A committee isn't proven under the header's state.
    InvalidCommitteeProof = 105,
    /// A header's execution payload or block hash isn't proven under its body.
    InvalidExecutionProof = 106,
    /// The store is past its weak-subjectivity bound; the instance is replaced, not advanced.
    StaleStore = 107,
    /// The update doesn't advance the store: already applied, or older.
    UpdateNotNewer = 108,
    /// The bootstrap header doesn't hash to the checkpoint.
    CheckpointRootMismatch = 120,

    /// The inclusion proof doesn't verify against the record's root.
    InvalidInclusionProof = 200,
    /// The proof verifies, but a condition on the value doesn't hold.
    ConditionUnsatisfied = 201,
    /// The claim names another chain than the record's.
    WrongChain = 202,
}
