//! Attesting to an older block (proposal section 8), natively, on mainnet data pinned in
//! `tests/data/history`: blocks 26352344–26352347 and 26352394, and the EIP-2935 history
//! contract's slot for 26352344 proven at 26352394. Fetched with `scripts/fetch-chain.sh`
//! (`header` and `history` modes).
//!
//!   cargo test --release --test history

use eth_lc_types::HeaderRecord;
use eth_lc_types::error::Error;
use eth_lc_types::history::{Ancestry, HISTORY_ADDRESS, history_slot, target_record};
use eth_lc_types::mpt::{Account, StorageProof};
use lez_hello_world::chain::hex_decode;
use serde_json::Value;

const TARGET: u64 = 26_352_344;
const ANCHOR: u64 = 26_352_394;

fn data(file: &str) -> String {
    format!("{}/tests/data/history/{file}", env!("CARGO_MANIFEST_DIR"))
}

fn json(file: &str) -> Value {
    serde_json::from_slice(&std::fs::read(data(file)).unwrap()).unwrap()
}

fn h32(v: &Value) -> [u8; 32] {
    let b = hex_decode(v.as_str().unwrap()).unwrap();
    let mut w = [0u8; 32];
    w[32 - b.len()..].copy_from_slice(&b);
    w
}

/// A header's RLP, checked to hash to the block hash the node reported: the encoder's check.
fn rlp(n: u64) -> Vec<u8> {
    let rlp = hex_decode(&std::fs::read_to_string(data(&format!("header-{n}.rlp"))).unwrap()).unwrap();
    assert_eq!(alloy_primitives::keccak256(&rlp).0, h32(&json(&format!("header-{n}.json"))["result"]["hash"]), "header {n}'s RLP doesn't hash to its block hash");
    rlp
}

/// A record of block `n` as the instance would hold it.
fn record(n: u64) -> HeaderRecord {
    let b = &json(&format!("header-{n}.json"))["result"];
    HeaderRecord {
        chain_id: 1,
        slot: 0,
        beacon_root: [0; 32],
        block_number: n,
        block_hash: h32(&b["hash"]),
        state_root: h32(&b["stateRoot"]),
        receipts_root: h32(&b["receiptsRoot"]),
        signers: 512,
    }
}

fn history_proof() -> StorageProof {
    let r = &json(&format!("history-{ANCHOR}-{TARGET}.json"))["result"];
    let nodes = |v: &Value| v.as_array().unwrap().iter().map(|x| hex_decode(x.as_str().unwrap()).unwrap()).collect();
    StorageProof {
        address: hex_decode(r["address"].as_str().unwrap()).unwrap().try_into().unwrap(),
        account: Account {
            nonce: u64::from_str_radix(r["nonce"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap(),
            balance: h32(&r["balance"]),
            storage_root: h32(&r["storageHash"]),
            code_hash: h32(&r["codeHash"]),
        },
        account_proof: nodes(&r["accountProof"]),
        slot: history_slot(TARGET),
        value: h32(&r["storageProof"][0]["value"]),
        storage_proof: nodes(&r["storageProof"][0]["proof"]),
    }
}

fn eip2935() -> Ancestry {
    Ancestry::Eip2935 { proof: history_proof(), header: rlp(TARGET) }
}

fn chain(from: u64) -> Ancestry {
    Ancestry::ParentChain { headers: (TARGET..=from).rev().map(rlp).collect() }
}

/// What a derived record of `TARGET` must equal: its execution fields, the anchor's signers.
fn expected(anchor: &HeaderRecord) -> HeaderRecord {
    HeaderRecord { signers: anchor.signers, ..record(TARGET) }
}

#[test]
fn every_pinned_header_encodes_to_its_hash() {
    for n in [TARGET, TARGET + 1, TARGET + 2, TARGET + 3, ANCHOR] {
        rlp(n);
    }
}

#[test]
fn eip2935_reaches_a_block_before_the_record() {
    let anchor = record(ANCHOR);
    assert_eq!(target_record(&anchor, &eip2935()), Ok(expected(&anchor)));
}

#[test]
fn eip2935_refuses_a_forged_header_or_slot_or_contract() {
    let anchor = record(ANCHOR);
    let mut header = rlp(TARGET);
    let last = header.len() - 1;
    header[last] ^= 1;
    assert_eq!(target_record(&anchor, &Ancestry::Eip2935 { proof: history_proof(), header }), Err(Error::BadExecutionBranch));
    // The real header, with the slot of another block.
    let mut other = history_proof();
    other.slot[31] ^= 1;
    assert!(target_record(&anchor, &Ancestry::Eip2935 { proof: other, header: rlp(TARGET) }).is_err());
    // The same proof, claimed for another contract.
    let mut contract = history_proof();
    contract.address[19] ^= 1;
    assert_eq!(target_record(&anchor, &Ancestry::Eip2935 { proof: contract, header: rlp(TARGET) }), Err(Error::BadInclusion));
    assert_eq!(history_proof().address, HISTORY_ADDRESS);
}

#[test]
fn eip2935_refuses_a_target_outside_the_window() {
    let mut anchor = record(ANCHOR);
    anchor.block_number = TARGET + 8_192;
    assert!(target_record(&anchor, &eip2935()).is_err());
}

#[test]
fn parent_chain_reaches_a_block_before_the_record() {
    let anchor = record(TARGET + 3);
    assert_eq!(target_record(&anchor, &chain(TARGET + 3)), Ok(expected(&anchor)));
}

#[test]
fn parent_chain_refuses_a_broken_link_or_another_anchor() {
    let anchor = record(TARGET + 3);
    let Ancestry::ParentChain { mut headers } = chain(TARGET + 3) else { unreachable!() };
    headers.swap(1, 2);
    assert_eq!(target_record(&anchor, &Ancestry::ParentChain { headers }), Err(Error::BadExecutionBranch));
    assert_eq!(target_record(&record(ANCHOR), &chain(TARGET + 3)), Err(Error::BadExecutionBranch));
}
