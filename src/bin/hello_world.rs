//! Deploys the `hello_world` guest program, calls it publicly or privately, and prints
//! where to read the result.
//!
//! The program appends its message bytes to its own actor state on the account it
//! receives, so calling it with `Hola mundo!` leaves that greeting in the account.
//!
//! Usage:
//!   hello_world <program.bin> <target-account-id> <funded-payer-account-id> [private]
//!
//! With `private`, the target must be a private account this wallet owns. The call then
//! runs locally under a ZK proof and only the proof reaches the sequencer. The in-proof
//! fee is paid from a funded private account the wallet owns, and the payer account only
//! receives the fee surplus back.
//!
//! Requires `LEE_WALLET_HOME_DIR` to point at a wallet home; the wallet prompts for the
//! password that seeds its keys.

use lez_hello_world::{deploy_program, fees, follow_private_tx};

use lee::{AccountId, privacy_preserving_transaction::circuit::ProgramCatalog, program::Program};
use wallet::{AccountIdentity, WalletCore};

const GREETING: &[u8] = b"Hola mundo!";
const USAGE: &str =
    "usage: hello_world <program.bin> <target-account-id> <funded-payer-account-id> [private]";

fn arg(n: usize, what: &str) -> String {
    std::env::args_os()
        .nth(n)
        .unwrap_or_else(|| {
            eprintln!("missing argument {n}: {what}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        })
        .into_string()
        .expect("argument should be valid UTF-8")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let program_path = arg(1, "path to the compiled guest binary");
    let target: AccountId = arg(2, "account to write the greeting into").parse()?;
    let payer: AccountId = arg(3, "funded account paying the fees").parse()?;
    let private = match std::env::args().nth(4).as_deref() {
        None => false,
        Some("private") => true,
        Some(other) => anyhow::bail!("unknown mode {other:?}\n{USAGE}"),
    };

    let mut wallet_core = WalletCore::from_env()?;

    let bytecode = std::fs::read(&program_path)
        .map_err(|e| anyhow::anyhow!("cannot read {program_path}: {e}"))?;
    let client = wallet_core.client_owned();
    let before_deploy = fees::snapshot(&client, payer).await?;
    let program = deploy_program(&mut wallet_core, bytecode.clone(), payer).await?;
    println!("deployed at header account {program}");
    let after_deploy = fees::snapshot(&client, payer).await?;
    fees::report_step("deploy", before_deploy, after_deploy);

    let message = Program::serialize_message(GREETING.to_vec())?;
    let tx = if private {
        // The prover runs the program locally, so it needs the bytecode, keyed by the header
        // the transaction addresses.
        let programs = ProgramCatalog::from([(program, Program::new(bytecode.into())?)]);
        // The in-proof fee reserve's surplus normally goes back to the payer's blinded alias,
        // where it can't be seen. Refunding it to the public payer instead makes the charged
        // fee measurable as reserve minus refund, at the cost of linking the call to that
        // account. A measurement choice, not what a privacy-minded caller would do.
        wallet_core.set_private_refund_target(Some(payer));
        let account = AccountIdentity::PrivateOwned(target).select_program_actor_state(program);
        let (tx, _) = wallet_core
            .send_privacy_preserving_tx(vec![account], 0, message, &programs)
            .await
            .map_err(|e| anyhow::anyhow!("submitting the private call failed: {e:?}"))?;
        tx
    } else {
        // `hello_world` writes its own actor state without consulting `is_authorized`, so
        // the target does not need to sign. `PublicNoSign` says so explicitly; the payer
        // still signs, because a charged transaction's fee payer is authorized by an
        // ordinary witness signature.
        let account = AccountIdentity::PublicNoSign(target).select_program_actor_state(program);
        wallet_core
            .send_pub_tx_paid_by(vec![account], 0, message, Some(payer))
            .await
            .map_err(|e| anyhow::anyhow!("submitting the call failed: {e:?}"))?
    };
    println!("submitted tx {tx}");

    let block = fees::wait_included(&client, tx).await?;
    println!("included in block {block}");
    if private {
        follow_private_tx(&mut wallet_core, block).await?;
    }
    let after_call = fees::snapshot(&client, payer).await?;
    if private {
        // Negative: the payer only received the surplus of the in-proof fee reserve.
        fees::report_step("call-private-refund", after_deploy, after_call);
    } else {
        fees::report_public_tx(&client, "call-public", tx, after_deploy, after_call).await?;
    }

    let scope = if private { "Private" } else { "Public" };
    println!(
        "\nread it back with:\n  wallet account get --raw --scope {program} --account-id {scope}/{target}"
    );

    Ok(())
}
