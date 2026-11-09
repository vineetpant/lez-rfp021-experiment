//! Helpers for deploying and calling a LEZ program.
//!
//! LEZ programs are stateless: the bytecode holds no data, and everything persistent
//! lives in actor state on the accounts a message reaches. Deploying is therefore a
//! matter of writing the ELF into accounts — one per 96 KiB segment — and then creating
//! a header account that points at that segment chain. The header's account id is what
//! callers use to address the program afterwards.

pub mod chain;

use lee::AccountId;
use program_loader_core::MAX_SEGMENT_DATA_LEN;
use wallet::{WalletCore, WalletSequencerClient, program_facades::program_loader::ProgramLoader};

/// Deploys `bytecode` through the `program_loader` program and returns the header
/// account's id, which addresses the deployed program from then on.
///
/// `payer` must be an existing, funded account. The segment and header accounts are
/// claimed fresh here, and a freshly claimed account holds no balance, so it cannot pay
/// for its own claim.
///
/// The program is deployed immutable. Since v0.4 the loader enforces it: `UpdateHeader`
/// refuses a header created immutable, so nothing can change this program afterwards.
pub async fn deploy_program<C: WalletSequencerClient>(
    wallet_core: &mut WalletCore<C>,
    bytecode: Vec<u8>,
    payer: AccountId,
) -> anyhow::Result<AccountId> {
    // Only the user ELF is uploaded: every LEZ program runs on the protocol's default kernel,
    // so the loader leaves the kernel out and refuses any other segment count.
    let user_elf_len = risc0_binfmt::ProgramBinary::decode(&bytecode)
        .map_err(|e| anyhow::anyhow!("not a RISC Zero program binary: {e}"))?
        .user_elf
        .len();
    let segment_count = user_elf_len.div_ceil(MAX_SEGMENT_DATA_LEN);

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

/// Brings the wallet's private accounts up to `block`, which carries a private transaction this
/// process sent, and saves them. A private account's next transaction spends the state this one
/// created; a wallet still holding the old state would replay a spent commitment, which the
/// sequencer refuses.
pub async fn follow_private_tx<C: WalletSequencerClient>(
    wallet_core: &mut WalletCore<C>,
    block: u64,
) -> anyhow::Result<()> {
    wallet_core.sync_to_block(block).await?;
    wallet_core.store_persistent_data()?;
    Ok(())
}

/// Fee measurement against a running node: what the sequencer actually charged, read from
/// the payer's balance, and split into exec and storage gas with the base fees of the block.
///
/// LEZ v0.4 exposes no receipts yet, so the balance is the only record of a charge. Each
/// measured step must be the payer's only activity between its two snapshots.
pub mod fees {
    use common::{HashType, transaction::LeeTransaction};
    use lee::AccountId;
    use sequencer_service_rpc::{RpcClient as _, SequencerClient};

    /// The payer's balance and the base fees, at one block height.
    #[derive(Debug, Clone, Copy)]
    pub struct Snapshot {
        pub height: u64,
        pub balance: u128,
        pub base_fee_exec: u64,
        pub base_fee_stor: u64,
    }

    pub async fn snapshot(client: &SequencerClient, payer: AccountId) -> anyhow::Result<Snapshot> {
        let quote = client.get_fee_state().await?;
        let balance = client.get_account_balance(payer).await?;
        Ok(Snapshot {
            height: quote.height,
            balance,
            base_fee_exec: quote.base_fee_exec,
            base_fee_stor: quote.base_fee_stor,
        })
    }

    /// Waits until `tx` is in a block, polling every two seconds for up to five minutes.
    ///
    /// A submit returns once the sequencer has accepted the transaction into its mempool, before
    /// any block carries it, so a balance read straight after would miss the charge.
    pub async fn wait_included(client: &SequencerClient, tx: HashType) -> anyhow::Result<u64> {
        wait_included_within(client, tx, 300).await
    }

    /// [`wait_included`], giving up after `secs`.
    pub async fn wait_included_within(client: &SequencerClient, tx: HashType, secs: u64) -> anyhow::Result<u64> {
        for _ in 0..secs.div_ceil(2) {
            if let Some((_, block)) = client.get_transaction(tx).await? {
                return Ok(block);
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        anyhow::bail!("{tx} was not included within {secs} s")
    }

    /// Whether `tx`, included in `block`, reverted. Being included isn't success: a public
    /// action that fails (here, any check a program panics on) is still included and charged,
    /// with its effects undone. v0.4 has no receipts, so the block's list of reverted
    /// transactions is the only record of the outcome.
    pub async fn reverted(client: &SequencerClient, tx: HashType, block: u64) -> anyhow::Result<bool> {
        let block = client
            .get_block(block)
            .await?
            .ok_or_else(|| anyhow::anyhow!("block {block} not found"))?;
        let position = block
            .body
            .transactions
            .iter()
            .position(|t| t.hash() == tx)
            .ok_or_else(|| anyhow::anyhow!("{tx} is not in block {}", block.header.block_id))?;
        Ok(block.body.is_reverted(position))
    }

    /// Prints one `STATUS` line: whether the step's transaction took effect.
    pub async fn report_status(client: &SequencerClient, label: &str, tx: HashType, block: u64) -> anyhow::Result<bool> {
        let reverted = reverted(client, tx, block).await?;
        println!("STATUS step={label} block={block} reverted={reverted}");
        Ok(reverted)
    }

    /// Prints one `FEE` line: the charge between two snapshots, in tokens.
    pub fn report_step(label: &str, before: Snapshot, after: Snapshot) {
        let charged = i128::try_from(before.balance).unwrap_or(i128::MAX)
            - i128::try_from(after.balance).unwrap_or(i128::MAX);
        println!(
            "FEE step={label} charged={charged} blocks={}..{} base_fee_exec={}..{} base_fee_stor={}..{}",
            before.height,
            after.height,
            before.base_fee_exec,
            after.base_fee_exec,
            before.base_fee_stor,
            after.base_fee_stor,
        );
    }

    /// Prints one `FEE` line for a single public transaction, split into gas.
    ///
    /// The node charges `exec_gas·base_fee_exec + bytes·base_fee_stor + tip`, where `bytes`
    /// is the Borsh size of the transaction and `exec_gas` its executed cycles, clamped to
    /// its declared `gas_limit`. Everything but `exec_gas` is read off the transaction, so
    /// `exec_gas` follows. It is exact only while the base fees held still across the step.
    pub async fn report_public_tx(
        client: &SequencerClient,
        label: &str,
        tx: HashType,
        before: Snapshot,
        after: Snapshot,
    ) -> anyhow::Result<()> {
        let (included, block) = client
            .get_transaction(tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("{tx} is not in a block"))?;
        let bytes = borsh::to_vec(&included)?.len() as u128;
        let LeeTransaction::Public(public) = &included else {
            anyhow::bail!("{tx} is not a public transaction");
        };
        let fee = public
            .message()
            .execution
            .fee
            .clone()
            .ok_or_else(|| anyhow::anyhow!("{tx} declares no fee"))?;
        let charged = before.balance - after.balance;
        let stor_cost = bytes * u128::from(after.base_fee_stor);
        let exec_cost = charged - stor_cost - u128::from(fee.tip);
        let steady = before.base_fee_exec == after.base_fee_exec
            && before.base_fee_stor == after.base_fee_stor;
        println!(
            "FEE step={label} tx={tx} block={block} charged={charged} bytes={bytes} \
             stor_gas={bytes} exec_gas={} exec_gas_remainder={} gas_limit={} tip={} \
             base_fee_exec={} base_fee_stor={} base_fees_steady={steady}",
            exec_cost / u128::from(after.base_fee_exec),
            exec_cost % u128::from(after.base_fee_exec),
            fee.gas_limit,
            fee.tip,
            after.base_fee_exec,
            after.base_fee_stor,
        );
        Ok(())
    }
}
