#![no_std]

use multiversx_sc::imports::*;

/// House edge 4.00%.
const HOUSE_EDGE_BPS: u64 = 400;
const BPS_DENOM: u64 = 10_000;
const MIN_UNDER: u64 = 2;
const MAX_UNDER: u64 = 96;
const ROLL_MOD: u64 = 100;
const MAX_BETS_PER_RESOLVE: usize = 50;

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
        max_bet: BigUint,
        round_blocks: u64,
        min_bankroll: BigUint,
    ) {
        require!(!treasury.is_zero(), "treasury required");
        require!(min_bet > 0, "min bet");
        require!(max_bet >= min_bet, "max < min");
        require!(round_blocks >= 2, "round too short");

        self.treasury().set(treasury);
        self.min_bet().set(min_bet);
        self.max_bet().set(max_bet);
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
        max_bet: BigUint,
        round_blocks: u64,
        min_bankroll: BigUint,
    ) {
        require!(self.round_open().is_empty() || !self.round_open().get(), "round open");
        require!(min_bet > 0 && max_bet >= min_bet, "bad bets");
        require!(round_blocks >= 2, "round too short");
        self.min_bet().set(min_bet);
        self.max_bet().set(max_bet);
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
        require!(
            self.bets_to_settle().is_empty(),
            "finish previous resolve"
        );

        let next_id = self.round_id().get() + 1;
        self.round_id().set(next_id);
        self.round_open().set(true);
        let end = self.blockchain().get_block_nonce() + self.round_blocks().get();
        self.round_end_block().set(end);
        self.round_liability().set(BigUint::zero());
        self.bets().clear();
        self.round_started_event(next_id, end);
    }

    /// Place a dice bet: win if future roll in [0, 99] is strictly < `under`.
    #[payable("EGLD")]
    #[endpoint(placeBet)]
    fn place_bet(&self, under: u64) {
        self.require_not_paused();
        require!(self.round_open().get(), "no open round");
        require!(
            self.blockchain().get_block_nonce() < self.round_end_block().get(),
            "betting closed"
        );
        require!(under >= MIN_UNDER && under <= MAX_UNDER, "under 2..96");

        let amount = self.call_value().egld().clone();
        require!(amount >= self.min_bet().get(), "below min");
        require!(amount <= self.max_bet().get(), "above max");

        let payout = self.potential_payout(&amount, under);
        let new_liability = self.round_liability().get() + &payout;
        let balance = self.blockchain().get_sc_balance(
            &EgldOrEsdtTokenIdentifier::egld(),
            0,
        );
        require!(
            balance >= &new_liability + &self.min_bankroll().get(),
            "insufficient bankroll"
        );

        self.round_liability().set(&new_liability);

        let player = self.blockchain().get_caller();
        self.bets().push(&Bet {
            player: player.clone(),
            amount: amount.clone(),
            under,
            settled: false,
            won: false,
            roll: 0,
        });

        self.bet_placed_event(self.round_id().get(), &player, &amount, under);
    }

    /// Close betting and start settling. Can be called by anyone after deadline.
    /// May need several calls if many bets (gas).
    #[endpoint(resolveRound)]
    fn resolve_round(&self) {
        require!(!self.round_open().is_empty() && self.round_open().get(), "no open round");
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
                self.claimable(&bet.player)
                    .update(|c| *c += payout);
            }
            processed += 1;
        }

        let all_done = self.all_bets_settled();
        if all_done {
            self.round_liability().clear();
            self.round_seed().clear();
            self.bets_to_settle_flag_clear();
        }
    }

    #[endpoint(claim)]
    fn claim(&self) {
        let caller = self.blockchain().get_caller();
        let amount = self.claimable(&caller).take();
        require!(amount > 0, "nothing to claim");
        self.tx().to(&caller).egld(&amount).transfer();
        self.claim_event(&caller, &amount);
    }

    /// Send surplus above min_bankroll + pending claims to PrideVault treasury.
    #[only_owner]
    #[endpoint(skimToTreasury)]
    fn skim_to_treasury(&self) {
        require!(self.round_open().is_empty() || !self.round_open().get(), "round open");
        require!(self.round_seed().is_empty(), "resolve in progress");

        let balance = self.blockchain().get_sc_balance(
            &EgldOrEsdtTokenIdentifier::egld(),
            0,
        );
        let reserved = self.min_bankroll().get() + self.total_claimable().get();
        require!(balance > reserved, "nothing to skim");
        let surplus = balance - reserved;
        let treasury = self.treasury().get();
        self.tx().to(&treasury).egld(&surplus).transfer();
        self.skim_event(&treasury, &surplus);
    }

    fn potential_payout(&self, amount: &BigUint, under: u64) -> BigUint {
        // payout = amount * 100 / under * (10000 - 400) / 10000
        amount * ROLL_MOD / under * (BPS_DENOM - HOUSE_EDGE_BPS) / BPS_DENOM
    }

    fn roll_for(&self, seed: u64, index: u64) -> u64 {
        let mut rng = RandomnessSource::new();
        // mix committed round seed + bet index; extra entropy from VM seed
        let extra = rng.next_u64();
        seed.wrapping_mul(1_000_003).wrapping_add(index).wrapping_add(extra) % ROLL_MOD
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

    fn bets_to_settle_flag_clear(&self) {
        // placeholder for readability; state already reflected by settled flags
    }

    fn bets_to_settle(&self) -> bool {
        if self.round_seed().is_empty() {
            return false;
        }
        !self.all_bets_settled()
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

    #[view(getMaxBet)]
    #[storage_mapper("maxBet")]
    fn max_bet(&self) -> SingleValueMapper<BigUint>;

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
