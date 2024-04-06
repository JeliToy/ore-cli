use std::{
    io::{stdout, Write},
    sync::{atomic::AtomicBool, Arc, Mutex},
};

use ore::{self, state::Bus, BUS_ADDRESSES, BUS_COUNT};
use rand::Rng;
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction, keccak::{hashv, Hash as KeccakHash}, pubkey::Pubkey, signature::Signer
};

use crate::{
    cu_limits::CU_LIMIT_MINE,
    utils::{get_proof, get_treasury},
    Miner,
};

impl Miner {
    pub async fn mine(&self, threads: u64, auto_claim: bool, beneficiary: Option<String>) {
        // Register, if needed.
        self.register().await;
        let signers = self.signers();
        let mut stdout = stdout();

        let mut count = 0_u16;

        // Start mining loop
        loop {
            // Fetch account state
            let treasury = get_treasury(self.cluster.clone()).await;
            let mut proofs = Vec::new();
            let mut rewards = Vec::new();

            for signer in signers.iter() {
                let proof = get_proof(self.cluster.clone(), signer.pubkey()).await;
                proofs.push(proof);
                let reward = (proof.claimable_rewards as f64) / (10f64.powf(ore::TOKEN_DECIMALS as f64));
                rewards.push(reward);
            }
            let reward_rate =
                (treasury.reward_rate as f64) / (10f64.powf(ore::TOKEN_DECIMALS as f64));

            // Escape sequence that clears the screen and the scrollback buffer
            stdout.write_all(b"\x1b[2J\x1b[3J\x1b[H").ok();

            println!("Claimable: {:?}", rewards.iter().sum::<f64>());
            println!("Reward rate: {} ORE", reward_rate);
            if auto_claim {
                println!("Auto-claiming rewards every 10 mines");
            }

            if auto_claim && count % 10 == 0 {
                println!("Auto-claiming rewards...");
                self.claim(self.cluster.clone(), beneficiary.clone()).await;
            }
            count += 1;

            println!("\nMining for a valid hash...");
            let new_solutions = signers.iter().enumerate().map(|(i, signer)| {
                    let (hash, nonce) = Self::find_next_hash_par(signer.pubkey(), proofs[i].hash.into(), treasury.difficulty.into(), threads);
                    (signer, hash, nonce)
            }).collect::<Vec<_>>();

            // Submit mine tx.
            // Use busses randomly so on each epoch, transactions don't pile on the same busses
            println!("\n\nSubmitting hash for validation...");
            loop {
                // Submit request.
                let bus = self.find_bus_id(treasury.reward_rate).await;
                let bus_rewards = (bus.rewards as f64) / (10f64.powf(ore::TOKEN_DECIMALS as f64));
                println!("Sending on bus {} ({} ORE)", bus.id, bus_rewards);
                let cu_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(CU_LIMIT_MINE * signers.len() as u32);
                let cu_price_ix =
                    ComputeBudgetInstruction::set_compute_unit_price(self.priority_fee);
                let ixs_mine = new_solutions.iter().map(|a|ore::instruction::mine(
                    a.0.pubkey(),
                    BUS_ADDRESSES[bus.id as usize],
                    a.1.into(),
                    a.2,
                ));
                let ixs: Vec<_> = vec![cu_limit_ix, cu_price_ix].into_iter().chain(ixs_mine).collect();
                match self
                    .send_and_confirm_with_nonce(&ixs, None)
                    .await
                {
                    Ok(sig) => {
                        println!("Success: {}", sig);
                        break;
                    }
                    Err(_err) => {
                        // TODO
                    }
                }
            }
        }
    }

    async fn find_bus_id(&self, reward_rate: u64) -> Bus {
        let mut rng = rand::thread_rng();
        loop {
            let bus_id = rng.gen_range(0..BUS_COUNT);
            if let Ok(bus) = self.get_bus(bus_id).await {
                if bus.rewards.gt(&reward_rate.saturating_mul(4)) {
                    return bus;
                }
            }
        }
    }

    fn find_next_hash_par(
        pubkey: Pubkey,
        hash: KeccakHash,
        difficulty: KeccakHash,
        threads: u64,
    ) -> (KeccakHash, u64) {
        let found_solution = Arc::new(AtomicBool::new(false));
        let solution = Arc::new(Mutex::<(KeccakHash, u64)>::new((
            KeccakHash::new_from_array([0; 32]),
            0,
        )));
        let thread_handles: Vec<_> = (0..threads)
            .map(|i| {
                std::thread::spawn({
                    let found_solution = found_solution.clone();
                    let solution = solution.clone();
                    let mut stdout = stdout();
                    move || {
                        let n = u64::MAX.saturating_div(threads).saturating_mul(i);
                        let mut next_hash: KeccakHash;
                        let mut nonce: u64 = n;
                        loop {
                            next_hash = hashv(&[
                                hash.to_bytes().as_slice(),
                                pubkey.to_bytes().as_slice(),
                                nonce.to_le_bytes().as_slice(),
                            ]);
                            if nonce % 10_000 == 0 {
                                if found_solution.load(std::sync::atomic::Ordering::Relaxed) {
                                    return;
                                }
                                if n == 0 {
                                    stdout
                                        .write_all(
                                            format!("\r{}", next_hash.to_string()).as_bytes(),
                                        )
                                        .ok();
                                }
                            }
                            if next_hash.le(&difficulty) {
                                stdout
                                    .write_all(format!("\r{}", next_hash.to_string()).as_bytes())
                                    .ok();
                                found_solution.store(true, std::sync::atomic::Ordering::Relaxed);
                                let mut w_solution = solution.lock().expect("failed to lock mutex");
                                *w_solution = (next_hash, nonce);
                                return;
                            }
                            nonce += 1;
                        }
                    }
                })
            })
            .collect();

        for thread_handle in thread_handles {
            thread_handle.join().unwrap();
        }

        let r_solution = solution.lock().expect("Failed to get lock");
        *r_solution
    }
}
