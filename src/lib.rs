#![no_std]

use multiversx_sc::imports::*;

/// Payout house edge 5.00%.
const HOUSE_EDGE_BPS: u64 = 500;
/// Instant rake 2.00% of each bet. Accrued in-contract; only skimmed when buffer is healthy.
const RAKE_BPS: u64 = 200;
const BPS_DENOM: u64 = 10_000;
const MIN_UNDER: u64 = 2;
const MAX_UNDER: u64 = 96;
const ROLL_MOD: u64 = 100;
const MAX_BETS_PER_RESOLVE: usize = 50;
/// Max single bet = 2% of free bankroll.
const MAX_BET_FREE_BPS: u64 = 200;
/// Keep at least 2× min_bankroll before any treasury skim.
const SKIM_BUFFER_MULT: u32 = 2;

#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, TypeAbi, Clone)]
pub struct Bet<M: ManagedTypeApi> {
    pub player: ManagedAddress<M>,
    pub amount: BigUint<M>,
    pub under: u64,
    pub settled: bool,
    pub won: bool,
    pub roll: u64,
}

#[multiversx_sc::contract]
pub trait PridevaultCasino {
    #[init]
    fn init(
        &self,
        treasury: ManagedAddress,
        min_bet: BigUint,
        cap_max_bet: BigUint,
        round_blocks: u64,
        min_bankroll: BigUint,
    ) {
        require!(!treasury.is_zero(), "treasury required");
        require!(min_bet > 0, "min bet");
        require!(cap_max_bet >= min_bet, "max < min");
        require!(round_blocks >= 2, "round too short");
        require!(min_bankroll > 0, "min bankroll");

        self.treasury().set(treasury);
        self.min_bet().set(min_bet);
        self.cap_max_bet().set(cap_max_bet);
        self.round_blocks().set(round_blocks);
        self.min_bankroll().set(min_bankroll);
        self.is_paused().set(false);
        self.round_id().set(0u64);
    }

    #[upgrade]
    fn upgrade(&self) {}

    #[only_owner]
    #[payable("EGLD")]
    #[endpoint(fundBankroll)]
    fn fund_bankroll(&self) {
        let amount = self.call_value().egld().clone();
        require!(amount > 0, "zero");
        self.bankroll_funded_event(&amount);
    }

    #[only_owner]
    #[endpoint(setParams)]
    fn set_params(
        &self,
        min_bet: BigUint,
        cap_max_bet: BigUint,
        round_blocks: u64,
        min_bankroll: BigUint,
    ) {
        require!(self.round_open().is_empty() || !self.round_open().get(), "round open");
        require!(min_bet > 0 && cap_max_bet >= min_bet, "bad bets");
        require!(round_blocks >= 2, "round too short");
        require!(min_bankroll > 0, "min bankroll");
        self.min_bet().set(min_bet);
        self.cap_max_bet().set(cap_max_bet);
        self.round_blocks().set(round_blocks);
        self.min_bankroll().set(min_bankroll);
    }

    #[only_owner]
    #[endpoint(setTreasury)]
    fn set_treasury(&self, treasury: ManagedAddress) {
        require!(!treasury.is_zero(), "treasury required");
        self.treasury().set(treasury);
    }

    #[only_owner]
    #[endpoint(pause)]
    fn pause(&self) {
        self.is_paused().set(true);
    }

    #[only_owner]
    #[endpoint(unpause)]
    fn unpause(&self) {
        let free = self.free_bankroll();
        require!(free >= self.min_bankroll().get(), "bankroll too thin");
        self.is_paused().set(false);
    }

    #[only_owner]
    #[endpoint(startRound)]
    fn start_round(&self) {
        self.require_not_paused();
        require!(
            self.round_open().is_empty() || !self.round_open().get(),
            "already open"
        );
        require!(!self.has_unsettled_bets(), "finish previous resolve");
        require!(self.free_bankroll() >= self.min_bankroll().get(), "bankroll too thin");

        let next_id = self.round_id().get() + 1;
        self.round_id().set(next_id);
        self.round_open().set(true);
        let end = self.blockchain().get_block_nonce() + self.round_blocks().get();
        self.round_end_block().set(end);
        self.round_liability().set(BigUint::zero());
        self.round_seed().clear();
        self.bets().clear();
        self.round_started_event(next_id, end);
    }

    #[payable("EGLD")]
    #[endpoint(placeBet)]
    fn place_bet(&self, under: u64) {
        self.require_not_paused();
        require!(!self.round_open().is_empty() && self.round_open().get(), "no open round");
        require!(
            self.blockchain().get_block_nonce() < self.round_end_block().get(),
            "betting closed"
        );
        require!(under >= MIN_UNDER && under <= MAX_UNDER, "under 2..96");

        let amount = self.call_value().egld().clone();
        require!(amount >= self.min_bet().get(), "below min");

        let dyn_max = self.dynamic_max_bet();
        require!(amount <= dyn_max, "above dynamic max");
        require!(amount <= self.cap_max_bet().get(), "above cap");

        let rake = &amount * RAKE_BPS / BPS_DENOM;
        let net = &amount - &rake;
        let payout = self.potential_payout(&net, under);
        let new_liability = self.round_liability().get() + &payout;

        self.round_liability().set(&new_liability);
        require!(
            self.free_bankroll() >= self.min_bankroll().get(),
            "insufficient bankroll"
        );

        if rake > 0 {
            self.rake_accrued().update(|r| *r += rake);
        }

        let player = self.blockchain().get_caller();
        self.bets().push(&Bet {
            player: player.clone(),
            amount: net,
            under,
            settled: false,
            won: false,
            roll: 0,
        });

        self.bet_placed_event(self.round_id().get(), &player, &amount, under);
    }

    #[endpoint(resolveRound)]
    fn resolve_round(&self) {
        let open = !self.round_open().is_empty() && self.round_open().get();
        let resolving = !self.round_seed().is_empty();
        require!(open || resolving, "nothing to resolve");
        require!(
            self.blockchain().get_block_nonce() >= self.round_end_block().get(),
            "too early"
        );

        if self.round_seed().is_empty() {
            let mut rng = RandomnessSource::new();
            let seed = rng.next_u64();
            self.round_seed().set(seed);
            self.round_open().set(false);
            self.round_resolved_event(self.round_id().get(), seed);
        }

        let seed = self.round_seed().get();
        let len = self.bets().len();
        let mut processed = 0usize;

        for i in 1..=len {
            if processed >= MAX_BETS_PER_RESOLVE {
                break;
            }
            let mut bet = self.bets().get(i);
            if bet.settled {
                continue;
            }

            let roll = self.roll_for(seed, i as u64);
            let won = roll < bet.under;
            bet.roll = roll;
            bet.won = won;
            bet.settled = true;
            self.bets().set(i, &bet);

            if won {
                let payout = self.potential_payout(&bet.amount, bet.under);
                self.claimable(&bet.player).update(|c| *c += &payout);
                self.total_claimable().update(|t| *t += payout);
            }
            processed += 1;
        }

        if self.all_bets_settled() {
            self.round_liability().clear();
            self.round_seed().clear();
            if self.free_bankroll() < self.min_bankroll().get() {
                self.is_paused().set(true);
            }
        }
    }

    #[endpoint(claim)]
    fn claim(&self) {
        let caller = self.blockchain().get_caller();
        let amount = self.claimable(&caller).take();
        require!(amount > 0, "nothing to claim");
        self.total_claimable().update(|t| *t -= &amount);
        self.tx().to(&caller).egld(&amount).transfer();
        self.claim_event(&caller, &amount);
    }

    /// Skim only rake + surplus above 2× min_bankroll. Bankroll stays self-funded.
    #[endpoint(skimToTreasury)]
    fn skim_to_treasury(&self) {
        require!(self.round_open().is_empty() || !self.round_open().get(), "round open");
        require!(self.round_seed().is_empty(), "resolve in progress");

        let buffer = self.min_bankroll().get() * SKIM_BUFFER_MULT;
        let free = self.free_bankroll();
        require!(free > buffer, "buffer not filled");

        let mut send_amount = self.rake_accrued().get();
        let extra = &free - &buffer;
        if extra > send_amount {
            send_amount = extra / 2u32 + send_amount;
            if send_amount > extra {
                send_amount = extra;
            }
        } else if send_amount > extra {
            send_amount = extra;
        }

        require!(send_amount > 0, "nothing to skim");

        let accrued = self.rake_accrued().get();
        if accrued <= send_amount {
            self.rake_accrued().clear();
        } else {
            self.rake_accrued().set(accrued - &send_amount);
        }

        let treasury = self.treasury().get();
        self.tx().to(&treasury).egld(&send_amount).transfer();
        self.skim_event(&treasury, &send_amount);
    }

    fn potential_payout(&self, amount: &BigUint, under: u64) -> BigUint {
        amount * ROLL_MOD / under * (BPS_DENOM - HOUSE_EDGE_BPS) / BPS_DENOM
    }

    fn roll_for(&self, seed: u64, index: u64) -> u64 {
        seed.wrapping_mul(1_000_003).wrapping_add(index) % ROLL_MOD
    }

    fn sc_egld_balance(&self) -> BigUint {
        self.blockchain()
            .get_sc_balance(&EgldOrEsdtTokenIdentifier::egld(), 0)
    }

    fn reserved(&self) -> BigUint {
        let mut r = self.total_claimable().get() + self.round_liability().get();
        r += self.rake_accrued().get();
        r
    }

    #[view(getFreeBankroll)]
    fn free_bankroll(&self) -> BigUint {
        let bal = self.sc_egld_balance();
        let res = self.reserved();
        if bal > res {
            bal - res
        } else {
            BigUint::zero()
        }
    }

    #[view(getDynamicMaxBet)]
    fn dynamic_max_bet(&self) -> BigUint {
        let from_bank = self.free_bankroll() * MAX_BET_FREE_BPS / BPS_DENOM;
        let cap = self.cap_max_bet().get();
        let min = self.min_bet().get();
        if from_bank < min {
            min
        } else if from_bank < cap {
            from_bank
        } else {
            cap
        }
    }

    fn all_bets_settled(&self) -> bool {
        let len = self.bets().len();
        for i in 1..=len {
            if !self.bets().get(i).settled {
                return false;
            }
        }
        true
    }

    fn has_unsettled_bets(&self) -> bool {
        !self.round_seed().is_empty() && !self.all_bets_settled()
    }

    fn require_not_paused(&self) {
        require!(!self.is_paused().get(), "paused");
    }

    #[view(getTreasury)]
    #[storage_mapper("treasury")]
    fn treasury(&self) -> SingleValueMapper<ManagedAddress>;

    #[view(getMinBet)]
    #[storage_mapper("minBet")]
    fn min_bet(&self) -> SingleValueMapper<BigUint>;

    #[view(getCapMaxBet)]
    #[storage_mapper("capMaxBet")]
    fn cap_max_bet(&self) -> SingleValueMapper<BigUint>;

    #[view(getRoundBlocks)]
    #[storage_mapper("roundBlocks")]
    fn round_blocks(&self) -> SingleValueMapper<u64>;

    #[view(getMinBankroll)]
    #[storage_mapper("minBankroll")]
    fn min_bankroll(&self) -> SingleValueMapper<BigUint>;

    #[view(isPaused)]
    #[storage_mapper("isPaused")]
    fn is_paused(&self) -> SingleValueMapper<bool>;

    #[view(getRoundId)]
    #[storage_mapper("roundId")]
    fn round_id(&self) -> SingleValueMapper<u64>;

    #[view(isRoundOpen)]
    #[storage_mapper("roundOpen")]
    fn round_open(&self) -> SingleValueMapper<bool>;

    #[view(getRoundEndBlock)]
    #[storage_mapper("roundEndBlock")]
    fn round_end_block(&self) -> SingleValueMapper<u64>;

    #[view(getRoundSeed)]
    #[storage_mapper("roundSeed")]
    fn round_seed(&self) -> SingleValueMapper<u64>;

    #[view(getRoundLiability)]
    #[storage_mapper("roundLiability")]
    fn round_liability(&self) -> SingleValueMapper<BigUint>;

    #[view(getRakeAccrued)]
    #[storage_mapper("rakeAccrued")]
    fn rake_accrued(&self) -> SingleValueMapper<BigUint>;

    #[storage_mapper("bets")]
    fn bets(&self) -> VecMapper<Bet<Self::Api>>;

    #[view(getClaimable)]
    #[storage_mapper("claimable")]
    fn claimable(&self, user: &ManagedAddress) -> SingleValueMapper<BigUint>;

    #[view(getTotalClaimable)]
    #[storage_mapper("totalClaimable")]
    fn total_claimable(&self) -> SingleValueMapper<BigUint>;

    #[view(getBetCount)]
    fn get_bet_count(&self) -> usize {
        self.bets().len()
    }

    #[event("roundStarted")]
    fn round_started_event(&self, #[indexed] round_id: u64, end_block: u64);

    #[event("betPlaced")]
    fn bet_placed_event(
        &self,
        #[indexed] round_id: u64,
        #[indexed] player: &ManagedAddress,
        amount: &BigUint,
        under: u64,
    );

    #[event("roundResolved")]
    fn round_resolved_event(&self, #[indexed] round_id: u64, seed: u64);

    #[event("claim")]
    fn claim_event(&self, #[indexed] player: &ManagedAddress, amount: &BigUint);

    #[event("skim")]
    fn skim_event(&self, #[indexed] treasury: &ManagedAddress, amount: &BigUint);

    #[event("bankrollFunded")]
    fn bankroll_funded_event(&self, amount: &BigUint);
}
