//! `NlheTurnSubgame` — `Game` impl for NLHE turn subgames (4-card board).
//!
//! Sibling to [`crate::NlheSubgame`] (the river `Game`). The turn tree has
//! a real chance layer — the river card — that makes vanilla enumerative
//! CFR prohibitive: each turn iteration would have to walk ~46 river
//! subgames (one per possible river card), each with thousands of info
//! sets and a 1326×1326 showdown matrix. So turn is the natural home for
//! MCCFR (External Sampling), which samples ONE river card per iteration.
//!
//! # Chance-layer handling (option b: encapsulated in NLHE)
//!
//! The [`solver_core::Game`] trait has no chance-node primitive. Instead
//! of extending the trait, we push the river-card chance above the root:
//!
//! * The **initial state** carries a `committed_river: Card` that was
//!   drawn outside the `Game` trait — typically by
//!   [`NlheTurnSubgame::sample_initial_state`] using an MCCfr-controlled
//!   PRNG.
//! * The tree inside `Game::apply` is chance-free: decisions propagate
//!   the same `committed_river` through every successor state.
//! * When turn betting closes (both players have acted on the turn and
//!   the last action is passive), the state is **terminal**. Utility is
//!   computed as the showdown on the 5-card board `turn_board ∪ {river}`.
//!
//! This encapsulates "chance = river card" inside the NLHE subgame
//! construction and keeps [`solver_core::MCCfr`] ignorant of chance
//! nodes. MCCfr drives the iteration with `run_with(|rng|
//! turn.sample_initial_state(rng))` to pull a fresh river sample per
//! iteration.
//!
//! # v0.1 simplification: no river betting
//!
//! When turn betting closes, the state is treated as terminal
//! (equivalent to "both players check the river") and utility is the
//! sign of the 5-card showdown for the committed combos, scaled by the
//! post-turn pot. This captures the turn strategy's own regret structure
//! faithfully — it's just a conservative choice about the river
//! continuation. A v0.2 extension will nest a full [`crate::NlheSubgame`]
//! (river) under each turn-terminal, solved live or looked up from cache.
//!
//! # Determinism
//!
//! All ingestion is deterministic given `committed_river`. The river
//! sample itself is pulled from the caller-supplied PRNG (MCCfr owns
//! the PRNG state), so seeding MCCfr with a fixed `u64` produces
//! bit-identical regret and strategy vectors across runs.

use rand::seq::SliceRandom;
use rand::Rng;

use solver_core::{Game, InfoSetId, Player};

use solver_eval::board::Board;
use solver_eval::card::Card;
use solver_eval::combo::{index_to_combo, NUM_COMBOS};
use solver_eval::eval::eval_7;
use solver_eval::hand::Hand;

use crate::action::{Action, ActionLog, Street};
use crate::bet_tree::BetTree;
use crate::range::Range;

/// An NLHE turn subgame.
///
/// A turn subgame is parameterized by a 4-card board, both players'
/// ranges, the pot and stack state entering the turn, a bet tree, and
/// who opens the turn. Iteration happens via MCCFR (External Sampling):
/// each iteration draws one river card from the remaining 48 cards
/// (excluding the 4 board cards — hero and villain holes are "dead"
/// only per-combo-pair, not universally) and treats that draw as the
/// chance-layer outcome for the whole iteration.
///
/// # Memory
///
/// Unlike the river subgame (which builds a 1326×1326 showdown matrix
/// at construction — 1.76 MB), the turn subgame does **not** pre-build
/// a per-river showdown matrix. Doing so would cost ~80 MB (46 rivers
/// × 1.76 MB), and only a fraction of those rivers are hit across the
/// 500–1000 MCCFR iterations a turn solve typically runs. We compute
/// showdown ranks on demand inside `utility` using [`eval_7`]. The
/// cost per showdown is two 7-card rank lookups, which is cheap
/// compared to the tree walk.
pub struct NlheTurnSubgame {
    turn_board: Board,
    hero_range: Range,
    villain_range: Range,
    pot_start: u32,
    stack_start: u32,
    first_to_act: Player,
    bet_tree: BetTree,

    /// The 46 cards still live in the deck, excluding the 4 board
    /// cards. Hero/villain hole cards are NOT excluded here because
    /// they vary per combo pair; we filter them per-pair inside
    /// `sample_initial_state`.
    deck_without_board: Vec<Card>,
}

impl NlheTurnSubgame {
    /// Build a new turn subgame.
    ///
    /// # Panics
    ///
    /// Panics if `turn_board.len != 4`. The turn, by definition, is the
    /// street after the flop where one community card has been revealed
    /// — so a turn board has exactly 4 cards. Flop/river go through
    /// different subgame types.
    pub fn new(
        turn_board: Board,
        hero_range: Range,
        villain_range: Range,
        pot_start: u32,
        stack_start: u32,
        first_to_act: Player,
        bet_tree: BetTree,
    ) -> Self {
        assert_eq!(
            turn_board.len, 4,
            "NlheTurnSubgame::new: turn has exactly 4 board cards, got {}",
            turn_board.len
        );

        let board_mask = board_card_mask(&turn_board);
        let mut deck_without_board = Vec::with_capacity(48);
        for c in 0..52u8 {
            if (board_mask >> c) & 1 == 0 {
                deck_without_board.push(Card(c));
            }
        }
        debug_assert_eq!(deck_without_board.len(), 48);

        Self {
            turn_board,
            hero_range,
            villain_range,
            pot_start,
            stack_start,
            first_to_act,
            bet_tree,
            deck_without_board,
        }
    }

    /// Borrow the turn board (4 cards).
    pub fn turn_board(&self) -> &Board {
        &self.turn_board
    }

    /// Borrow the bet tree.
    pub fn bet_tree(&self) -> &BetTree {
        &self.bet_tree
    }

    /// Borrow the hero range.
    pub fn hero_range(&self) -> &Range {
        &self.hero_range
    }

    /// Borrow the villain range.
    pub fn villain_range(&self) -> &Range {
        &self.villain_range
    }

    /// Sample a full initial state for one MCCFR iteration.
    ///
    /// Samples three independent things from `rng`:
    /// 1. One river card uniformly from the 46 non-{board, hero, villain}
    ///    cards. The river excludes the hero/villain combos too, since
    ///    the river has to be a live card that could have been dealt.
    /// 2. One hero combo weighted by `hero_range` (zero-weight combos
    ///    skipped, conflicting combos skipped).
    /// 3. One villain combo weighted by `villain_range` (with the same
    ///    restrictions, and additionally must not share a card with the
    ///    hero combo).
    ///
    /// The triple `(river, hero_combo, villain_combo)` is the chance
    /// outcome for this iteration; it stays fixed across the iteration's
    /// tree walks.
    ///
    /// Returns `None` if no valid combo pair survives — usually a sign
    /// of a misconfigured subgame (hero and villain ranges don't
    /// overlap the remaining deck after board exclusion, or the board
    /// is inconsistent with both ranges). Callers typically `expect`
    /// this.
    pub fn sample_initial_state<R: Rng + ?Sized>(&self, rng: &mut R) -> Option<TurnState> {
        let board_mask = board_card_mask(&self.turn_board);

        // Hero combos alive given only the board mask.
        let hero_live: Vec<(u16, f32)> = (0..NUM_COMBOS)
            .filter_map(|i| {
                let w = self.hero_range.weights[i];
                if w <= 0.0 {
                    return None;
                }
                let (a, b) = index_to_combo(i);
                if (board_mask & (card_mask(a) | card_mask(b))) != 0 {
                    return None;
                }
                Some((i as u16, w))
            })
            .collect();
        if hero_live.is_empty() {
            return None;
        }

        // Pre-cache villain-live combos (without hero exclusion); hero
        // exclusion is per-pair below.
        let villain_live: Vec<(u16, f32)> = (0..NUM_COMBOS)
            .filter_map(|i| {
                let w = self.villain_range.weights[i];
                if w <= 0.0 {
                    return None;
                }
                let (a, b) = index_to_combo(i);
                if (board_mask & (card_mask(a) | card_mask(b))) != 0 {
                    return None;
                }
                Some((i as u16, w))
            })
            .collect();
        if villain_live.is_empty() {
            return None;
        }

        // Sample hero combo weighted by range weight.
        let hero_combo = sample_weighted(&hero_live, rng)?;
        let (ha, hb) = index_to_combo(hero_combo as usize);
        let hero_mask = card_mask(ha) | card_mask(hb);

        // Sample villain combo: weighted by range, excluding combos
        // that share a card with hero.
        let villain_candidates: Vec<(u16, f32)> = villain_live
            .iter()
            .copied()
            .filter(|(vi, _)| {
                let (va, vb) = index_to_combo(*vi as usize);
                (hero_mask & (card_mask(va) | card_mask(vb))) == 0
            })
            .collect();
        if villain_candidates.is_empty() {
            return None;
        }
        let villain_combo = sample_weighted(&villain_candidates, rng)?;
        let (va, vb) = index_to_combo(villain_combo as usize);
        let combo_mask = hero_mask | card_mask(va) | card_mask(vb);

        // Sample river from {deck_without_board} \ {hero, villain}.
        let mut river_candidates: Vec<Card> = self
            .deck_without_board
            .iter()
            .copied()
            .filter(|c| (combo_mask & card_mask(*c)) == 0)
            .collect();
        // 52 - 4 (board) - 4 (2 hero + 2 villain) = 44, never empty for
        // any real subgame.
        if river_candidates.is_empty() {
            return None;
        }
        let river = *river_candidates.choose(rng)?;
        river_candidates.clear();

        Some(TurnState {
            hero_combo,
            villain_combo,
            committed_river: river,
            actions: ActionLog::new(),
        })
    }

    /// Returns the (hero_street_chips, villain_street_chips) committed
    /// on the turn for `state`.
    ///
    /// [`ActionLog::pot_contributions_on`] returns `(sb, bb)`. On
    /// postflop streets BB acts first under HU convention. We map that
    /// back to `(first_actor, second_actor)`, then onto (Hero, Villain)
    /// using `self.first_to_act`.
    fn street_contributions(&self, state: &TurnState) -> (u32, u32) {
        let (sb_slot, bb_slot) = state.actions.pot_contributions_on(Street::Turn);
        let (first_actor, second_actor) = (bb_slot, sb_slot);
        match self.first_to_act {
            Player::Hero => (first_actor, second_actor),
            Player::Villain => (second_actor, first_actor),
        }
    }

    /// Total chips each player has committed as of `state`, counting
    /// the implicit half-of-pot each player put in pre-turn.
    fn committed_totals(&self, state: &TurnState) -> (u32, u32) {
        let (hs, vs) = self.street_contributions(state);
        let half = self.pot_start / 2;
        let other_half = self.pot_start - half;
        (half + hs, other_half + vs)
    }

    /// Which player is acting now, given the turn-action count and who
    /// opened the turn.
    fn turn_current_player(&self, state: &TurnState) -> Player {
        let n = state.actions.iter_street(Street::Turn).count();
        if n % 2 == 0 {
            self.first_to_act
        } else {
            self.first_to_act.opponent()
        }
    }

    /// Legal actions at a non-terminal turn state. Mirrors
    /// `NlheSubgame::legal_river_actions` in spirit: Check + bet-tree
    /// sizings + AllIn when opening, Fold/Call/AllIn when facing a bet.
    fn legal_turn_actions(&self, state: &TurnState) -> Vec<Action> {
        let (hs, vs) = self.street_contributions(state);
        let current = self.turn_current_player(state);
        let my_street = match current {
            Player::Hero => hs,
            Player::Villain => vs,
        };
        let opp_street = match current {
            Player::Hero => vs,
            Player::Villain => hs,
        };
        let my_stack_remaining = self.stack_start.saturating_sub(my_street);
        let current_pot = self.pot_start + hs + vs;

        if opp_street > my_street {
            // Facing a bet/raise.
            let to_call = opp_street - my_street;
            let mut actions = vec![Action::Fold];
            if my_stack_remaining >= to_call {
                actions.push(Action::Call);
            }
            let last_was_allin_raise = matches!(
                state.actions.iter_street(Street::Turn).last(),
                Some(Action::AllIn) | Some(Action::Raise(_))
            );
            if !last_was_allin_raise && my_stack_remaining > to_call && my_stack_remaining > 0 {
                actions.push(Action::AllIn);
            }
            actions
        } else {
            // Opening or facing a check.
            let mut actions = vec![Action::Check];

            if my_stack_remaining == 0 {
                return actions;
            }

            for &fraction in self.bet_tree.sizings_for(Street::Turn) {
                if !fraction.is_finite() {
                    continue;
                }
                let chips = (current_pot as f32 * fraction).round() as u32;
                if chips == 0 {
                    continue;
                }
                let total_for_actor = my_street + chips;
                if total_for_actor >= self.stack_start {
                    continue;
                }
                actions.push(Action::Bet(total_for_actor));
            }

            actions.push(Action::AllIn);
            actions
        }
    }

    /// Evaluate the showdown utility for hero given a 5-card board
    /// assembled from the turn + committed river. Returns hero_utility.
    fn showdown_utility_for_hero(&self, state: &TurnState) -> f32 {
        let (h_lo, h_hi) = index_to_combo(state.hero_combo as usize);
        let (v_lo, v_hi) = index_to_combo(state.villain_combo as usize);
        let hero_hand = Hand::new(h_lo, h_hi);
        let villain_hand = Hand::new(v_lo, v_hi);
        let full_board = Board::river(
            self.turn_board.cards[0],
            self.turn_board.cards[1],
            self.turn_board.cards[2],
            self.turn_board.cards[3],
            state.committed_river,
        );
        let hr = eval_7(&hero_hand, &full_board);
        let vr = eval_7(&villain_hand, &full_board);
        let (hero_total, villain_total) = self.committed_totals(state);
        use std::cmp::Ordering;
        match hr.cmp(&vr) {
            Ordering::Greater => villain_total as f32,
            Ordering::Less => -(hero_total as f32),
            Ordering::Equal => 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Game-tree state for a turn subgame. Cheap to clone.
///
/// Unlike [`crate::subgame::SubgameState`], this additionally carries a
/// `committed_river` — the chance-layer outcome for this iteration. The
/// MCCFR driver samples a fresh `committed_river` per iteration (via
/// [`NlheTurnSubgame::sample_initial_state`]) and the whole tree walk
/// uses that one card.
#[derive(Clone, Debug)]
pub struct TurnState {
    /// Hero's combo index (0..1326). Visible to Hero's info sets only.
    pub hero_combo: u16,
    /// Villain's combo index (0..1326).
    pub villain_combo: u16,
    /// The sampled river card, fixed for the whole iteration. Terminal
    /// utility reads this to assemble a full 5-card board for the
    /// showdown evaluator.
    pub committed_river: Card,
    /// Actions taken on the turn. Pre-turn actions are folded into
    /// `pot_start` at construction time.
    pub actions: ActionLog,
}

// ---------------------------------------------------------------------------
// Game trait impl
// ---------------------------------------------------------------------------

impl Game for NlheTurnSubgame {
    type State = TurnState;
    type Action = Action;

    fn initial_state(&self) -> Self::State {
        // Without an MCCFR-driven sampling closure the "initial state"
        // is a deterministic fallback (combo 0/1, river = first non-
        // board card). All real driving code uses
        // `sample_initial_state(rng)` instead, which is what MCCfr's
        // `run_with` / `iterate_with` invoke.
        let river = self
            .deck_without_board
            .first()
            .copied()
            .expect("deck_without_board is empty — impossible");
        TurnState {
            hero_combo: 0,
            villain_combo: 1,
            committed_river: river,
            actions: ActionLog::new(),
        }
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        // (a) Someone folded on the turn.
        let has_fold = state
            .actions
            .iter_street(Street::Turn)
            .any(|a| matches!(a, Action::Fold));
        if has_fold {
            return true;
        }
        // (b) Turn betting has closed. In v0.1 that transitions directly
        //     to showdown — no river-street subtree is expanded.
        turn_is_closed(&state.actions)
    }

    fn utility(&self, state: &Self::State, player: Player) -> f32 {
        let (hero_total, villain_total) = self.committed_totals(state);

        let folder_on_turn = state
            .actions
            .iter_street(Street::Turn)
            .enumerate()
            .find_map(|(i, a)| if matches!(a, Action::Fold) { Some(i) } else { None });

        let hero_from_hero_pov: f32 = if let Some(fold_idx) = folder_on_turn {
            let folder = if fold_idx % 2 == 0 {
                self.first_to_act
            } else {
                self.first_to_act.opponent()
            };
            match folder {
                Player::Hero => -(hero_total as f32),
                Player::Villain => villain_total as f32,
            }
        } else {
            // Turn closed without a fold → showdown on the sampled
            // 5-card board.
            self.showdown_utility_for_hero(state)
        };

        match player {
            Player::Hero => hero_from_hero_pov,
            Player::Villain => -hero_from_hero_pov,
        }
    }

    fn current_player(&self, state: &Self::State) -> Player {
        debug_assert!(
            !self.is_terminal(state),
            "NlheTurnSubgame::current_player called on terminal state"
        );
        self.turn_current_player(state)
    }

    fn legal_actions(&self, state: &Self::State) -> Vec<Self::Action> {
        debug_assert!(
            !self.is_terminal(state),
            "legal_actions called on terminal state"
        );
        self.legal_turn_actions(state)
    }

    fn apply(&self, state: &Self::State, action: &Self::Action) -> Self::State {
        let mut next = state.clone();
        next.actions.push(Street::Turn, *action);
        next
    }

    fn info_set(&self, state: &Self::State, player: Player) -> InfoSetId {
        let combo = match player {
            Player::Hero => state.hero_combo,
            Player::Villain => state.villain_combo,
        };
        // Crucially, the `committed_river` is NOT part of the info set
        // — it's a chance outcome the acting player hasn't observed yet
        // (it's *next* street's card). Including it would leak
        // information the player doesn't have and silently make the
        // tree omniscient.
        InfoSetId(info_set_hash(combo, player, &state.actions))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// 52-bit mask with the bit for `card.0` set.
fn card_mask(card: Card) -> u64 {
    1u64 << card.0
}

/// Mask of all cards in `board`.
fn board_card_mask(board: &Board) -> u64 {
    let mut m = 0u64;
    for c in board.as_slice() {
        m |= card_mask(*c);
    }
    m
}

/// Deterministic FNV-1a style hash over `(combo, player, actions)`.
///
/// Identical construction to the one in `crate::subgame` — we duplicate
/// it rather than make it `pub(crate)` to keep the two files
/// independent of each other's implementation details. A2 owns this
/// file; A17 owns the river one. The hash function contract is:
/// "deterministic across runs, distinguishes combo + player + action
/// history". Both files uphold the contract independently.
fn info_set_hash(combo: u16, player: Player, actions: &ActionLog) -> u32 {
    const FNV_OFFSET: u32 = 0x811c9dc5;
    const FNV_PRIME: u32 = 0x01000193;

    // Tag to distinguish turn info sets from any accidentally-colliding
    // river info sets, should a caller ever mix subgame types against
    // the same MCCfr/CfrPlus state table. Cheap safety.
    const TURN_TAG: u32 = 0xA1;

    let mut h: u32 = FNV_OFFSET;
    h ^= TURN_TAG;
    h = h.wrapping_mul(FNV_PRIME);

    h ^= combo as u32 & 0xff;
    h = h.wrapping_mul(FNV_PRIME);
    h ^= (combo as u32 >> 8) & 0xff;
    h = h.wrapping_mul(FNV_PRIME);

    h ^= match player {
        Player::Hero => 0x11,
        Player::Villain => 0x22,
    };
    h = h.wrapping_mul(FNV_PRIME);

    for (street, action) in actions.iter() {
        h ^= street as u8 as u32;
        h = h.wrapping_mul(FNV_PRIME);
        let (tag, amt) = match action {
            Action::Fold => (1u32, 0u32),
            Action::Check => (2, 0),
            Action::Call => (3, 0),
            Action::Bet(x) => (4, x),
            Action::Raise(x) => (5, x),
            Action::AllIn => (6, 0),
        };
        h ^= tag;
        h = h.wrapping_mul(FNV_PRIME);
        for byte in amt.to_le_bytes() {
            h ^= byte as u32;
            h = h.wrapping_mul(FNV_PRIME);
        }
    }

    h
}

/// Turn-specific closure check — mirrors `river_is_closed` in
/// [`crate::subgame`] but scoped to [`Street::Turn`].
fn turn_is_closed(actions: &ActionLog) -> bool {
    let acts: Vec<Action> = actions.iter_street(Street::Turn).collect();
    if acts.is_empty() {
        return false;
    }
    if acts.iter().any(|a| matches!(a, Action::Fold)) {
        return true;
    }
    let last = *acts.last().unwrap();
    if matches!(last, Action::Bet(_) | Action::Raise(_)) {
        return false;
    }
    match last {
        Action::Check => acts.len() >= 2 && matches!(acts[acts.len() - 2], Action::Check),
        Action::Call => true,
        Action::AllIn => false, // AllIn re-opens until called/folded
        Action::Fold | Action::Bet(_) | Action::Raise(_) => {
            unreachable!("handled above")
        }
    }
}

/// Sample one element from a `(index, weight)` list, weighted by weight.
///
/// Returns `None` only if the list is empty or all weights are zero.
/// Uses a simple linear scan over the cumulative sum — fine because the
/// lists here are small (up to ~1326 entries, usually far fewer for a
/// real range).
fn sample_weighted<R: Rng + ?Sized>(items: &[(u16, f32)], rng: &mut R) -> Option<u16> {
    if items.is_empty() {
        return None;
    }
    let total: f32 = items.iter().map(|(_, w)| *w).sum();
    if total <= 0.0 {
        return None;
    }
    let r: f32 = rng.gen::<f32>() * total;
    let mut acc = 0.0f32;
    for &(i, w) in items {
        acc += w;
        if r < acc {
            return Some(i);
        }
    }
    // Rounding fallback — return the last element.
    items.last().map(|(i, _)| *i)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_xoshiro::Xoshiro256StarStar;

    fn sample_turn_board() -> Board {
        // Board: AhKh2sQc. Broadway-heavy; reasonable spot for tests.
        Board::parse("AhKh2sQc").unwrap()
    }

    #[test]
    fn construction_succeeds_for_turn() {
        let _sg = NlheTurnSubgame::new(
            sample_turn_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
    }

    #[test]
    #[should_panic(expected = "4 board cards")]
    fn wrong_board_length_panics() {
        let _ = NlheTurnSubgame::new(
            Board::parse("AhKh2s").unwrap(), // flop, len=3
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
    }

    #[test]
    fn sample_initial_state_is_deterministic() {
        let sg = NlheTurnSubgame::new(
            sample_turn_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let mut rng_a = Xoshiro256StarStar::seed_from_u64(42);
        let mut rng_b = Xoshiro256StarStar::seed_from_u64(42);
        let a = sg.sample_initial_state(&mut rng_a).unwrap();
        let b = sg.sample_initial_state(&mut rng_b).unwrap();
        assert_eq!(a.hero_combo, b.hero_combo);
        assert_eq!(a.villain_combo, b.villain_combo);
        assert_eq!(a.committed_river.0, b.committed_river.0);
    }

    #[test]
    fn sample_river_is_not_on_turn_board() {
        let sg = NlheTurnSubgame::new(
            sample_turn_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let board_cards: std::collections::HashSet<u8> =
            sg.turn_board.as_slice().iter().map(|c| c.0).collect();

        // 50 samples should hit several distinct river cards; none may
        // be on the turn board.
        let mut rng = Xoshiro256StarStar::seed_from_u64(7);
        for _ in 0..50 {
            let s = sg.sample_initial_state(&mut rng).unwrap();
            assert!(
                !board_cards.contains(&s.committed_river.0),
                "river card {} collides with turn board",
                s.committed_river.0
            );
        }
    }

    #[test]
    fn legal_actions_at_root() {
        let sg = NlheTurnSubgame::new(
            sample_turn_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let mut rng = Xoshiro256StarStar::seed_from_u64(0);
        let root = sg.sample_initial_state(&mut rng).unwrap();
        let actions = sg.legal_actions(&root);
        assert!(actions.contains(&Action::Check));
        assert!(actions.contains(&Action::AllIn));
        assert!(actions.iter().any(|a| matches!(a, Action::Bet(_))));
    }

    #[test]
    fn fold_is_terminal_and_pays_pot() {
        let sg = NlheTurnSubgame::new(
            sample_turn_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let mut rng = Xoshiro256StarStar::seed_from_u64(0);
        let mut state = sg.sample_initial_state(&mut rng).unwrap();
        state.actions.push(Street::Turn, Action::Bet(50));
        state.actions.push(Street::Turn, Action::Fold);
        assert!(sg.is_terminal(&state));
        // Villain (second actor; Hero opened) folded → hero wins villain's
        // half of pot_start = 50.
        let u = sg.utility(&state, Player::Hero);
        assert!((u - 50.0).abs() < 1e-4, "expected +50, got {u}");
    }

    #[test]
    fn check_check_goes_to_showdown() {
        // Build a turn spot where hero has a stronger hand than villain
        // on 3 of the 4 board cards PLUS most river outcomes.
        //
        // Turn board: AhKh2sQc. Hero = AdAs (top pair + trips with any
        // river A; but more importantly an overpair to K on an A-high
        // board). Villain = 3c3d (underpair).
        //
        // Expected: hero wins on most rivers. We test a specific river
        // where villain clearly loses — any non-3, non-King, non-Queen,
        // non-2 river.
        let sg = NlheTurnSubgame::new(
            sample_turn_board(),
            Range::parse("AA").unwrap(),
            Range::parse("33").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        // Pick specific combos to avoid any board conflicts:
        let hero_combo = solver_eval::combo::combo_index(
            Card::parse("Ac").unwrap(),
            Card::parse("Ad").unwrap(),
        ) as u16;
        let villain_combo = solver_eval::combo::combo_index(
            Card::parse("3c").unwrap(),
            Card::parse("3d").unwrap(),
        ) as u16;
        // River = 7s (blank, no help for anyone).
        let river = Card::parse("7s").unwrap();

        let state = TurnState {
            hero_combo,
            villain_combo,
            committed_river: river,
            actions: {
                let mut a = ActionLog::new();
                a.push(Street::Turn, Action::Check);
                a.push(Street::Turn, Action::Check);
                a
            },
        };
        assert!(sg.is_terminal(&state));
        // Hero has trips of aces (two aces + board ace), villain has
        // pair of 3s. Hero wins 50 (villain's half of pot_start).
        let u = sg.utility(&state, Player::Hero);
        assert!((u - 50.0).abs() < 1e-4, "expected +50, got {u}");
    }

    #[test]
    fn turn_is_closed_semantics() {
        let mut al = ActionLog::new();
        assert!(!turn_is_closed(&al));
        al.push(Street::Turn, Action::Check);
        assert!(!turn_is_closed(&al), "single check does not close");
        al.push(Street::Turn, Action::Check);
        assert!(turn_is_closed(&al), "check-check closes");

        let mut al = ActionLog::new();
        al.push(Street::Turn, Action::Bet(30));
        assert!(!turn_is_closed(&al));
        al.push(Street::Turn, Action::Call);
        assert!(turn_is_closed(&al));
    }

    #[test]
    fn info_set_excludes_river_card() {
        // An info set must not depend on `committed_river` — the acting
        // player hasn't seen the river yet at a turn decision node.
        let sg = NlheTurnSubgame::new(
            sample_turn_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let hero_combo = 5u16;
        let villain_combo = 7u16;
        let actions = ActionLog::new();
        let s_a = TurnState {
            hero_combo,
            villain_combo,
            committed_river: Card(10),
            actions: actions.clone(),
        };
        let s_b = TurnState {
            hero_combo,
            villain_combo,
            committed_river: Card(40), // different river
            actions,
        };
        assert_eq!(
            sg.info_set(&s_a, Player::Hero),
            sg.info_set(&s_b, Player::Hero),
            "info sets must not depend on committed_river"
        );
    }

    #[test]
    fn info_set_hashes_differ_for_players_and_combos() {
        let al = ActionLog::new();
        let h_a = info_set_hash(100, Player::Hero, &al);
        let h_b = info_set_hash(100, Player::Villain, &al);
        let h_c = info_set_hash(101, Player::Hero, &al);
        assert_ne!(h_a, h_b);
        assert_ne!(h_a, h_c);
    }

    #[test]
    fn sample_weighted_respects_weights() {
        let mut rng = Xoshiro256StarStar::seed_from_u64(0);
        let items = vec![(0u16, 0.9_f32), (1u16, 0.1_f32)];
        let mut counts = [0u32; 2];
        for _ in 0..10_000 {
            let i = sample_weighted(&items, &mut rng).unwrap();
            counts[i as usize] += 1;
        }
        let p0 = counts[0] as f32 / 10_000.0;
        assert!((p0 - 0.9).abs() < 0.02, "weighted sampler off: p0 = {p0}");
    }
}
