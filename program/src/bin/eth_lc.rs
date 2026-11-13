//! `eth_lc`: an Ethereum light client as a LEZ program, following finalised Ethereum through the
//! sync committee, with the consensus specs' checks and BLS through `blst`.
//!
//! The trust boundary is `from_own_program`. `Advance`, `WriteRecord` and `CheckRecord` act only
//! on messages from another actor of this same program, and the only code that sends them runs
//! after the check they depend on. So the instance never re-runs the BLS verification: it trusts
//! the program, and the program's code is fixed, because it is deployed immutable.
//!
//! Each verified finalised header is written once to its own account, at the address
//! `header_account` derives from the instance and the execution block number, and never
//! changed. A header's account refuses to act unless it sits at that address, so a header can't
//! be planted anywhere else.
//!
//! Every refusal panics with its typed error (`eth_lc_types::error::Refusal`), which the wallet's
//! local run reports before anything is proven or sent.

use eth_lc_types::history::{Ancestry, target_header};
use eth_lc_types::statement::{self, Claim, Statement};
use eth_lc_types::{Event, FinalisedHeader, Instance, LcMsg, core, finalised_header, header_account};
use lee_core::account::{AccountId, Actor};
use lee_core::program::{ReceiveInput, Response, write_once};

fn instance(input: &ReceiveInput) -> Instance {
    borsh::from_slice(input.pre_state.as_ref()).expect("an initialised instance")
}

/// The account of `header`, as an actor of this program.
fn header_actor(input: &ReceiveInput, header: &FinalisedHeader) -> Actor {
    let program = input.receiver.program_account_id;
    Actor::new(header_account(&program, &header.instance, header.block_number), program)
}

/// The header this account holds, after checking it is the account that header belongs at.
fn held_header(input: &ReceiveInput) -> FinalisedHeader {
    let held: FinalisedHeader = borsh::from_slice(input.pre_state.as_ref()).expect("header not held by this instance");
    assert_eq!(input.receiver, header_actor(input, &held), "not this header's account");
    held
}

/// Has `header` written to its account, and announces it.
fn finalise(input: &ReceiveInput, response: Response, header: FinalisedHeader) -> Response {
    let to = header_actor(input, &header);
    let event = Event::HeaderFinalised { instance: header.instance.clone(), block_number: header.block_number, header: to.account_id.clone() };
    response.event(event.to_program_event()).send_sync(to, &LcMsg::WriteRecord { header })
}

/// The statement for `claim`, checked against `instance`'s `header` or an earlier block it
/// reaches.
fn attest(instance: &AccountId, header: &FinalisedHeader, claim: &Claim, ancestry: Option<Ancestry>) -> Statement {
    let target = match ancestry {
        None => header.clone(),
        Some(a) => target_header(header, &a).unwrap_or_else(|e| panic!("the ancestry verifies: {e:?}")),
    };
    statement::check(instance, &target, claim).unwrap_or_else(|e| panic!("the claim holds: {e:?}"))
}

fn handle(input: &ReceiveInput) -> Response {
    match borsh::from_slice::<LcMsg>(&input.message).expect("an eth_lc message") {
        LcMsg::Init { config, bootstrap, committee, rlp } => {
            assert!(input.pre_state.as_ref().is_empty(), "instance already initialised");
            // Whoever holds the account chooses the checkpoint and the config. Without the
            // signature anyone could squat an account with a checkpoint of their choosing.
            assert!(input.is_authorized, "Init must be signed by the instance account");
            let store = core::bootstrap(&config, &bootstrap, &committee)
                .unwrap_or_else(|e| panic!("the bootstrap verifies against the checkpoint: {e:?}"));
            let instance_id = input.receiver.account_id.clone();
            let period = core::sync_committee_period(bootstrap.header.beacon.slot);
            let latest = finalised_header(&instance_id, &config, &bootstrap.header, rlp.as_deref(), 0, period)
                .unwrap_or_else(|e| panic!("the checkpoint's header: {e:?}"));
            let handoff = Event::CommitteeHandoff { instance: instance_id, period, committee_root: store.current_sync_committee };
            let state = Response::set_state(borsh::to_vec(&Instance { config, store, latest: latest.clone() }).unwrap()).event(handoff.to_program_event());
            finalise(input, state, latest)
        }

        LcMsg::Update { instance, config, store, update, current, next, rlp } => {
            let (next_store, advanced) =
                core::update(&config, &store, &update, &current, next.as_deref()).unwrap_or_else(|e| panic!("the update verifies: {e:?}"));
            let header = advanced.then(|| {
                let signers = core::signers(&update.sync_committee_bits) as u16;
                let period = core::sync_committee_period(update.signature_slot);
                finalised_header(&instance.account_id, &config, &update.finalized_header, rlp.as_deref(), signers, period)
                    .unwrap_or_else(|e| panic!("the finalised header: {e:?}"))
            });
            // A store goes stale `weak_subjectivity_period` after its finalized slot. Bounding
            // the transaction there lets the sequencer's clock refuse an update to a stale store,
            // on top of the check against the update's own signature slot.
            let stale_from = config.stale_from_ms(store.finalized_header.slot);
            Response::keep_state()
                .send_sync(instance, &LcMsg::Advance { config_hash: config.hash(), prev: store, next: next_store, header })
                .try_timestamp_window((None, Some(stale_from)))
                .expect("a valid window")
        }

        LcMsg::Advance { config_hash, prev, next, header } => {
            assert!(input.from_own_program(), "Advance must come from this program");
            let mut held = instance(input);
            assert_eq!(config_hash, held.config.hash(), "update verified under another config");
            assert_eq!(prev, held.store, "update verified on top of another store");
            let handoff = next.next_sync_committee.filter(|_| next.next_sync_committee != prev.next_sync_committee).map(|root| Event::CommitteeHandoff {
                instance: input.receiver.account_id.clone(),
                period: core::sync_committee_period(next.finalized_header.slot) + 1,
                committee_root: root,
            });
            held.store = next;
            if let Some(h) = &header {
                held.latest = h.clone();
            }
            let mut response = Response::set_state(borsh::to_vec(&held).unwrap());
            if let Some(e) = handoff {
                response = response.event(e.to_program_event());
            }
            match header {
                Some(h) => finalise(input, response, h),
                None => response,
            }
        }

        LcMsg::WriteRecord { header } => {
            assert!(input.from_own_program(), "WriteRecord must come from this program");
            assert_eq!(input.receiver, header_actor(input, &header), "not this header's account");
            Response::set_state(write_once(input.pre_state.as_ref(), borsh::to_vec(&header).unwrap()))
        }

        LcMsg::Attest { instance, claim, ancestry, reply } => {
            let held = held_header(input);
            assert_eq!(held.instance, instance, "a header of another instance");
            let statement = attest(&instance, &held, &claim, ancestry);
            let response = Response::keep_state().event(Event::Attested { statement: statement.clone() }.to_program_event());
            match (reply, input.from) {
                (true, Some(caller)) => response.send_sync(caller, &LcMsg::Attested { statement }),
                (true, None) => panic!("reply asked for, but no actor sent Attest"),
                (false, _) => response,
            }
        }

        LcMsg::AttestPrivate { instance, header, claim, ancestry } => {
            assert_eq!(header.instance, instance.account_id, "a header of another instance");
            attest(&instance.account_id, &header, &claim, ancestry);
            let to = header_actor(input, &header);
            Response::keep_state().send_sync(to, &LcMsg::CheckRecord { instance: instance.account_id, header })
        }

        LcMsg::CheckRecord { instance, header } => {
            assert!(input.from_own_program(), "CheckRecord must come from this program");
            let held = held_header(input);
            assert!(held.instance == instance && held == header, "header not held by this instance");
            Response::keep_state()
        }

        LcMsg::Attested { .. } => panic!("Attested is for consumers, not for eth_lc"),
    }
}

lee_core::actor_entrypoint!(raw handle);
