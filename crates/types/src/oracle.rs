//! Helios's `consensus-core`, off-chain: the client decodes Beacon API JSON into Helios's types
//! and converts them into the core's ([`header`], [`bootstrap`], [`update`]), and the tests run
//! Helios's own checks on the same input as a differential oracle ([`helios_update`]). Nothing
//! here runs in the program.

use crate::core::{self, Execution};
use crate::{InstanceConfig, Root};
use helios_consensus_core::consensus_spec::MainnetConsensusSpec as Spec;
use helios_consensus_core::types::{
    BeaconBlockHeader, Bootstrap, FinalityUpdate, Fork, Forks, LightClientHeader, LightClientStore, SyncCommittee, Update,
};
use helios_consensus_core::{apply_finality_update, apply_update, verify_finality_update, verify_update};
use ssz::Encode;

pub type Committee = SyncCommittee<Spec>;

/// A period's best update, which can hand off the next committee, or a finality update.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum AnyUpdate {
    Update(Update<Spec>),
    Finality(FinalityUpdate<Spec>),
}

/// A committee's keys as the program takes them: SSZ.
pub fn committee(c: &Committee) -> Vec<u8> {
    c.as_ssz_bytes()
}

fn beacon(b: &BeaconBlockHeader) -> core::BeaconBlockHeader {
    core::BeaconBlockHeader { slot: b.slot, proposer_index: b.proposer_index, parent_root: b.parent_root.0, state_root: b.state_root.0, body_root: b.body_root.0 }
}

fn roots(branch: &[alloy_primitives::B256]) -> Vec<Root> {
    branch.iter().map(|b| b.0).collect()
}

pub fn header(h: &LightClientHeader) -> core::LightClientHeader {
    match h {
        LightClientHeader::Gloas(g) => core::LightClientHeader {
            beacon: beacon(&g.beacon),
            execution: Execution::BlockHash(g.execution_block_hash.0),
            execution_branch: roots(&g.execution_branch),
        },
        _ => {
            let e = h.execution().expect("a header from Deneb on carries its payload header");
            let payload = core::ExecutionPayloadHeader {
                parent_hash: e.parent_hash().0,
                fee_recipient: e.fee_recipient().0.0,
                state_root: e.state_root().0,
                receipts_root: e.receipts_root().0,
                logs_bloom: e.logs_bloom().as_ssz_bytes(),
                prev_randao: e.prev_randao().0,
                block_number: *e.block_number(),
                gas_limit: *e.gas_limit(),
                gas_used: *e.gas_used(),
                timestamp: *e.timestamp(),
                extra_data: e.extra_data().as_ssz_bytes(),
                base_fee_per_gas: e.base_fee_per_gas().to_le_bytes(),
                block_hash: e.block_hash().0,
                transactions_root: e.transactions_root().0,
                withdrawals_root: e.withdrawals_root().map_or([0; 32], |r| r.0),
                blob_gas_used: e.blob_gas_used().copied().unwrap_or(0),
                excess_blob_gas: e.excess_blob_gas().copied().unwrap_or(0),
            };
            core::LightClientHeader {
                beacon: beacon(h.beacon()),
                execution: Execution::Payload(payload),
                execution_branch: roots(h.execution_branch().expect("a payload branch")),
            }
        }
    }
}

pub fn bootstrap(b: &Bootstrap<Spec>) -> core::LightClientBootstrap {
    core::LightClientBootstrap { header: header(&b.header()), current_sync_committee_branch: roots(b.current_sync_committee_branch()) }
}

pub fn update(u: &AnyUpdate) -> core::LightClientUpdate {
    match u {
        AnyUpdate::Update(u) => {
            let next = u.next_sync_committee();
            let next = (*next != Committee::default()).then(|| (committee(next), roots(u.next_sync_committee_branch())));
            let agg = u.sync_aggregate();
            core::LightClientUpdate {
                attested_header: header(&u.attested_header()),
                finalized_header: header(&u.finalized_header()),
                finality_branch: roots(u.finality_branch()),
                next_sync_committee: next,
                sync_committee_bits: agg.sync_committee_bits.as_ssz_bytes().try_into().expect("512 bits"),
                sync_committee_signature: agg.sync_committee_signature.as_ssz_bytes().try_into().expect("96 bytes"),
                signature_slot: *u.signature_slot(),
            }
        }
        AnyUpdate::Finality(u) => {
            let agg = u.sync_aggregate();
            core::LightClientUpdate {
                attested_header: header(&u.attested_header()),
                finalized_header: header(&u.finalized_header()),
                finality_branch: roots(u.finality_branch()),
                next_sync_committee: None,
                sync_committee_bits: agg.sync_committee_bits.as_ssz_bytes().try_into().expect("512 bits"),
                sync_committee_signature: agg.sync_committee_signature.as_ssz_bytes().try_into().expect("96 bytes"),
                signature_slot: *u.signature_slot(),
            }
        }
    }
}

/// Helios's `Forks` for `cfg`, whose forks are in that order.
pub fn forks(cfg: &InstanceConfig) -> Forks {
    let fork = |i: usize| match cfg.finality.forks.get(i) {
        Some((epoch, v)) => Fork { epoch: *epoch, fork_version: (*v).into() },
        None => Fork { epoch: u64::MAX, ..Default::default() },
    };
    let mut f = Forks {
        genesis: fork(0),
        altair: fork(1),
        bellatrix: fork(2),
        capella: fork(3),
        deneb: fork(4),
        electra: fork(5),
        fulu: fork(6),
        gloas: fork(7),
    };
    if cfg.gloas_epoch() == u64::MAX {
        f.gloas = Fork { epoch: u64::MAX, ..Default::default() };
    }
    f
}

/// Helios's store for `store`, given the committees whose roots it holds and its full header.
pub fn helios_store(finalized: &LightClientHeader, current: &Committee, next: Option<&Committee>) -> LightClientStore<Spec> {
    LightClientStore {
        finalized_header: finalized.clone(),
        optimistic_header: finalized.clone(),
        current_sync_committee: current.clone(),
        next_sync_committee: next.cloned(),
        ..Default::default()
    }
}

/// Helios's verdict on `u` on top of `store`: the store after it, or the error. Helios applies an
/// update below 2/3 participation as a no-op; that is reported here as its own error, so the
/// oracle and the core can be compared.
pub fn helios_update(cfg: &InstanceConfig, store: &LightClientStore<Spec>, u: &AnyUpdate) -> Result<LightClientStore<Spec>, String> {
    let forks = forks(cfg);
    let genesis = cfg.chain.genesis_validators_root.into();
    match u {
        AnyUpdate::Update(u) => verify_update(u, *u.signature_slot(), store, genesis, &forks),
        AnyUpdate::Finality(u) => verify_finality_update(u, *u.signature_slot(), store, genesis, &forks),
    }
    .map_err(|e| format!("{e}"))?;
    let mut after = store.clone();
    match u {
        AnyUpdate::Update(u) => apply_update(&mut after, u),
        AnyUpdate::Finality(u) => apply_finality_update(&mut after, u),
    };
    let unchanged = after.finalized_header.beacon().slot == store.finalized_header.beacon().slot
        && after.next_sync_committee.is_some() == store.next_sync_committee.is_some();
    if unchanged {
        return Err("not applied (below 2/3 participation, or nothing new)".into());
    }
    Ok(after)
}
