//! `eth_lc`: an Ethereum light client as a LEZ program, on Helios's consensus checks.
//!
//! The trust boundary is `from_own_program`. `Advance`, `WriteRecord` and `CheckRecord` act only
//! on messages from another actor of this same program, and the only code that sends them runs
//! after the check they depend on. So the instance never re-runs the BLS verification: it trusts
//! the program, and the program's code is fixed, because it is deployed immutable.
//!
//! Each verified header is written once to its own record account, at the address
//! `record_account` derives from the instance and the slot, and never changed. A record account
//! refuses to act unless it sits at that address, so a record can't be planted anywhere else.
//!
//! Every refusal panics with its typed error (`eth_lc_types::error::Error`), which the wallet's
//! local run reports before anything is proven or sent.

use eth_lc_types::history::{Ancestry, target_record};
use eth_lc_types::statement::{self, Statement};
use eth_lc_types::{HeaderRecord, Instance, LcMsg, RecordState, record_account, selector, verify};
use lee_core::account::{AccountId, Actor};
use lee_core::program::{ProgramEvent, ReceiveInput, Response, write_once};

fn instance(input: &ReceiveInput) -> Instance {
    borsh::from_slice(input.pre_state.as_ref()).expect("an initialised instance")
}

/// The record account of `instance`'s header at `slot`, as an actor of this program.
fn record_actor(input: &ReceiveInput, instance: &AccountId, slot: u64) -> Actor {
    let program = input.receiver.program_account_id;
    Actor::new(record_account(&program, instance, slot), program)
}

/// Writes `record` to its account, from the instance that verified it.
fn write_record(input: &ReceiveInput, response: Response, record: HeaderRecord) -> Response {
    let instance = input.receiver.account_id;
    let to = record_actor(input, &instance, record.slot);
    response.send_sync(to, &LcMsg::WriteRecord { instance, record })
}

/// The record this account holds, after checking it is the account its record belongs at.
fn held_record(input: &ReceiveInput) -> RecordState {
    let held: RecordState = borsh::from_slice(input.pre_state.as_ref()).expect("record not held by this instance");
    assert_eq!(input.receiver, record_actor(input, &held.instance, held.record.slot), "not this record's account");
    held
}

/// The statement for `query`, proven against `record` or an earlier block it reaches.
fn attest(record: &HeaderRecord, query: &statement::Query, ancestry: Option<Ancestry>) -> Statement {
    let target = match ancestry {
        None => record.clone(),
        Some(a) => target_record(record, &a).unwrap_or_else(|e| panic!("the ancestry verifies: {e:?}")),
    };
    statement::evaluate(&target, query).unwrap_or_else(|e| panic!("the statement holds: {e:?}"))
}

fn handle(input: &ReceiveInput) -> Response {
    match borsh::from_slice::<LcMsg>(&input.message).expect("an eth_lc message") {
        LcMsg::Init { cfg, checkpoint, bootstrap, rlp } => {
            assert!(input.pre_state.as_ref().is_empty(), "instance already initialised");
            // Whoever holds the account chooses the checkpoint and the config. Without the
            // signature anyone could squat an account with a checkpoint of their choosing.
            assert!(input.is_authorized, "Init must be signed by the instance account");
            let (store, latest) = verify::bootstrap(&cfg, &checkpoint, &bootstrap, rlp.as_deref())
                .unwrap_or_else(|e| panic!("the bootstrap verifies against the checkpoint: {e:?}"));
            let state = Response::set_state(borsh::to_vec(&Instance { cfg, store, latest: latest.clone() }).unwrap());
            write_record(input, state, latest)
        }

        LcMsg::Update { instance, cfg, store, update, current, next, rlp } => {
            let (next_store, record) = verify::update(&cfg, &store, &update, &current, next.as_deref(), rlp.as_deref())
                .unwrap_or_else(|e| panic!("the update verifies: {e:?}"));
            // A store goes stale `max_store_age` after its finalized slot. Bounding the
            // transaction there lets the sequencer's clock refuse an update to a stale store, on
            // top of the check against the update's own signature slot.
            let stale_from = cfg.stale_from_ms(store.finalized_slot);
            Response::keep_state()
                .send_sync(instance, &LcMsg::Advance { cfg_hash: cfg.hash(), prev: store, next: next_store, record })
                .try_timestamp_window((None, Some(stale_from)))
                .expect("a valid window")
        }

        LcMsg::Advance { cfg_hash, prev, next, record } => {
            assert!(input.from_own_program(), "Advance must come from this program");
            let mut held = instance(input);
            assert_eq!(cfg_hash, held.cfg.hash(), "update verified under another config");
            assert_eq!(prev, held.store, "update verified on top of another store");
            held.store = next;
            if let Some(r) = &record {
                held.latest = r.clone();
            }
            let state = Response::set_state(borsh::to_vec(&held).unwrap());
            match record {
                Some(r) => write_record(input, state, r),
                None => state,
            }
        }

        LcMsg::WriteRecord { instance, record } => {
            assert!(input.from_own_program(), "WriteRecord must come from this program");
            assert_eq!(input.receiver, record_actor(input, &instance, record.slot), "not this record's account");
            let data = borsh::to_vec(&RecordState { instance, record }).unwrap();
            Response::set_state(write_once(input.pre_state.as_ref(), data))
        }

        LcMsg::Attest { instance, query, ancestry, reply } => {
            let held = held_record(input);
            assert_eq!(held.instance, instance, "a record of another instance");
            let statement = attest(&held.record, &query, ancestry);
            let event = ProgramEvent { selector: selector("Attested(bytes32,uint64,uint64)"), data: borsh::to_vec(&statement).unwrap() };
            let response = Response::keep_state().event(event);
            match (reply, input.from) {
                (true, Some(caller)) => response.send_sync(caller, &LcMsg::Attested { statement }),
                (true, None) => panic!("reply asked for, but no actor sent Attest"),
                (false, _) => response,
            }
        }

        LcMsg::AttestPrivate { instance, record, query, ancestry } => {
            attest(&record, &query, ancestry);
            let to = record_actor(input, &instance.account_id, record.slot);
            Response::keep_state().send_sync(to, &LcMsg::CheckRecord { instance: instance.account_id, record })
        }

        LcMsg::CheckRecord { instance, record } => {
            assert!(input.from_own_program(), "CheckRecord must come from this program");
            let held = held_record(input);
            assert!(held.instance == instance && held.record == record, "record not held by this instance");
            Response::keep_state()
        }

        LcMsg::Attested { .. } => panic!("Attested is for consumers, not for eth_lc"),
    }
}

lee_core::actor_entrypoint!(raw handle);
