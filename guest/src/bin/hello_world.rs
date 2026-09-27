//! A LEZ program that appends its instruction bytes to its own shard on the single
//! account it is given.
//!
//! Adapted from `hello_world.rs` in logos-execution-zone's `program_deployment` example.
//!
//! Two things about this program are worth noticing, because they are what makes LEZ
//! different.
//!
//! It holds no state. Everything persistent lives in the accounts passed into the call,
//! which is why one deployed program can back any number of independent instances — they
//! are just different sets of accounts.
//!
//! And it cannot tell whether the account it was handed is public or private. Privacy is
//! the protocol's business, decided by whoever builds the transaction, not by the program.
//! The same bytecode runs on-chain for a public account and locally under a ZK proof for a
//! private one.

use lee_core::{
    account::ShardData,
    program::{Plan, PlanInput, run_program},
};

/// Raw bytes to append. The program gives them no meaning; the caller chooses them.
type Instruction = Vec<u8>;

fn main() {
    run_program(plan, apply)
}

/// Declares what this call intends to touch, before any of it happens.
///
/// LEZ schedules transactions in parallel by comparing these declarations, so two calls
/// naming different accounts run at once. Contention is per account: a program that
/// funnels every call through one shared account serialises itself.
#[expect(
    clippy::needless_pass_by_value,
    reason = "run_program passes the decoded instruction by value; this planner only reads it"
)]
fn plan(input: &PlanInput, instruction: Instruction) -> Plan {
    let mut plan = Plan::new(input);
    let [account] = input.accounts.as_slice() else {
        panic!("Input accounts should consist of a single account");
    };
    plan.effect(account, &instruction);
    plan
}

/// Produces the shard's new contents. Returning `None` would leave the shard untouched.
///
/// Note that this program never checks `is_authorized`, so anyone can append to its shard
/// on any account. That is fine for a greeting and wrong for anything holding value.
#[expect(
    clippy::unnecessary_wraps,
    reason = "run_program's apply returns None to keep a shard unchanged"
)]
fn apply(greeting: Vec<u8>, pre_data: &ShardData) -> Option<Vec<u8>> {
    let mut bytes = pre_data.clone().into_inner();
    bytes.extend(greeting);
    Some(bytes)
}
