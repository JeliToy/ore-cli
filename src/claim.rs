use std::str::FromStr;

use ore::{self, state::Proof, utils::AccountDeserialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::pubkey::Pubkey;
use solana_sdk::{
    commitment_config::CommitmentConfig, compute_budget::ComputeBudgetInstruction,
    signature::Signer,
};

use crate::{
    cu_limits::{CU_LIMIT_ATA, CU_LIMIT_CLAIM},
    utils::proof_pubkey,
    Miner,
};

impl Miner {
    pub async fn claim(&self, cluster: String, beneficiary: Option<String>) {
        let client = RpcClient::new_with_commitment(cluster, CommitmentConfig::confirmed());
        let beneficiary = match beneficiary {
            Some(beneficiary) => {
                Pubkey::from_str(&beneficiary).expect("Failed to parse beneficiary address")
            }
            None => self.initialize_ata().await,
        };
        let mut pubkey_amounts = Vec::new();
        let mut signer_indexes = Vec::new();
        for (i, signer) in self.signers().iter().enumerate() {
            let data = client
                .get_account(&proof_pubkey(signer.pubkey()))
                .await
                .unwrap()
                .data;
            let proof = Proof::try_from_bytes(
                &data,
            )
            .unwrap();
            if proof.claimable_rewards > 0 {
                pubkey_amounts.push((signer.pubkey(), proof.claimable_rewards));
                signer_indexes.push(i);
            }
        }

        if pubkey_amounts.is_empty() {
            println!("No rewards to claim");
            return;
        }

        println!("Claiming rewards for {:?} miners...", pubkey_amounts.len());

        let cu_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(CU_LIMIT_CLAIM * pubkey_amounts.len() as u32);
        let prio_fee = match self.jito_keypair {
            Some(_) => 1000,
            None => self.priority_fee,
        };
        let cu_price_ix = ComputeBudgetInstruction::set_compute_unit_price(prio_fee);
        let mine_ixs = pubkey_amounts.iter().map(|a|ore::instruction::claim(a.0, beneficiary, a.1));
        let ixs = vec![cu_limit_ix, cu_price_ix].into_iter().chain(mine_ixs).collect::<Vec<_>>();

        println!("Submitting claim transaction...");
        match self
            .send_and_confirm_with_nonce(&ixs, Some(signer_indexes), false)
            .await
        {
            Ok(sig) => {
                println!("Claimed {:} ORE to account {:}", pubkey_amounts.iter().map(|a|a.1).sum::<u64>(), beneficiary);
                println!("{:?}", sig);
            }
            Err(err) => {
                println!("Error: {:?}", err);
            }
        }
    }

    async fn initialize_ata(&self) -> Pubkey {
        // Initialize client.
        let signer = &self.signers()[0];
        let client =
            RpcClient::new_with_commitment(self.cluster.clone(), CommitmentConfig::confirmed());

        // Build instructions.
        let token_account_pubkey = spl_associated_token_account::get_associated_token_address(
            &signer.pubkey(),
            &ore::MINT_ADDRESS,
        );

        // Check if ata already exists
        if let Ok(Some(_ata)) = client.get_token_account(&token_account_pubkey).await {
            return token_account_pubkey;
        }

        // Sign and send transaction.
        let cu_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(CU_LIMIT_ATA);
        let cu_price_ix = ComputeBudgetInstruction::set_compute_unit_price(self.priority_fee);
        let ix = spl_associated_token_account::instruction::create_associated_token_account(
            &signer.pubkey(),
            &signer.pubkey(),
            &ore::MINT_ADDRESS,
            &spl_token::id(),
        );
        println!("Creating token account {}...", token_account_pubkey);
        match self
            .send_and_confirm_with_nonce(&[cu_limit_ix, cu_price_ix, ix], Some(vec![0]), true)
            .await
        {
            Ok(_sig) => println!("Created token account {:?}", token_account_pubkey),
            Err(e) => println!("Transaction failed: {:?}", e),
        }

        // Return token account address
        token_account_pubkey
    }
}
