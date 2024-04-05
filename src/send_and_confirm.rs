use std::{
    io::{stdout, Write},
    time::Duration,
};

use solana_client::{
    client_error::{ClientError, ClientErrorKind, Result as ClientResult},
    nonblocking::rpc_client::RpcClient,
    rpc_config::RpcSendTransactionConfig,
};
use solana_program::instruction::Instruction;
use solana_rpc_client_nonce_utils::nonblocking;
use solana_sdk::{
    commitment_config::{CommitmentConfig, CommitmentLevel}, pubkey::Pubkey, signature::{Signature, Signer}, system_instruction, transaction::Transaction
};
use solana_transaction_status::{TransactionConfirmationStatus, UiTransactionEncoding};

use crate::Miner;

const RPC_RETRIES: usize = 0;
const GATEWAY_RETRIES: usize = 4;
const CONFIRM_RETRIES: usize = 5;
const LOOP_SEND_DELAY_MS: u64 = 200;
const LOOP_SEND_COUNT: u64 = 10;

impl Miner {
    pub async fn get_or_create_nonce_acct(&self) -> Pubkey {
        let payer_pubkey = self.signer().pubkey();
        let nonce_pubkey = Pubkey::create_with_seed(&payer_pubkey, "nonce", &solana_program::system_program::ID).unwrap();
        let client = RpcClient::new_with_commitment(self.cluster.clone(), CommitmentConfig::confirmed());
        let opt_nonce_account = client.get_account_with_commitment(&nonce_pubkey, CommitmentConfig { commitment: CommitmentLevel::Confirmed }).await.unwrap().value;
        if opt_nonce_account.is_none() {
            println!("Creating nonce account {} from base {}", nonce_pubkey, payer_pubkey);
            let nonce_lamports = client.get_minimum_balance_for_rent_exemption(80).await.unwrap();
            let ixs = system_instruction::create_nonce_account_with_seed(&payer_pubkey, &nonce_pubkey, &payer_pubkey, "nonce", &payer_pubkey, nonce_lamports);
            self.send_and_confirm(&ixs, false).await.unwrap();
            println!("Created nonce account");
        }
        nonce_pubkey
    }

    pub async fn send_and_confirm_with_nonce(
        &self,
        ixs: &[Instruction],
    ) -> ClientResult<Signature> {
        let signer = self.signer();
        let signer_pubkey = signer.pubkey();
        let client =
            RpcClient::new_with_commitment(self.cluster.clone(), CommitmentConfig::confirmed());

        let nonce_pubkey = self.get_or_create_nonce_acct().await;
        let nonce_account = client.get_account(&nonce_pubkey).await.unwrap();
        let nonce_data = nonblocking::data_from_account(&nonce_account).unwrap();
        let advance_ix = system_instruction::advance_nonce_account(&nonce_pubkey, &signer_pubkey);

        let mut new_ixs = vec![advance_ix];
        new_ixs.extend_from_slice(ixs);

        let mut tx = Transaction::new_with_payer(&new_ixs, Some(&self.signer().pubkey()));
        tx.sign(&[&signer], nonce_data.blockhash());

        let sig = tx.signatures.get(0).unwrap();

        let sim_res = client.simulate_transaction(&tx).await.unwrap();
        if sim_res.value.err.is_some() {
            println!("Simulation failed: {:?}", sim_res.value.err);
            return Err(ClientError {
                request: None,
                kind: ClientErrorKind::Custom("Simulation failed".into()),
            });
        }   
        
        let send_cfg = RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: Some(CommitmentLevel::Confirmed),
            encoding: Some(UiTransactionEncoding::Base64),
            max_retries: Some(3),
            min_context_slot: None,
        };

        let mut cnt = 0;
        loop {
            println!("Sending nonced transaction {}", sig);
            let sig = client.send_transaction_with_config(&tx, send_cfg).await.unwrap();

            tokio::time::sleep(Duration::from_millis(2000)).await;

            if client.get_signature_status_with_commitment(&sig, CommitmentConfig { commitment: CommitmentLevel::Confirmed }).await.unwrap().is_some() {
                println!("Transaction landed!");
                return Ok(sig)
            }
            cnt += 1;
            println!("Transaction did not land {}", cnt);
        }
    }

    pub async fn send_and_confirm(
        &self,
        ixs: &[Instruction],
        skip_confirm: bool,
    ) -> ClientResult<Signature> {
        let mut stdout = stdout();
        let signer = self.signer();
        let client =
            RpcClient::new_with_commitment(self.cluster.clone(), CommitmentConfig::confirmed());

        // Return error if balance is zero
        let balance = client
            .get_balance_with_commitment(&signer.pubkey(), CommitmentConfig::confirmed())
            .await
            .unwrap();
        if balance.value <= 0 {
            return Err(ClientError {
                request: None,
                kind: ClientErrorKind::Custom("Insufficient SOL balance".into()),
            });
        }

        // Build tx
        let (mut hash, mut slot) = client
            .get_latest_blockhash_with_commitment(CommitmentConfig::confirmed())
            .await
            .unwrap();
        let mut send_cfg = RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: Some(CommitmentLevel::Confirmed),
            encoding: Some(UiTransactionEncoding::Base64),
            max_retries: Some(RPC_RETRIES),
            min_context_slot: Some(slot),
        };
        let mut tx = Transaction::new_with_payer(ixs, Some(&signer.pubkey()));
        tx.sign(&[&signer], hash);
        
        // Submit tx
        let mut sigs = vec![];

        // Loop
        let mut attempts = 0;
        let wait = Duration::from_millis(LOOP_SEND_DELAY_MS);
        loop {
            println!("Attempt: {:?}", attempts);
            let spam = client.send_transaction_with_config(&tx, send_cfg).await;
            for _ in 0..LOOP_SEND_COUNT {
                tokio::time::sleep(wait).await;
                let _ = client.send_transaction_with_config(&tx, send_cfg).await;
            }
            match spam {
                Ok(sig) => {
                    sigs.push(sig);
                    println!("{:?}", sig);

                    // Confirm tx
                    if skip_confirm {
                        return Ok(sig);
                    }
                    for _ in 0..CONFIRM_RETRIES {
                        std::thread::sleep(Duration::from_millis(2000));
                        match client.get_signature_statuses(&sigs).await {
                            Ok(signature_statuses) => {
                                println!("Confirms: {:?}", signature_statuses.value);
                                for signature_status in signature_statuses.value {
                                    if let Some(signature_status) = signature_status.as_ref() {
                                        if signature_status.confirmation_status.is_some() {
                                            let current_commitment = signature_status
                                                .confirmation_status
                                                .as_ref()
                                                .unwrap();
                                            match current_commitment {
                                                TransactionConfirmationStatus::Processed => {}
                                                TransactionConfirmationStatus::Confirmed
                                                | TransactionConfirmationStatus::Finalized => {
                                                    println!("Transaction landed!");
                                                    return Ok(sig);
                                                }
                                            }
                                        } else {
                                            println!("No status");
                                        }
                                    }
                                }
                            }

                            // Handle confirmation errors
                            Err(err) => {
                                println!("Error: {:?}", err);
                            }
                        }
                    }
                    println!("Transaction did not land");
                }

                // Handle submit errors
                Err(err) => {
                    println!("Error {:?}", err);
                }
            }
            stdout.flush().ok();

            // Retry
            std::thread::sleep(Duration::from_millis(200));
            (hash, slot) = client
                .get_latest_blockhash_with_commitment(CommitmentConfig::confirmed())
                .await
                .unwrap();
            send_cfg = RpcSendTransactionConfig {
                skip_preflight: true,
                preflight_commitment: Some(CommitmentLevel::Confirmed),
                encoding: Some(UiTransactionEncoding::Base64),
                max_retries: Some(RPC_RETRIES),
                min_context_slot: Some(slot),
            };
            tx.sign(&[&signer], hash);
            attempts += 1;
            if attempts > GATEWAY_RETRIES {
                return Err(ClientError {
                    request: None,
                    kind: ClientErrorKind::Custom("Max retries".into()),
                });
            }
        }
    }
}
