#![no_std]

use multiversx_sc::imports::*;
use multiversx_sc::derive_imports::*;

const HOUSE_EDGE_BPS: u64 = 500;
/// ROAR keeps the current 2% rake.
const RAKE_ROAR_BPS: u64 = 200;
/// EGLD rake is higher than ROAR.
const RAKE_EGLD_BPS: u64 = 400;
const BPS_DENOM: u64 = 10_000;
const MIN_UNDER: u64 = 2;
const MAX_UNDER: u64 = 96;
const ROLL_MOD: u64 = 100;
const MAX_BETS_PER_RESOLVE: usize = 50;
const MAX_BET_FREE_BPS: u64 = 200;
const SKIM_BUFFER_MULT: u32 = 2;

#[type_abi]
#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone)]
pub struct Bet<M: ManagedTypeApi> {
    pub player: ManagedAddress<M>,
    pub amount: BigUint<M>,
    pub under: u64,
    pub is_roar: bool,
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
        roar_token: TokenIdentifier,
        min_bet_egld: BigUint,
        cap_max_egld: BigUint,
        min_bet_roar: BigUint,
        cap_max_roar: BigUint,
        round_blocks: u64,
        min_bankroll_egld: BigUint,
        min_bankroll_roar: BigUint,
    ) {
        require!(!treasury.is_zero(), "treasury required");
        require!(roar_token.is_valid_esdt_identifier(), "bad ROAR token");
        require!(min_bet_egld > 0 && cap_max_egld >= min_bet_egld, "bad EGLD bets");
        require!(min_bet_roar > 0 && cap_max_roar >= min_bet_roar, "bad ROAR bets");
        require!(round_blocks >= 2, "round too short");
        require!(min_bankroll_egld > 0 && min_bankroll_roar > 0, "min bankroll");

        self.treasury().set(treasury);
        self.roar_token().set(roar_token);
        self.min_bet_egld().set(min_bet_egld);
        self.cap_max_egld().set(cap_max_egld);
        self.min_bet_roar().set(min_bet_roar);
        self.cap_max_roar().set(cap_max_roar);
        self.round_blocks().set(round_blocks);
        self.min_bankroll_egld().set(min_bankroll_egld);
        self.min_bankroll_roar().set(min_bankroll_roar);
        self.is_paused().set(false);
        self.round_id().set(0u64);
    }

    #[upgrade]
    fn upgrade(&self) {}

    #[only_owner]
    #[payable]
    #[endpoint(fundBankroll)]
    fn fund_bankroll(&self) {
        let payment = self.call_value().egld_or_single_esdt();
        require!(payment.amount > 0, "zero");
        if !payment.token_identifier.is_egld() {
            require!(
                payment.token_identifier == self.roar_id_or(),
                "token not accepted"
            );
        }
        self.bankroll_funded_event(&payment.amount);
    }

    #[only_owner]
    #[endpoint(setRoarToken)]
    fn set_roar_token(&self, roar_token: TokenIdentifier) {
        require!(roar_token.is_valid_esdt_identifier(), "bad ROAR token");
        require!(
            self.total_claimable_roar().get() == 0,
            "ROAR claims pending"
        );
        self.roar_token().set(roar_token);
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

    #[endpoint(startRound)]
    fn start_round(&self) {
        self.require_not_paused();
        require!(self.round_open().is_empty() || !self.round_open().get(), "already open");
        require!(!self.has_unsettled_bets(), "finish previous resolve");
        require!(
            self.free_bankroll_egld() >= self.min_bankroll_egld().get()
                || self.free_bankroll_roar() >= self.min_bankroll_roar().get(),
            "bankroll thin"
        );

        let next_id = self.round_id().get() + 1;
        self.round_id().set(next_id);
        self.round_open().set(true);
        let end = self.blockchain().get_block_nonce() + self.round_blocks().get();
        self.round_end_block().set(end);
        self.round_liability_egld().set(BigUint::zero());
        self.round_liability_roar().set(BigUint::zero());
        self.round_seed().clear();
        self.bets().clear();
        self.round_started_event(next_id, end);
    }

    /// Pay EGLD or ROAR. Rake: EGLD 4%, ROAR 2%.
    #[payable]
    #[endpoint(placeBet)]
    fn place_bet(&self, under: u64) {
        self.require_not_paused();
        require!(!self.round_open().is_empty() && self.round_open().get(), "no open round");
        require!(
            self.blockchain().get_block_nonce() < self.round_end_block().get(),
            "betting closed"
        );
        require!(under >= MIN_UNDER && under <= MAX_UNDER, "under 2..96");
        require!(self.bets().len() < MAX_BETS_PER_RESOLVE, "round full");

        let payment = self.call_value().egld_or_single_esdt();
        let is_roar = !payment.token_identifier.is_egld();
        if is_roar {
            require!(payment.token_identifier == self.roar_id_or(), "token not accepted");
            require!(payment.token_nonce == 0, "ROAR must be fungible");
        }

        let amount = payment.amount;
        let min = if is_roar { self.min_bet_roar().get() } else { self.min_bet_egld().get() };
        let cap = if is_roar { self.cap_max_roar().get() } else { self.cap_max_egld().get() };
        require!(amount >= min, "below min");
        require!(amount <= cap, "above cap");
        require!(amount <= self.dynamic_max_bet(is_roar), "above dynamic max");

        let rake_bps = if is_roar { RAKE_ROAR_BPS } else { RAKE_EGLD_BPS };
        let rake = &amount * rake_bps / BPS_DENOM;
        let net = &amount - &rake;
        let payout = self.potential_payout(&net, under);

        if is_roar {
            self.round_liability_roar().update(|l| *l += &payout);
            require!(self.free_bankroll_roar() >= self.min_bankroll_roar().get(), "ROAR bankroll thin");
            if rake > 0 {
                self.rake_accrued_roar().update(|r| *r += rake);
            }
        } else {
            self.round_liability_egld().update(|l| *l += &payout);
            require!(self.free_bankroll_egld() >= self.min_bankroll_egld().get(), "EGLD bankroll thin");
            if rake > 0 {
                self.rake_accrued_egld().update(|r| *r += rake);
            }
        }

        let player = self.blockchain().get_caller();
        self.bets().push(&Bet {
            player: player.clone(),
            amount: net,
            under,
            is_roar,
            settled: false,
            won: false,
            roll: 0,
        });
        self.bet_placed_event(self.round_id().get(), &player, under, &amount);
    }

    #[endpoint(resolveRound)]
    fn resolve_round(&self) {
        let open = !self.round_open().is_empty() && self.round_open().get();
        let resolving = !self.round_seed().is_empty();
        require!(open || resolving, "nothing to resolve");
        require!(
            self.blockchain().get_block_nonce() > self.round_end_block().get(),
            "too early"
        );

        if self.round_seed().is_empty() {
            let mut rng = RandomnessSource::new();
            let seed = rng.next_u64() ^ self.round_id().get();
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
            bet.roll = roll;
            bet.won = roll < bet.under;
            bet.settled = true;
            self.bets().set(i, &bet);
            if bet.won {
                let payout = self.potential_payout(&bet.amount, bet.under);
                if bet.is_roar {
                    self.claimable_roar(&bet.player).update(|c| *c += &payout);
                    self.total_claimable_roar().update(|t| *t += payout);
                } else {
                    self.claimable_egld(&bet.player).update(|c| *c += &payout);
                    self.total_claimable_egld().update(|t| *t += payout);
                }
            }
            processed += 1;
        }

        if self.all_bets_settled() {
            self.round_liability_egld().clear();
            self.round_liability_roar().clear();
            self.round_seed().clear();
            if self.free_bankroll_egld() < self.min_bankroll_egld().get()
                && self.free_bankroll_roar() < self.min_bankroll_roar().get()
            {
                self.is_paused().set(true);
            }
        }
    }

    #[endpoint(claim)]
    fn claim(&self) {
        let caller = self.blockchain().get_caller();
        let egld_amt = self.claimable_egld(&caller).take();
        let roar_amt = self.claimable_roar(&caller).take();
        require!(egld_amt > 0 || roar_amt > 0, "nothing to claim");
        if egld_amt > 0 {
            self.total_claimable_egld().update(|t| *t -= &egld_amt);
            self.tx().to(&caller).egld(&egld_amt).transfer();
        }
        if roar_amt > 0 {
            self.total_claimable_roar().update(|t| *t -= &roar_amt);
            let token = self.roar_token().get();
            self.tx()
                .to(&caller)
                .single_esdt(&token, 0, &roar_amt)
                .transfer();
        }
        self.claim_event(&caller, &egld_amt);
    }

    #[endpoint(skimToTreasury)]
    fn skim_to_treasury(&self) {
        require!(self.round_open().is_empty() || !self.round_open().get(), "round open");
        require!(self.round_seed().is_empty(), "resolve in progress");
        let treasury = self.treasury().get();
        self.skim_one(false, &treasury);
        self.skim_one(true, &treasury);
    }

    fn skim_one(&self, is_roar: bool, treasury: &ManagedAddress) {
        let buffer = if is_roar {
            self.min_bankroll_roar().get() * SKIM_BUFFER_MULT
        } else {
            self.min_bankroll_egld().get() * SKIM_BUFFER_MULT
        };
        let free = if is_roar {
            self.free_bankroll_roar()
        } else {
            self.free_bankroll_egld()
        };
        if free <= buffer {
            return;
        }
        let accrued = if is_roar {
            self.rake_accrued_roar().get()
        } else {
            self.rake_accrued_egld().get()
        };
        let extra = &free - &buffer;
        let mut send_amount = accrued.clone();
        if extra > send_amount {
            send_amount = extra.clone() / 2u32 + send_amount;
            if send_amount > extra {
                send_amount = extra;
            }
        } else if send_amount > extra {
            send_amount = extra;
        }
        if send_amount == 0 {
            return;
        }
        if accrued <= send_amount {
            if is_roar {
                self.rake_accrued_roar().clear();
            } else {
                self.rake_accrued_egld().clear();
            }
        } else if is_roar {
            self.rake_accrued_roar().set(accrued - &send_amount);
        } else {
            self.rake_accrued_egld().set(accrued - &send_amount);
        }
        if is_roar {
            let token = self.roar_token().get();
            self.tx()
                .to(treasury)
                .single_esdt(&token, 0, &send_amount)
                .transfer();
        } else {
            self.tx().to(treasury).egld(&send_amount).transfer();
        }
        self.skim_event(treasury, &send_amount);
    }

    fn potential_payout(&self, amount: &BigUint, under: u64) -> BigUint {
        amount * ROLL_MOD / under * (BPS_DENOM - HOUSE_EDGE_BPS) / BPS_DENOM
    }

    fn roll_for(&self, seed: u64, index: u64) -> u64 {
        seed.wrapping_mul(1_000_003).wrapping_add(index) % ROLL_MOD
    }

    fn roar_id_or(&self) -> EgldOrEsdtTokenIdentifier {
        EgldOrEsdtTokenIdentifier::esdt(self.roar_token().get())
    }

    fn token_balance(&self, is_roar: bool) -> BigUint {
        if is_roar {
            self.blockchain()
                .get_sc_balance(&self.roar_id_or(), 0)
        } else {
            self.blockchain()
                .get_sc_balance(&EgldOrEsdtTokenIdentifier::egld(), 0)
        }
    }

    fn reserved(&self, is_roar: bool) -> BigUint {
        if is_roar {
            self.total_claimable_roar().get()
                + self.round_liability_roar().get()
                + self.rake_accrued_roar().get()
        } else {
            self.total_claimable_egld().get()
                + self.round_liability_egld().get()
                + self.rake_accrued_egld().get()
        }
    }

    #[view(getFreeBankrollEgld)]
    fn free_bankroll_egld(&self) -> BigUint {
        let bal = self.token_balance(false);
        let res = self.reserved(false);
        if bal > res { bal - res } else { BigUint::zero() }
    }

    #[view(getFreeBankrollRoar)]
    fn free_bankroll_roar(&self) -> BigUint {
        let bal = self.token_balance(true);
        let res = self.reserved(true);
        if bal > res { bal - res } else { BigUint::zero() }
    }

    #[view(getDynamicMaxBetEgld)]
    fn dynamic_max_bet_egld(&self) -> BigUint {
        self.dynamic_max_bet(false)
    }

    #[view(getDynamicMaxBetRoar)]
    fn dynamic_max_bet_roar(&self) -> BigUint {
        self.dynamic_max_bet(true)
    }

    fn dynamic_max_bet(&self, is_roar: bool) -> BigUint {
        let free = if is_roar { self.free_bankroll_roar() } else { self.free_bankroll_egld() };
        let from_bank = free * MAX_BET_FREE_BPS / BPS_DENOM;
        let cap = if is_roar { self.cap_max_roar().get() } else { self.cap_max_egld().get() };
        let min = if is_roar { self.min_bet_roar().get() } else { self.min_bet_egld().get() };
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

    #[view(getRoarToken)]
    #[storage_mapper("roarToken")]
    fn roar_token(&self) -> SingleValueMapper<TokenIdentifier>;

    #[view(getMinBetEgld)]
    #[storage_mapper("minBetEgld")]
    fn min_bet_egld(&self) -> SingleValueMapper<BigUint>;

    #[view(getCapMaxEgld)]
    #[storage_mapper("capMaxEgld")]
    fn cap_max_egld(&self) -> SingleValueMapper<BigUint>;

    #[view(getMinBetRoar)]
    #[storage_mapper("minBetRoar")]
    fn min_bet_roar(&self) -> SingleValueMapper<BigUint>;

    #[view(getCapMaxRoar)]
    #[storage_mapper("capMaxRoar")]
    fn cap_max_roar(&self) -> SingleValueMapper<BigUint>;

    #[view(getRoundBlocks)]
    #[storage_mapper("roundBlocks")]
    fn round_blocks(&self) -> SingleValueMapper<u64>;

    #[view(getMinBankrollEgld)]
    #[storage_mapper("minBankrollEgld")]
    fn min_bankroll_egld(&self) -> SingleValueMapper<BigUint>;

    #[view(getMinBankrollRoar)]
    #[storage_mapper("minBankrollRoar")]
    fn min_bankroll_roar(&self) -> SingleValueMapper<BigUint>;

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

    #[storage_mapper("roundLiabilityEgld")]
    fn round_liability_egld(&self) -> SingleValueMapper<BigUint>;

    #[storage_mapper("roundLiabilityRoar")]
    fn round_liability_roar(&self) -> SingleValueMapper<BigUint>;

    #[view(getRakeAccruedEgld)]
    #[storage_mapper("rakeAccruedEgld")]
    fn rake_accrued_egld(&self) -> SingleValueMapper<BigUint>;

    #[view(getRakeAccruedRoar)]
    #[storage_mapper("rakeAccruedRoar")]
    fn rake_accrued_roar(&self) -> SingleValueMapper<BigUint>;

    #[storage_mapper("bets")]
    fn bets(&self) -> VecMapper<Bet<Self::Api>>;

    #[view(getClaimableEgld)]
    #[storage_mapper("claimableEgld")]
    fn claimable_egld(&self, user: &ManagedAddress) -> SingleValueMapper<BigUint>;

    #[view(getClaimableRoar)]
    #[storage_mapper("claimableRoar")]
    fn claimable_roar(&self, user: &ManagedAddress) -> SingleValueMapper<BigUint>;

    #[storage_mapper("totalClaimableEgld")]
    fn total_claimable_egld(&self) -> SingleValueMapper<BigUint>;

    #[storage_mapper("totalClaimableRoar")]
    fn total_claimable_roar(&self) -> SingleValueMapper<BigUint>;

    #[view(getBetCount)]
    fn get_bet_count(&self) -> usize {
        self.bets().len()
    }

    #[view(getRakeBps)]
    fn get_rake_bps(&self) -> MultiValue2<u64, u64> {
        (RAKE_EGLD_BPS, RAKE_ROAR_BPS).into()
    }

    #[event("roundStarted")]
    fn round_started_event(&self, #[indexed] round_id: u64, end_block: u64);

    #[event("betPlaced")]
    fn bet_placed_event(
        &self,
        #[indexed] round_id: u64,
        #[indexed] player: &ManagedAddress,
        #[indexed] under: u64,
        amount: &BigUint,
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
