//! Deploys the `hello_world` guest program, calls it, and prints where to read the
//! result.
//!
//! The program appends its instruction bytes to its own shard on the account it is
//! given, so calling it with `Hola mundo!` leaves that greeting in the account.
//!
//! Usage:
//!   hello_world <path/to/hello_world.bin> <target-account-id> <funded-payer-account-id>
//!
//! Requires `LEE_WALLET_HOME_DIR` to point at a wallet home; the wallet prompts for the
//! password that seeds its keys.

use lez_hello_world::deploy_program;

use lee::{AccountId, program::Program};
use wallet::{AccountIdentity, WalletCore};

const GREETING: &[u8] = b"Hola mundo!";

fn arg(n: usize, what: &str) -> String {
    std::env::args_os()
        .nth(n)
        .unwrap_or_else(|| {
            eprintln!("missing argument {n}: {what}");
            eprintln!(
                "usage: hello_world <program.bin> <target-account-id> <funded-payer-account-id>"
            );
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

    let mut wallet_core = WalletCore::from_env().await?;

    let bytecode = std::fs::read(&program_path)
        .map_err(|e| anyhow::anyhow!("cannot read {program_path}: {e}"))?;
    let program = deploy_program(&mut wallet_core, bytecode, payer).await?;
    println!("deployed at header account {program}");

    // `hello_world` writes its own shard without consulting `is_authorized`, so the
    // target account does not need to sign for it. `PublicNoSign` says so explicitly;
    // the payer still co-signs, because a charged transaction's fee payer is authorized
    // by an ordinary witness signature.
    let accounts = vec![AccountIdentity::PublicNoSign(target).select_program_shard(program)];

    // `send_pub_tx_paid_by` assembles the shard selectors, the nonces (including the
    // payer's), the fee declaration and the witness set.
    //
    // Building the message by hand with `Message::try_new` — as upstream's own examples
    // do — produces a fee-exempt message, which the sequencer rejects with
    // `InvalidParams: "Incorrect fee"`. Fee exemption is for system transactions; an
    // ordinary program call is charged.
    let tx = wallet_core
        .send_pub_tx_paid_by(
            accounts,
            Program::serialize_instruction(GREETING.to_vec())?,
            program,
            Some(payer),
        )
        .await
        .map_err(|e| anyhow::anyhow!("submitting the call failed: {e:?}"))?;
    println!("submitted tx {tx:?}");

    // `--scope all` reports an uninitialised account even when the shard holds data, so
    // the read has to name the program.
    println!(
        "\nread it back with:\n  wallet account get --raw --scope {program} --account-id Public/{target}"
    );

    Ok(())
}
