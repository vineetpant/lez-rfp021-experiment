//! Messages, state and checks of the `eth_lc` program, an Ethereum light client on LEZ.
//!
//! One deployed program serves any number of instances. An instance is the program's actor
//! state on one public account: the chain config it was bound to at `Init`, a compact
//! light-client store, and its latest record. Each verified header is also written once to its
//! own **record** account, at an address derived from the instance and the header's slot
//! ([`record_account`]), never rewritten or removed.
//!
//! The light-client checks are [`core`]: the consensus specs' light-client protocol, with BLS
//! through `blst`, written for cost inside the zkVM. Helios's `consensus-core` checks the same
//! updates in the tests, as a differential oracle ([`oracle`], host only, behind the `helios`
//! feature). Merkle-Patricia proofs are checked with `alloy-trie` ([`mpt`]). Statements,
//! predicates and history proofs are in [`statement`] and [`history`].

pub mod core;
pub mod error;
pub mod history;
pub mod mpt;
#[cfg(feature = "helios")]
pub mod oracle;
pub mod statement;

use alloy_consensus::Header as ExecutionHeader;
use alloy_primitives::B256;
use alloy_rlp::Decodable;
use borsh::{BorshDeserialize, BorshSerialize};
use error::Refusal;
use history::Ancestry;
use lee_core::account::{AccountId, Actor};
use lee_core::program::{PdaSeed, ProgramEvent};
use sha2::{Digest, Sha256};
use statement::{Claim, Statement};

pub type Root = [u8; 32];

/// The chain an instance follows: its execution chain and its consensus network.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ChainBinding {
    pub chain_id: u64,
    pub genesis_validators_root: Root,
}

/// How the instance decides a header is final.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct FinalityParams {
    pub genesis_time: u64,
    /// `(activation epoch, fork version)` of every fork, oldest first.
    pub forks: Vec<(u64, [u8; 4])>,
    /// Signers an update needs, of 512; never fewer than the specs' supermajority of 342.
    pub min_participants: u16,
    /// Seconds after its finalized slot that a store may still be advanced. Past it the
    /// instance is replaced, not refreshed (F.10).
    pub weak_subjectivity_period: u64,
}

/// What an instance is bound to at `Init` and never changes (F.9, F.13).
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct InstanceConfig {
    /// The beacon block root the instance starts from.
    pub checkpoint: Root,
    pub chain: ChainBinding,
    pub finality: FinalityParams,
}

/// The fewest signers any instance accepts: more than 2/3 of 512.
pub const SUPERMAJORITY: u16 = 342;

/// Positions in the fork schedule, oldest first, where the state's field positions change.
const ELECTRA: usize = 5;
const GLOAS: usize = 7;

impl InstanceConfig {
    /// The fork version in force at `epoch`.
    pub fn fork_version(&self, epoch: u64) -> [u8; 4] {
        self.finality.forks.iter().rev().find(|(e, _)| *e <= epoch).map_or([0; 4], |(_, v)| *v)
    }

    fn fork_epoch(&self, position: usize) -> u64 {
        self.finality.forks.get(position).map_or(u64::MAX, |(e, _)| *e)
    }

    /// Electra deepened the state tree.
    pub fn electra_epoch(&self) -> u64 {
        self.fork_epoch(ELECTRA)
    }

    /// Gloas deepened it again, and moved the execution payload out of the block body.
    pub fn gloas_epoch(&self) -> u64 {
        self.fork_epoch(GLOAS)
    }

    pub fn hash(&self) -> Root {
        let mut h = Sha256::new();
        h.update(b"eth_lc/config/v1");
        h.update(borsh::to_vec(self).expect("config encodes"));
        h.finalize().into()
    }

    /// Unix time of `slot`, in seconds.
    pub fn slot_time(&self, slot: u64) -> u64 {
        self.finality.genesis_time + slot * 12
    }

    /// When a store finalized at `slot` stops being advanceable, in milliseconds.
    pub fn stale_from_ms(&self, slot: u64) -> u64 {
        (self.slot_time(slot) + self.finality.weak_subjectivity_period) * 1000
    }
}

pub use crate::core::LightClientStore;

/// A verified finalised header, as its account holds it and attestations are checked against
/// (D4). It names its instance and chain, so a statement is refused on another (F.5, F.15).
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct FinalisedHeader {
    pub instance: AccountId,
    pub chain: ChainBinding,
    pub slot: u64,
    pub beacon_root: Root,
    pub block_number: u64,
    /// Where history proofs start (proposal section 7.1).
    pub block_hash: Root,
    pub state_root: Root,
    pub receipts_root: Root,
    /// Of 512, so a consumer can require more than the instance does.
    pub signers: u16,
    /// The sync-committee period whose committee signed it.
    pub committee_period: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Instance {
    pub config: InstanceConfig,
    pub store: LightClientStore,
    pub latest: FinalisedHeader,
}

/// The seed of `instance`'s finalised header for execution block `block_number`.
pub fn header_seed(instance: &AccountId, block_number: u64) -> PdaSeed {
    let mut h = Sha256::new();
    h.update(b"header");
    h.update(instance.value());
    h.update(block_number.to_le_bytes());
    PdaSeed::new(h.finalize().into())
}

/// Where `instance`'s finalised header for `block_number` lives: a public account derived from
/// the program, so no one holds a key to it and only the program writes it.
pub fn header_account(program: &AccountId, instance: &AccountId, block_number: u64) -> AccountId {
    AccountId::for_public_pda(program, &header_seed(instance, block_number))
}

/// What the program emits, so clients and operators follow an instance without polling it.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Event {
    CommitteeHandoff { instance: AccountId, period: u64, committee_root: Root },
    HeaderFinalised { instance: AccountId, block_number: u64, header: AccountId },
    Attested { statement: statement::Statement },
}

impl Event {
    /// As a LEZ event: tagged with the first 8 bytes of `sha256("eth_lc::<name>")`, LEZ's
    /// convention, and the event itself as data.
    pub fn to_program_event(&self) -> ProgramEvent {
        let name = match self {
            Event::CommitteeHandoff { .. } => "eth_lc::CommitteeHandoff",
            Event::HeaderFinalised { .. } => "eth_lc::HeaderFinalised",
            Event::Attested { .. } => "eth_lc::Attested",
        };
        let tag: [u8; 32] = Sha256::digest(name.as_bytes()).into();
        ProgramEvent { selector: tag[..8].try_into().expect("8 bytes"), data: borsh::to_vec(self).expect("event encodes") }
    }
}

pub(crate) fn b256(r: &Root) -> B256 {
    B256::from(*r)
}

/// The execution block a verified header commits to: number, hash, state and receipts roots.
/// Up to Fulu they sit in the payload header the verifier proved under the body root. From Gloas
/// only the block hash does, and `rlp`, the execution header, must hash to it.
fn execution_block(header: &core::LightClientHeader, rlp: Option<&[u8]>) -> Result<(u64, Root, Root, Root), Refusal> {
    match &header.execution {
        core::Execution::Payload(p) => Ok((p.block_number, p.block_hash, p.state_root, p.receipts_root)),
        core::Execution::BlockHash(hash) => {
            let rlp = rlp.ok_or(Refusal::InvalidExecutionProof)?;
            if alloy_primitives::keccak256(rlp).0 != *hash {
                return Err(Refusal::InvalidExecutionProof);
            }
            let block = ExecutionHeader::decode(&mut &rlp[..]).map_err(|_| Refusal::InvalidExecutionProof)?;
            Ok((block.number, *hash, block.state_root.0, block.receipts_root.0))
        }
    }
}

/// The finalised header `instance` keeps for a verified `header`, signed by `signers` of the
/// committee of `committee_period`.
pub fn finalised_header(
    instance: &AccountId,
    config: &InstanceConfig,
    header: &core::LightClientHeader,
    rlp: Option<&[u8]>,
    signers: u16,
    committee_period: u64,
) -> Result<FinalisedHeader, Refusal> {
    let (block_number, block_hash, state_root, receipts_root) = execution_block(header, rlp)?;
    Ok(FinalisedHeader {
        instance: instance.clone(),
        chain: config.chain.clone(),
        slot: header.beacon.slot,
        beacon_root: core::beacon_root(&header.beacon),
        block_number,
        block_hash,
        state_root,
        receipts_root,
        signers,
        committee_period,
    })
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub enum LcMsg {
    /// Public, to an empty instance, signed by its account: bind `config` and start the store
    /// from its checkpoint, with the bootstrap and its committee's keys (SSZ); `rlp` is the
    /// execution header, needed from Gloas on.
    Init {
        config: InstanceConfig,
        bootstrap: core::LightClientBootstrap,
        committee: Vec<u8>,
        rlp: Option<Vec<u8>>,
    },
    /// Private, to the submitter's own actor of this program: verify `update` on top of `store`
    /// under `config`, with the keys (SSZ) of the committees `store` holds the roots of, then
    /// advance `instance`. The instance checks that `store` and `config` are the ones it holds,
    /// so the submitter can't pick either.
    Update {
        instance: Actor,
        config: InstanceConfig,
        store: LightClientStore,
        update: core::LightClientUpdate,
        current: Vec<u8>,
        next: Option<Vec<u8>>,
        rlp: Option<Vec<u8>>,
    },
    /// Public, to the instance, only from this program: adopt `next` if `prev` is the store
    /// held and `config_hash` the config bound, and write `header`, if the update finalised a new
    /// one.
    Advance {
        config_hash: Root,
        prev: LightClientStore,
        next: LightClientStore,
        header: Option<FinalisedHeader>,
    },
    /// Public, to a finalised header's account, only from this program: write `header` once.
    WriteRecord { header: FinalisedHeader },
    /// Public, to `instance`'s finalised header to prove against: check `claim` against it or,
    /// through `ancestry`, an earlier block. Emits `Attested`, and with `reply` also sends the
    /// statement back to the calling actor.
    Attest {
        instance: AccountId,
        claim: Claim,
        ancestry: Option<Ancestry>,
        reply: bool,
    },
    /// Private, to the caller's own actor of this program: check `claim` against `header`, or an
    /// earlier block through `ancestry`, then have that header's account confirm it holds
    /// `header`. Only that check is published; the claim and the statement stay in the caller's
    /// proof.
    AttestPrivate {
        instance: Actor,
        header: FinalisedHeader,
        claim: Claim,
        ancestry: Option<Ancestry>,
    },
    /// Public, to a finalised header's account, only from this program: refuse unless it is
    /// `instance`'s and holds `header`.
    CheckRecord { instance: AccountId, header: FinalisedHeader },
    /// From a finalised header's account to the actor that sent `Attest { reply: true, .. }`.
    Attested { statement: Statement },
}
