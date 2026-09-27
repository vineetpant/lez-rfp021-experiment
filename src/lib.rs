//! Helpers for deploying and calling a LEZ program.
//!
//! LEZ programs are stateless: the bytecode holds no data, and everything persistent
//! lives in accounts handed to it explicitly on each call. Deploying is therefore a
//! matter of writing the ELF into accounts — one per 96 KiB segment — and then creating
//! a header account that points at that segment chain. The header's account id is what
//! callers use to address the program afterwards.

use lee::AccountId;
use program_loader_core::MAX_SEGMENT_DATA_LEN;
use wallet::{WalletCore, program_facades::program_loader::ProgramLoader};

/// Deploys `bytecode` through the `program_loader` program and returns the header
/// account's id, which addresses the deployed program from then on.
///
/// `payer` must be an existing, funded account. The segment and header accounts are
/// claimed fresh here, and a freshly claimed account holds no balance, so it cannot pay
/// for its own claim.
///
/// The program is deployed as self-declared immutable. That declaration is not enforced
/// by the protocol — it records intent, and nothing stops a program that exposes a
/// mutating instruction from being changed through it.
pub async fn deploy_program(
    wallet_core: &mut WalletCore,
    bytecode: Vec<u8>,
    payer: AccountId,
) -> anyhow::Result<AccountId> {
    let segment_count = bytecode.len().div_ceil(MAX_SEGMENT_DATA_LEN);

    let header = wallet_core.create_new_account_public(None).0;
    let segments: Vec<AccountId> =
        std::iter::repeat_with(|| wallet_core.create_new_account_public(None).0)
            .take(segment_count)
            .collect();
    // Persist the new keys before submitting: if deployment fails after the accounts are
    // claimed on-chain, the wallet must still hold the keys to them.
    wallet_core.store_persistent_data()?;

    ProgramLoader(wallet_core)
        .deploy(header, &segments, bytecode, true, Some(payer))
        .await
}
