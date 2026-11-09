//! Messages, state and checks of the `eth_lc` program, an Ethereum light client on LEZ.
//!
//! One deployed program serves any number of instances. An instance is the program's actor
//! state on one public account: the chain config it was bound to at `Init`, a compact
//! light-client store, and its latest record. Each verified header is also written once to its
//! own **record** account, at an address derived from the instance and the header's slot
//! ([`record_account`]), never rewritten or removed.
//!
//! The consensus checks are Helios's (`helios-consensus-core`); [`verify`] wraps them for a store
//! that keeps 32-byte committee roots instead of 512 keys, and enforces the 2/3 quorum as an
//! error. Merkle-Patricia proofs are checked with `alloy-trie` ([`mpt`]). Statements, predicates
//! and history proofs are ours ([`statement`], [`history`]).

pub mod error;
pub mod history;
pub mod mpt;
pub mod statement;
pub mod verify;

use alloy_primitives::B256;
use borsh::{BorshDeserialize, BorshSerialize};
use error::Error;
use helios_consensus_core::types::Forks;
use history::Ancestry;
use lee_core::account::{AccountId, Actor};
use lee_core::program::PdaSeed;
use sha2::{Digest, Sha256};
use statement::{Query, Statement};

pub type Root = [u8; 32];

/// What an instance is bound to at `Init`, never changed afterwards.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub genesis_validators_root: Root,
    pub genesis_time: u64,
    /// Helios's `Forks`, CBOR-encoded.
    pub forks: Vec<u8>,
    /// How long after its finalized slot a store may still be advanced, in seconds. Past it,
    /// the instance is outside the weak-subjectivity period and is replaced, not refreshed.
    pub max_store_age: u64,
}

impl ChainConfig {
    pub fn forks(&self) -> Result<Forks, Error> {
        serde_cbor::from_slice(&self.forks).map_err(|_| Error::Malformed)
    }

    pub fn hash(&self) -> Root {
        let mut h = Sha256::new();
        h.update(b"eth_lc/config/v1");
        h.update(borsh::to_vec(self).expect("config encodes"));
        h.finalize().into()
    }

    /// Unix time of `slot`, in seconds.
    pub fn slot_time(&self, slot: u64) -> u64 {
        self.genesis_time + slot * 12
    }

    /// When a store finalized at `slot` stops being advanceable, in milliseconds.
    pub fn stale_from_ms(&self, slot: u64) -> u64 {
        (self.slot_time(slot) + self.max_store_age) * 1000
    }
}

/// The light-client store an instance keeps. The committees are kept as their SSZ roots; an
/// update supplies the keys, and [`verify::update`] checks them against these.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Store {
    /// Helios's `LightClientHeader` of the finalized header, CBOR-encoded.
    pub finalized_header: Vec<u8>,
    pub finalized_slot: u64,
    pub current_committee: Root,
    pub next_committee: Option<Root>,
}

/// One finalized header as the program keeps it, and attestations are proven against.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct HeaderRecord {
    pub chain_id: u64,
    pub slot: u64,
    pub beacon_root: Root,
    pub block_number: u64,
    pub block_hash: Root,
    pub state_root: Root,
    pub receipts_root: Root,
    /// How many of the 512 committee members signed the update that finalized it, so a
    /// consumer can require more than 2/3.
    pub signers: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Instance {
    pub cfg: ChainConfig,
    pub store: Store,
    pub latest: HeaderRecord,
}

/// A record account's state: the record, and the instance whose update verified it.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct RecordState {
    pub instance: AccountId,
    pub record: HeaderRecord,
}

/// The seed of `instance`'s record of the header at `slot`.
pub fn record_seed(instance: &AccountId, slot: u64) -> PdaSeed {
    let mut h = Sha256::new();
    h.update(b"eth_lc/record/v1");
    h.update(instance.value());
    h.update(slot.to_le_bytes());
    PdaSeed::new(h.finalize().into())
}

/// Where `instance`'s record of the header at `slot` lives: a public account derived from the
/// program, so no one holds a key to it and only the program writes it.
pub fn record_account(program: &AccountId, instance: &AccountId, slot: u64) -> AccountId {
    AccountId::for_public_pda(program, &record_seed(instance, slot))
}

/// An event's 8-byte selector: the start of the keccak of its signature, as Ethereum does.
pub fn selector(signature: &str) -> [u8; 8] {
    alloy_primitives::keccak256(signature.as_bytes())[..8].try_into().expect("8 bytes")
}

pub(crate) fn b256(r: &Root) -> B256 {
    B256::from(*r)
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub enum LcMsg {
    /// Public, to an empty instance, signed by its account: bind `cfg` and start the store
    /// from `checkpoint`, a beacon block root the deployer trusts. `bootstrap` is Helios's
    /// `Bootstrap`, CBOR-encoded, and `committee` its committee's keys as SSZ; `rlp` is the
    /// execution header, needed from Gloas on.
    Init {
        cfg: ChainConfig,
        checkpoint: Root,
        bootstrap: Vec<u8>,
        committee: Vec<u8>,
        rlp: Option<Vec<u8>>,
    },
    /// Private, to the submitter's own actor of this program: verify `update` (Helios's
    /// `verify::AnyUpdate`, CBOR-encoded) on top of `store` under `cfg`, with the committee keys
    /// `current` and `next`, then advance `instance`. The instance checks that `store` and
    /// `cfg` are the ones it holds, so the submitter can't pick either.
    Update {
        instance: Actor,
        cfg: ChainConfig,
        store: Store,
        update: Vec<u8>,
        current: Vec<u8>,
        next: Option<Vec<u8>>,
        rlp: Option<Vec<u8>>,
    },
    /// Public, to the instance, only from this program: adopt `next` if `prev` is the store
    /// held and `cfg_hash` the config bound, and write `record`, if the update finalized a new
    /// header.
    Advance {
        cfg_hash: Root,
        prev: Store,
        next: Store,
        record: Option<HeaderRecord>,
    },
    /// Public, to a record account, only from this program: write `record` once.
    WriteRecord { instance: AccountId, record: HeaderRecord },
    /// Public, to `instance`'s record account of the header to prove against: prove `query`,
    /// against that header or, through `ancestry`, an earlier block. Emits `Attested`, and with
    /// `reply` also sends it back to the calling actor.
    Attest {
        instance: AccountId,
        query: Query,
        ancestry: Option<Ancestry>,
        reply: bool,
    },
    /// Private, to the caller's own actor of this program: prove `query` against `record`, or
    /// an earlier block through `ancestry`, then have `instance`'s record account of that slot
    /// confirm it holds `record`. Only that check is published; the query, the proof and the
    /// statement stay in the caller's proof.
    AttestPrivate {
        instance: Actor,
        record: HeaderRecord,
        query: Query,
        ancestry: Option<Ancestry>,
    },
    /// Public, to a record account, only from this program: refuse unless it is `instance`'s
    /// record and holds `record`.
    CheckRecord { instance: AccountId, record: HeaderRecord },
    /// From a record account to the actor that sent `Attest { reply: true, .. }`.
    Attested { statement: Statement },
}
