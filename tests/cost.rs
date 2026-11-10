//! What each public message costs, run through the deployed program's ELF in RISC Zero's
//! executor with the per-transaction cap, as the sequencer meters it. Needs the guest built
//! (`lc/guest/target/riscv32im-risc0-zkvm-elf/docker/eth_lc.bin`).
//!
//!   cargo test --release --test cost -- --nocapture

mod support;

use eth_lc_types::LcMsg;
use eth_lc_types::oracle;
use lee::{AccountId, Actor, ActorState, program::Program};
use lee_core::program::ReceiveInput;
use risc0_zkvm::{ExecutorEnv, default_executor};
use support::mainnet;

const CAP: u64 = 10_000_000;
const PROBE: u64 = 200_000_000;

fn elf() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/lc/guest/target/riscv32im-risc0-zkvm-elf/docker/eth_lc.bin")).expect("build the guest first")
}

fn run(message: &LcMsg) -> Result<u64, String> {
    let input = ReceiveInput {
        receiver: Actor::new(AccountId::new([7; 32]), AccountId::new([9; 32])),
        from: None,
        is_authorized: true,
        pre_state: ActorState::empty(),
        message: Program::serialize_message(message).unwrap(),
    };
    let mut builder = ExecutorEnv::builder();
    builder.session_limit(Some(PROBE));
    Program::write_receive_input(&input, &mut builder).unwrap();
    let env = builder.build().unwrap();
    default_executor().execute(env, &elf()).map(|info| info.cycles()).map_err(|e| format!("{e:#}"))
}

#[test]
fn init_fits_one_transaction() {
    let chain = mainnet();
    let committee = oracle::committee(chain.bootstrap.current_sync_committee());
    let init = LcMsg::Init { cfg: chain.cfg.clone(), checkpoint: chain.checkpoint, bootstrap: oracle::bootstrap(&chain.bootstrap), committee, rlp: None };
    match run(&init) {
        Ok(cycles) => println!("Init: {cycles} cycles of a {CAP} cap"),
        Err(e) => panic!("Init: {e}"),
    }
}

#[test]
fn update_cycles() {
    let chain = mainnet();
    let (mut steps, _) = support::steps(&chain);
    let s = steps.pop().unwrap();
    let update = LcMsg::Update {
        instance: Actor::new(AccountId::new([7; 32]), AccountId::new([9; 32])),
        cfg: chain.cfg.clone(),
        store: s.store.clone(),
        update: oracle::update(&s.update),
        current: s.current.clone(),
        next: s.next.clone(),
        rlp: None,
    };
    match run(&update) {
        Ok(cycles) => println!("Update (finality): {cycles} cycles"),
        Err(e) => panic!("Update: {e}"),
    }
}

#[test]
fn attest_cycles() {
    use eth_lc_types::statement::{Attestation, Op, Predicate, Proof};
    use eth_lc_types::{RecordState, record_account};
    let chain = mainnet();
    let (mut steps, _) = support::steps(&chain);
    let s = steps.pop().unwrap();
    let u = oracle::update(&s.update);
    let record = eth_lc_types::record_of(&chain.cfg, &u.finalized_header, None, 512).unwrap();
    let (instance, program) = (AccountId::new([7; 32]), AccountId::new([9; 32]));
    let proof = lez_hello_world::chain::storage_proof(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mainnet-1904/getproof-26376237.json")).unwrap();
    let attestation = Attestation { chain_id: 1, proofs: vec![Proof::Storage(proof)], predicates: vec![Predicate::CheckSlotValue { proof: 0, op: Op::Greater, value: [0; 32] }] };
    let input = ReceiveInput {
        receiver: Actor::new(record_account(&program, &instance, record.slot), program),
        from: None,
        is_authorized: false,
        pre_state: ActorState::from(borsh::to_vec(&RecordState { instance, record }).unwrap()),
        message: Program::serialize_message(&LcMsg::Attest { instance, attestation, ancestry: None, reply: false }).unwrap(),
    };
    let mut builder = ExecutorEnv::builder();
    builder.session_limit(Some(PROBE));
    Program::write_receive_input(&input, &mut builder).unwrap();
    match default_executor().execute(builder.build().unwrap(), &elf()) {
        Ok(info) => println!("Attest (storage slot): {} cycles", info.cycles()),
        Err(e) => panic!("Attest: {e:#}"),
    }
}

#[test]
fn handoff_cycles() {
    let chain = mainnet();
    let (steps, _) = support::steps(&chain);
    // The first step learns the next committee: a handoff update.
    let s = &steps[1];
    let update = LcMsg::Update {
        instance: Actor::new(AccountId::new([7; 32]), AccountId::new([9; 32])),
        cfg: chain.cfg.clone(),
        store: s.store.clone(),
        update: oracle::update(&s.update),
        current: s.current.clone(),
        next: s.next.clone(),
        rlp: None,
    };
    let u = oracle::update(&s.update);
    match run(&update) {
        Ok(cycles) => println!("Update (handoff, next committee {}): {cycles} cycles", u.next_sync_committee.is_some()),
        Err(e) => panic!("Update: {e}"),
    }
}
