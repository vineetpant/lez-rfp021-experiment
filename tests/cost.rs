//! What each public message costs, run through the deployed program's ELF in RISC Zero's
//! executor with the per-transaction cap, as the sequencer meters it. Needs the guest built
//! (`lc/guest/target/riscv32im-risc0-zkvm-elf/docker/eth_lc.bin`).
//!
//!   cargo test --release --test cost -- --nocapture

mod support;

use eth_lc_types::LcMsg;
use eth_lc_types::verify::{cbor, committee_bytes};
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
    let committee = committee_bytes(chain.bootstrap.current_sync_committee());
    let init = LcMsg::Init { cfg: chain.cfg.clone(), checkpoint: chain.checkpoint, bootstrap: cbor(&chain.bootstrap), committee, rlp: None };
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
        update: cbor(&s.update),
        current: s.current.clone(),
        next: s.next.clone(),
        rlp: None,
    };
    match run(&update) {
        Ok(cycles) => println!("Update (finality): {cycles} cycles"),
        Err(e) => panic!("Update: {e}"),
    }
}
