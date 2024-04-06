use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig, compute_budget::ComputeBudgetInstruction,
    signature::Signer,
};

use crate::{cu_limits::CU_LIMIT_REGISTER, utils::proof_pubkey, Miner};

impl Miner {
    pub async fn register(&self) {
        // Return early if miner is already registered
        let mut signers_needing_register = Vec::new();
        let mut signer_indexes = Vec::new();
        let signers = self.signers();
        print!("Checking if {} miners are registered...", signers.len());
        let client =
            RpcClient::new_with_commitment(self.cluster.clone(), CommitmentConfig::confirmed());
        for (i, signer) in signers.iter().enumerate() {
            let proof_address = proof_pubkey(signer.pubkey());
            if client.get_account(&proof_address).await.is_err() {
                signers_needing_register.push(signer);
                signer_indexes.push(i);
            }
        }
        println!("{} miners need to register", signers_needing_register.len());

        if signers_needing_register.is_empty() {
            return;
        }

        println!("Generating challenge...");
        
        let cu_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(CU_LIMIT_REGISTER * signers_needing_register.len() as u32);
        let cu_price_ix = ComputeBudgetInstruction::set_compute_unit_price(self.priority_fee);
        let ixs_iter = signers_needing_register.iter().map(|a|ore::instruction::register(a.pubkey()));
        let ixs: Vec<_> = vec![cu_limit_ix, cu_price_ix].into_iter().chain(ixs_iter).collect();

        self.send_and_confirm_with_nonce(&ixs, Some(signer_indexes))
            .await
            .expect("Transaction failed");
    }
}
