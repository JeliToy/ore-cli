mod balance;
mod busses;
mod claim;
mod cu_limits;
#[cfg(feature = "admin")]
mod initialize;
mod mine;
mod register;
mod rewards;
mod send_and_confirm;
mod treasury;
#[cfg(feature = "admin")]
mod update_admin;
#[cfg(feature = "admin")]
mod update_difficulty;
mod utils;

use std::{path::Path, sync::Arc};

use clap::{command, Parser, Subcommand};
use solana_sdk::signature::{read_keypair_file, Keypair};

struct Miner {
    pub keypairs: Vec<Keypair>,
    pub priority_fee: u64,
    pub cluster: String,
}

#[derive(Parser, Debug)]
#[command(about, version)]
struct Args {
    #[arg(
        long,
        value_name = "NETWORK_URL",
        help = "Network address of your RPC provider",
        default_value = "https://api.mainnet-beta.solana.com"
    )]
    rpc: String,

    #[arg(
        long,
        value_name = "KEYPAIR_FILEPATH",
        help = "Filepath to keypair to use. To use multiple, provide the first and name the others with a number suffix. E.g. key.json, key1.json, key2.json, ...",
    )]
    keypair: Option<String>,

    #[arg(
        long,
        value_name = "MICROLAMPORTS",
        help = "Number of microlamports to pay as priority fee per transaction",
        default_value = "10"
    )]
    priority_fee: u64,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(about = "Fetch the Ore balance of an account")]
    Balance(BalanceArgs),

    #[command(about = "Fetch the distributable rewards of the busses")]
    Busses(BussesArgs),

    #[command(about = "Mine Ore using local compute")]
    Mine(MineArgs),

    #[command(about = "Claim available mining rewards")]
    Claim(ClaimArgs),

    #[command(about = "Fetch your balance of unclaimed mining rewards")]
    Rewards(RewardsArgs),

    #[command(about = "Fetch the treasury account and balance")]
    Treasury(TreasuryArgs),

    #[cfg(feature = "admin")]
    #[command(about = "Initialize the program")]
    Initialize(InitializeArgs),

    #[cfg(feature = "admin")]
    #[command(about = "Update the program admin authority")]
    UpdateAdmin(UpdateAdminArgs),

    #[cfg(feature = "admin")]
    #[command(about = "Update the mining difficulty")]
    UpdateDifficulty(UpdateDifficultyArgs),
}

#[derive(Parser, Debug)]
struct BalanceArgs {
    #[arg(
        // long,
        value_name = "ADDRESS",
        help = "The address of the account to fetch the balance of"
    )]
    pub address: Option<String>,
}

#[derive(Parser, Debug)]
struct BussesArgs {}

#[derive(Parser, Debug)]
struct RewardsArgs {
    #[arg(
        // long,
        value_name = "ADDRESS",
        help = "The address of the account to fetch the rewards balance of"
    )]
    pub address: Option<String>,
}

#[derive(Parser, Debug)]
struct MineArgs {
    #[arg(
        long,
        short,
        value_name = "THREAD_COUNT",
        help = "The number of threads to dedicate to mining",
        default_value = "1"
    )]
    threads: u64,

    #[arg(
        long,
        short,
        value_name = "AUTO_CLAIM",
        help = "To automatically claim rewards every 10 mines. Defaults to false.",
        action,
    )]
    auto_claim: bool,

    #[arg(
        long,
        value_name = "TOKEN_ACCOUNT_ADDRESS",
        help = "Token account to receive mining rewards."
    )]
    beneficiary: Option<String>,
}

#[derive(Parser, Debug)]
struct TreasuryArgs {}

#[derive(Parser, Debug)]
struct ClaimArgs {
    #[arg(
        // long,
        value_name = "AMOUNT",
        help = "The amount of rewards to claim. Defaults to max."
    )]
    amount: Option<f64>,

    #[arg(
        long,
        value_name = "TOKEN_ACCOUNT_ADDRESS",
        help = "Token account to receive mining rewards."
    )]
    beneficiary: Option<String>,
}

#[cfg(feature = "admin")]
#[derive(Parser, Debug)]
struct InitializeArgs {}

#[cfg(feature = "admin")]
#[derive(Parser, Debug)]
struct UpdateAdminArgs {
    new_admin: String,
}

#[cfg(feature = "admin")]
#[derive(Parser, Debug)]
struct UpdateDifficultyArgs {}

#[tokio::main]
async fn main() {
    loop {
        // Initialize miner.
        let args = Args::parse();
        let cluster = args.rpc;
        let miner = Arc::new(Miner::new(cluster.clone(), args.priority_fee, args.keypair));

        // Execute user command.
        match args.command {
            Commands::Balance(args) => {
                miner.balance(args.address).await;
            }
            Commands::Busses(_) => {
                miner.busses().await;
            }
            Commands::Rewards(args) => {
                miner.rewards(args.address).await;
            }
            Commands::Treasury(_) => {
                miner.treasury().await;
            }
            Commands::Mine(args) => {
                miner.mine(args.threads, args.auto_claim, args.beneficiary).await;
            }
            Commands::Claim(args) => {
                miner.claim(cluster, args.beneficiary).await;
            }
            #[cfg(feature = "admin")]
            Commands::Initialize(_) => {
                miner.initialize().await;
            }
            #[cfg(feature = "admin")]
            Commands::UpdateAdmin(args) => {
                miner.update_admin(args.new_admin).await;
            }
            #[cfg(feature = "admin")]
            Commands::UpdateDifficulty(_) => {
                miner.update_difficulty().await;
            }
        }
    }
}

impl Miner {
    pub fn new(cluster: String, priority_fee: u64, keypair_filepath: Option<String>) -> Self {
        let mut keypairs = Vec::new();
        match keypair_filepath.clone() {
            Some(filepath) => {
                let mut f = filepath.clone();
                let temp = f.clone();
                let path = Path::new(&temp);
                let orig_filename = path.file_name().unwrap().to_str().unwrap().split('.').next().unwrap();
                let mut i = 1_u8;
                while let Ok(key) = read_keypair_file(f) {
                    keypairs.push(key);
                    f = path.with_file_name(format!("{}{}.json", orig_filename, i)).to_str().unwrap().to_string();
                    i += 1;
                }
            }
            None => panic!("No keypair provided"),
        }
        if keypairs.len() == 0 {
            panic!("No keypair found");
        }
        println!("Found {} keypairs", keypairs.len());
        Self {
            keypairs,
            priority_fee,
            cluster,
        }
    }

    pub fn signers(&self) -> &[Keypair] {
        &self.keypairs
    }
}
