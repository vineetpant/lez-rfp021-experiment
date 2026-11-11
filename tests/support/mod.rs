//! Shared by the native tests: the recorded mainnet segment, walked forward the way the program
//! advances an instance, with Helios walking it alongside as the oracle.
#![allow(dead_code)]

use eth_lc_types::core;
use eth_lc_types::error::Refusal;
use eth_lc_types::oracle::{self, AnyUpdate};
use helios_consensus_core::consensus_spec::MainnetConsensusSpec as Spec;
use helios_consensus_core::types::LightClientStore;
use lez_hello_world::chain::{Chain, load_chain};

/// Periods 1904–1906 of mainnet: a bootstrap at slot 15597568, the best update of each period
/// (two committee handoffs), and a finality update, recorded from a beacon node.
pub fn mainnet() -> Chain {
    load_chain(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mainnet-1904"), 1).expect("the recorded segment loads")
}

/// One step: the store before it (the core's and Helios's), the update, and the committee keys
/// the core checks it with.
pub struct Step {
    pub store: core::LightClientStore,
    pub helios: LightClientStore<Spec>,
    pub update: AnyUpdate,
    pub current: Vec<u8>,
    pub next: Option<Vec<u8>>,
}

impl Step {
    pub fn apply(&self, chain: &Chain) -> Result<(core::LightClientStore, bool), Refusal> {
        self.apply_other(chain, &self.update)
    }

    /// The core's verdict on `update` in place of this step's.
    pub fn apply_other(&self, chain: &Chain, update: &AnyUpdate) -> Result<(core::LightClientStore, bool), Refusal> {
        core::update(&chain.config, &self.store, &oracle::update(update), &self.current, self.next.as_deref())
    }

    /// Helios's verdict on `update` on this step's store.
    pub fn helios(&self, chain: &Chain, update: &AnyUpdate) -> Result<LightClientStore<Spec>, String> {
        oracle::helios_update(&chain.config, &self.helios, update)
    }
}

/// The store a bootstrap gives, in the core and in Helios.
pub fn boot(chain: &Chain) -> (core::LightClientStore, LightClientStore<Spec>) {
    let b = oracle::bootstrap(&chain.bootstrap);
    let committee = chain.bootstrap.current_sync_committee();
    let store = core::bootstrap(&chain.config, &b, &oracle::committee(committee)).expect("bootstrap");
    (store, oracle::helios_store(&chain.bootstrap.header(), committee, None))
}

/// The keys for `store`'s committees, from the segment.
pub fn keys(chain: &Chain, store: &core::LightClientStore) -> (Vec<u8>, Option<Vec<u8>>) {
    let current = oracle::committee(chain.committee(&store.current_sync_committee).expect("current committee in the segment"));
    let next = store.next_sync_committee.map(|r| oracle::committee(chain.committee(&r).expect("next committee in the segment")));
    (current, next)
}

/// Every update that advances the store, in order, from the bootstrap; and the final store. The
/// core's store drives it; Helios's follows.
pub fn steps(chain: &Chain) -> (Vec<Step>, core::LightClientStore) {
    let (mut store, mut helios) = boot(chain);
    let mut out = vec![];
    for update in &chain.updates {
        let (current, next) = keys(chain, &store);
        let step = Step { store: store.clone(), helios: helios.clone(), update: update.clone(), current, next };
        if let Ok((after, _)) = step.apply(chain) {
            helios = step.helios(chain, update).expect("Helios applies what the core applies");
            store = after;
            out.push(step);
        }
    }
    (out, store)
}

// ---- receipts of block 26376237, as an execution node serves them ----

use alloy_rlp::{BufMut, Encodable, Header};
use alloy_trie::{HashBuilder, Nibbles, proof::ProofRetainer};
use eth_lc_types::statement::Proof;
use lez_hello_world::chain::hex_decode;
use serde_json::Value;

fn hex(v: &Value) -> Vec<u8> {
    hex_decode(v.as_str().unwrap()).unwrap()
}

fn uint(v: &Value) -> u64 {
    u64::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}

fn list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload: Vec<u8> = items.concat();
    let mut out = Vec::new();
    Header { list: true, payload_length: payload.len() }.encode(&mut out);
    out.put_slice(&payload);
    out
}

pub fn rlp<T: Encodable + ?Sized>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.encode(&mut out);
    out
}

/// A receipt from `eth_getBlockReceipts`, encoded as it sits in the receipt trie (EIP-2718).
pub fn encode_receipt(r: &Value) -> Vec<u8> {
    let logs: Vec<Vec<u8>> = r["logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            let topics: Vec<Vec<u8>> = l["topics"].as_array().unwrap().iter().map(|t| rlp(&hex(t)[..])).collect();
            list(&[rlp(&hex(&l["address"])[..]), list(&topics), rlp(&hex(&l["data"])[..])])
        })
        .collect();
    let body = list(&[rlp(&uint(&r["status"])), rlp(&uint(&r["cumulativeGasUsed"])), rlp(&hex(&r["logsBloom"])[..]), list(&logs)]);
    match uint(&r["type"]) {
        0 => body,
        t => [vec![t as u8], body].concat(),
    }
}

/// A proof of transaction `tx`'s receipt, built from the block's receipts: the trie root it
/// rebuilds, and the proof.
pub fn receipt_proof(tx: u64) -> ([u8; 32], Proof) {
    let j: Value = serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mainnet-1904/receipts-26376237.json")).unwrap()).unwrap();
    let receipts = j["result"].as_array().unwrap();
    let mut leaves: Vec<(Nibbles, Vec<u8>)> = receipts.iter().enumerate().map(|(i, r)| (Nibbles::unpack(rlp(&(i as u64))), encode_receipt(r))).collect();
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    let target = Nibbles::unpack(rlp(&tx));
    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![target]));
    for (k, v) in &leaves {
        hb.add_leaf(*k, v);
    }
    let root = hb.root();
    let proof = hb.take_proof_nodes().matching_nodes_sorted(&target).into_iter().map(|(_, n)| n.to_vec()).collect();
    (root.0, Proof::Log { tx_index: tx, receipt: encode_receipt(&receipts[tx as usize]), proof, log_index: 0 })
}
