//! A LEZ program that appends its message bytes to its own actor state on the account it
//! receives.
//!
//! Adapted from `hello_world.rs` in logos-execution-zone's `program_deployment` example.
//!
//! Two things about this program are worth noticing, because they are what makes LEZ
//! different.
//!
//! It holds no state. Everything persistent lives in actor state: one slot per (account,
//! program) pair, which only this program can write. That is why one deployed program can
//! back any number of independent instances. They are just different accounts.
//!
//! And it cannot tell whether the account it receives is public or private. Privacy is the
//! protocol's business, decided by whoever builds the transaction, not by the program. The
//! same bytecode runs on the sequencer for a public account and locally under a ZK proof for
//! a private one.

use lee_core::program::{ReceiveInput, Response};

lee_core::actor_entrypoint!(handle_message);

/// Appends the greeting to this program's actor state on the receiving account.
///
/// It never checks `input.is_authorized`, so anyone can append to any account's greeting.
/// That is fine for a greeting and wrong for anything holding value.
fn handle_message(input: &ReceiveInput, greeting: Vec<u8>) -> Response {
    let mut bytes = input.pre_state.to_vec();
    bytes.extend(greeting);
    Response::set_state(bytes)
}
