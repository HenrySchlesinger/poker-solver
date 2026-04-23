//! `NlheSubgame` — implements `solver_core::Game` for NLHE river subgames.
//!
//! This is the bridge between the game-agnostic CFR algorithm and the
//! specific rules of No-Limit Hold'em. For v0.1 we target **river only**
//! (turn/flop are Day 4+). A river subgame is built from:
//!
//! - A 5-card board (complete; no chance nodes on the river).
//! - Hero and Villain ranges (1326-wide weight vectors).
//! - `pot_start`: chips already in the pot at the start of the river.
//! - `stack_start`: effective stack each player has entering the river.
//! - `first_to_act`: which player opens the river.
//! - A [`BetTree`] constraining allowed bet sizes.
//!
//! # The chance layer
//!
//! The `Game` trait has no chance primitive. Like the Kuhn fixture, we
//! handle the chance layer (pairs of `(hero_combo, villain_combo)` drawn
//! from the two ranges) outside the trait: the caller hands the solver
//! every non-zero-weight pair as a root with weight equal to the product
//! of the two range weights, normalized so the roots sum to 1. This is
//! the "Vector CFR folded into a chance layer" formulation the brief
//! suggests for v0.1 correctness. Day 3 replaces the per-pair loop with
//! a SIMD matrix op.
//!
//! # Info-set identity
//!
//! An info set is keyed by `(acting_player_combo, river_action_history)`.
//! The opposing player's combo is NOT in the key (they don't see it).
//! We pack the combo and a deterministic hash of the action history
//! into a `u32` for the `InfoSetId`. The hash is FNV-1a style — NOT
//! `DefaultHasher`, which is non-deterministic across runs.
//!
//! # Legal action discretization (v0.1)
//!
//! - **No aggression yet on river:** [`Action::Check`], plus
//!   [`Action::Bet(amount)`] per bet-tree pot-fraction, plus
//!   [`Action::AllIn`]. Bet amounts that exceed the remaining stack are
//!   clamped up to [`Action::AllIn`].
//! - **Facing a bet or raise:** [`Action::Fold`], [`Action::Call`], and
//!   — if a raise budget remains — [`Action::AllIn`]. In v0.1 the only
//!   supported raise is all-in (keeps the tree tractable while still
//!   letting CFR discover a raising strategy).
//! - **After a raise (already all-in one side):** [`Action::Fold`] /
//!   [`Action::Call`]. No further re-raises.
//!
//! The bet tree's `INF` sizing maps to [`Action::AllIn`].

use solver_core::{Game, InfoSetId, Player};

use solver_eval::board::Board;
use solver_eval::card::Card;
use solver_eval::combo::{index_to_combo, NUM_COMBOS};
use solver_eval::eval::eval_7;
use solver_eval::hand::Hand;

use crate::action::{Action, ActionLog, Street};
use crate::bet_tree::BetTree;
use crate::range::Range;

/// An NLHE river subgame.
///
/// Construction is expensive — it builds a `NUM_COMBOS × NUM_COMBOS`
/// showdown sign matrix eagerly so every terminal lookup is a single
/// indirection. Cloning the subgame clones the matrix, which is
/// `~1.76 MB` (1326² bytes). Intended to be built once and passed by
/// reference to a single [`solver_core::CfrPlus`] solver.
///
/// The type is `#[allow(missing_docs)]` on private helpers only; all
/// public surface is documented because the crate enforces
/// `#![warn(missing_docs)]`.
pub struct NlheSubgame {
    board: Board,
    hero_range: Range,
    villain_range: Range,
    pot_start: u32,
    stack_start: u32,
    first_to_act: Player,
    bet_tree: BetTree,

    /// `showdown_sign[hero_combo][villain_combo]`:
    /// `+1` = hero beats villain, `-1` = villain beats hero, `0` = tie
    /// or dead-card conflict (combo shares a card with board or the
    /// other combo — treated as "no showdown"; CFR walks that skip
    /// those pairs at the chance layer).
    showdown_sign: Box<ShowdownMatrix>,
}

/// Packed `1326 × 1326` showdown sign matrix, kept behind a `Box`
/// because it's 1.76 MB and we don't want to live on the stack.
type ShowdownMatrix = [[i8; NUM_COMBOS]; NUM_COMBOS];

impl NlheSubgame {
    /// Build a new river subgame.
    ///
    /// # Panics
    ///
    /// Panics if `board.len != 5` — this type only handles complete
    /// river boards. Turn/flop subgames are a Day 4+ feature.
    pub fn new(
        board: Board,
        hero_range: Range,
        villain_range: Range,
        pot_start: u32,
        stack_start: u32,
        first_to_act: Player,
        bet_tree: BetTree,
    ) -> Self {
        assert_eq!(
            board.len, 5,
            "NlheSubgame::new: v0.1 handles river only (need 5 board cards, got {})",
            board.len
        );
        let showdown_sign = build_showdown_matrix(&board);
        Self {
            board,
            hero_range,
            villain_range,
            pot_start,
            stack_start,
            first_to_act,
            bet_tree,
            showdown_sign,
        }
    }

    /// Enumerate `(hero_combo, villain_combo, weight)` triples for every
    /// pair that:
    /// 1. Has a non-zero weight in the hero *and* villain range,
    /// 2. Does not conflict with the board, and
    /// 3. Does not share a card between hero and villain.
    ///
    /// Returned weights are **unnormalized** — the product of the two
    /// range weights. The caller is responsible for normalizing
    /// (e.g., dividing by the sum) before passing to
    /// [`solver_core::CfrPlus::iterate_from`], which treats its weights
    /// as chance-layer priors.
    pub fn enumerate_combo_pairs(&self) -> Vec<(u16, u16, f32)> {
        let mut out = Vec::new();
        let board_mask = board_card_mask(&self.board);

        for h in 0..NUM_COMBOS {
            let hw = self.hero_range.weights[h];
            if hw <= 0.0 {
                continue;
            }
            let (h_lo, h_hi) = index_to_combo(h);
            if card_conflict(board_mask, &[h_lo, h_hi]) {
                continue;
            }
            let hero_mask = card_mask(h_lo) | card_mask(h_hi);

            for v in 0..NUM_COMBOS {
                let vw = self.villain_range.weights[v];
                if vw <= 0.0 {
                    continue;
                }
                let (v_lo, v_hi) = index_to_combo(v);
                if card_conflict(board_mask, &[v_lo, v_hi]) {
                    continue;
                }
                if card_conflict(hero_mask, &[v_lo, v_hi]) {
                    continue;
                }

                out.push((h as u16, v as u16, hw * vw));
            }
        }

        out
    }

    /// Build the initial `(state, weight)` roots for every non-conflicting
    /// combo pair, with weights normalized to sum to 1.
    ///
    /// This is the vector the caller passes to
    /// [`solver_core::CfrPlus::iterate_from`] / [`solver_core::CfrPlus::run_from`].
    /// Under the v0.1 pair-enumeration formulation, each "root" is a
    /// state representing a specific `(hero_combo, villain_combo)` deal,
    /// and the chance-layer weight is the (normalized) product of the
    /// two range weights.
    pub fn chance_roots(&self) -> Vec<(SubgameState, f32)> {
        let triples = self.enumerate_combo_pairs();
        let total: f32 = triples.iter().map(|(_, _, w)| *w).sum();
        if total <= 0.0 {
            return Vec::new();
        }
        let inv = 1.0 / total;
        triples
            .into_iter()
            .map(|(h, v, w)| {
                (
                    SubgameState {
                        hero_combo: h,
                        villain_combo: v,
                        actions: ActionLog::new(),
                    },
                    w * inv,
                )
            })
            .collect()
    }

    /// Borrow the bet tree (used by tests and by the CFR driver to
    /// introspect sizings).
    pub fn bet_tree(&self) -> &BetTree {
        &self.bet_tree
    }

    /// Returns the (hero_street_chips, villain_street_chips) committed
    /// so far on the river for `state`.
    ///
    /// [`ActionLog::pot_contributions_on`] returns `(sb, bb)`. On
    /// postflop streets its convention is "BB (second element) acts
    /// first" (per HU rules), so we swap into `(first_actor,
    /// second_actor)` here, then re-map onto (Hero, Villain) using
    /// [`Self::first_to_act`].
    fn street_contributions(&self, state: &SubgameState) -> (u32, u32) {
        let (sb_slot, bb_slot) = state.actions.pot_contributions_on(Street::River);
        // On river, BB opens; second actor is SB. So first_actor = bb_slot,
        // second_actor = sb_slot.
        let (first_actor, second_actor) = (bb_slot, sb_slot);
        match self.first_to_act {
            Player::Hero => (first_actor, second_actor),
            Player::Villain => (second_actor, first_actor),
        }
    }

    /// (hero_total_committed, villain_total_committed) — including the
    /// implicit half-of-`pot_start` each player contributed before the
    /// river started. We model `pot_start` as pre-split (both players
    /// contributed `pot_start / 2`), which is exact in heads-up play
    /// that reached the river through a balanced call.
    fn committed_totals(&self, state: &SubgameState) -> (u32, u32) {
        let (hs, vs) = self.street_contributions(state);
        let half = self.pot_start / 2;
        let other_half = self.pot_start - half; // covers odd pot_start
        (half + hs, other_half + vs)
    }

    /// Current player to act, given the river action count and who
    /// opened the river.
    fn river_current_player(&self, state: &SubgameState) -> Player {
        let n = state.actions.iter_street(Street::River).count();
        if n % 2 == 0 {
            self.first_to_act
        } else {
            self.first_to_act.opponent()
        }
    }

    /// Produce legal actions at a non-terminal state.
    ///
    /// See the module docs for the v0.1 discretization. Returned vector
    /// is non-empty at every non-terminal state — the solver's walk
    /// asserts this.
    fn legal_river_actions(&self, state: &SubgameState) -> Vec<Action> {
        // If either player is all-in and the last action was not a
        // call, the opponent's only "decision" is to call — but under
        // the is_terminal + is_street_closed contract, that case is a
        // terminal (showdown) once the Call lands. This method is only
        // called on non-terminals.
        let (hs, vs) = self.street_contributions(state);
        let current = self.river_current_player(state);
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
            // Facing a bet/raise — respond.
            let to_call = opp_street - my_street;
            let mut actions = vec![Action::Fold];
            // Call: can always "call" (even for less, which goes all-in).
            // For v0.1 simplicity, Call is only legal if we can match
            // the full bet; otherwise only AllIn (for less) is an
            // option. Since stack_start is the same for both, and the
            // aggressor can't have bet more than stack_start, this is
            // typically OK.
            if my_stack_remaining >= to_call {
                actions.push(Action::Call);
            }
            // Raise shapes in v0.1: AllIn only, to keep the tree
            // tractable. Only legal if we can actually raise (not
            // already committed to shove, and we have more than the
            // call amount left).
            let last_was_allin_raise = matches!(
                state.actions.iter_street(Street::River).last(),
                Some(Action::AllIn) | Some(Action::Raise(_))
            );
            if !last_was_allin_raise && my_stack_remaining > to_call && my_stack_remaining > 0 {
                actions.push(Action::AllIn);
            }
            actions
        } else {
            // Opening or facing a check — we're not behind in chips.
            let mut actions = vec![Action::Check];

            if my_stack_remaining == 0 {
                // Already all-in but nothing to respond to — can only
                // check (trivially). Terminal on the next check.
                return actions;
            }

            for &fraction in self.bet_tree.sizings_for(Street::River) {
                if !fraction.is_finite() {
                    // INF sizing → AllIn, handled below.
                    continue;
                }
                let chips = (current_pot as f32 * fraction).round() as u32;
                // Minimum bet = 1 chip (anything less is meaningless).
                if chips == 0 {
                    continue;
                }
                let total_for_actor = my_street + chips;
                if total_for_actor >= self.stack_start {
                    // Bet exceeds stack → emit as AllIn instead (below).
                    continue;
                }
                actions.push(Action::Bet(total_for_actor));
            }

            // Always allow an explicit all-in if we have chips left.
            actions.push(Action::AllIn);

            actions
        }
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Game-tree state. Cheap to clone — CFR's walk clones this at every
/// branch. Heavy structures (board, ranges, showdown matrix) live on
/// [`NlheSubgame`] and are borrowed immutably during the walk.
#[derive(Clone)]
pub struct SubgameState {
    /// Hero's combo index (0..1326). Visible to Hero's info sets only.
    pub hero_combo: u16,
    /// Villain's combo index (0..1326). Visible to Villain's info sets only.
    pub villain_combo: u16,
    /// Actions on the river. Pre-river actions are folded into
    /// `pot_start` at construction time, so we don't store them.
    pub actions: ActionLog,
}

// ---------------------------------------------------------------------------
// Game trait impl
// ---------------------------------------------------------------------------

impl Game for NlheSubgame {
    type State = SubgameState;
    type Action = Action;

    fn initial_state(&self) -> Self::State {
        // Without a chance layer, the "initial state" is a deterministic
        // default — pick combo 0 for each side. All real driving code
        // uses `chance_roots()` + `iterate_from(...)` instead.
        SubgameState {
            hero_combo: 0,
            villain_combo: 1,
            actions: ActionLog::new(),
        }
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        // Terminal shapes:
        //   (a) Last river action is Fold.
        //   (b) River street has closed (both players have acted and
        //       the last action is a passive one).
        //   (c) Both players are all-in and there's been a matching
        //       response (closed street).
        let has_fold = state
            .actions
            .iter_street(Street::River)
            .any(|a| matches!(a, Action::Fold));
        if has_fold {
            return true;
        }
        // River-street-specific closure check. Our ActionLog helper
        // works for any street; we want only river closure here.
        river_is_closed(&state.actions)
    }

    fn utility(&self, state: &Self::State, player: Player) -> f32 {
        let (hero_total, villain_total) = self.committed_totals(state);

        // Fold terminal: non-folder wins, gains opponent's total commitment.
        let folder_on_river = state
            .actions
            .iter_street(Street::River)
            .enumerate()
            .find_map(|(i, a)| {
                if matches!(a, Action::Fold) {
                    Some(i)
                } else {
                    None
                }
            });

        let hero_from_hero_pov: f32;
        if let Some(fold_idx) = folder_on_river {
            // Who folded? The n-th voluntary actor on river is:
            //   first_to_act if n is even, opponent if n is odd.
            let folder = if fold_idx % 2 == 0 {
                self.first_to_act
            } else {
                self.first_to_act.opponent()
            };
            hero_from_hero_pov = match folder {
                Player::Hero => -(hero_total as f32),
                Player::Villain => villain_total as f32,
            };
        } else {
            // Showdown.
            let sign = self.showdown_sign[state.hero_combo as usize][state.villain_combo as usize];
            hero_from_hero_pov = match sign {
                1 => villain_total as f32,  // hero wins: gains opp commit
                -1 => -(hero_total as f32), // hero loses: loses own commit
                0 => 0.0,                   // tie: split, zero net
                _ => unreachable!("showdown_sign must be -1, 0, or 1"),
            };
        }

        match player {
            Player::Hero => hero_from_hero_pov,
            Player::Villain => -hero_from_hero_pov,
        }
    }

    fn current_player(&self, state: &Self::State) -> Player {
        assert!(
            !self.is_terminal(state),
            "NlheSubgame::current_player called on terminal state"
        );
        self.river_current_player(state)
    }

    fn legal_actions(&self, state: &Self::State) -> Vec<Self::Action> {
        debug_assert!(
            !self.is_terminal(state),
            "legal_actions called on terminal state"
        );
        self.legal_river_actions(state)
    }

    fn apply(&self, state: &Self::State, action: &Self::Action) -> Self::State {
        let mut next = state.clone();
        next.actions.push(Street::River, *action);
        next
    }

    fn info_set(&self, state: &Self::State, player: Player) -> InfoSetId {
        let combo = match player {
            Player::Hero => state.hero_combo,
            Player::Villain => state.villain_combo,
        };
        InfoSetId(info_set_hash(combo, player, &state.actions))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// 52-bit card mask (bit `card.0` set if `card` is present).
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

/// True if any of `cards` is already in `mask`.
fn card_conflict(mask: u64, cards: &[Card]) -> bool {
    cards.iter().any(|c| (mask & card_mask(*c)) != 0)
}

/// Deterministic FNV-1a style hash over `(combo, player, actions)`.
///
/// We deliberately avoid `DefaultHasher` (non-deterministic across
/// runs in some configs). FNV-1a gives a simple, portable, well-
/// distributed 32-bit hash that lets us pack an `InfoSetId` into a
/// single `u32`.
fn info_set_hash(combo: u16, player: Player, actions: &ActionLog) -> u32 {
    const FNV_OFFSET: u32 = 0x811c9dc5;
    const FNV_PRIME: u32 = 0x01000193;

    let mut h: u32 = FNV_OFFSET;

    // Mix combo (2 bytes).
    h ^= combo as u32 & 0xff;
    h = h.wrapping_mul(FNV_PRIME);
    h ^= (combo as u32 >> 8) & 0xff;
    h = h.wrapping_mul(FNV_PRIME);

    // Mix player so Hero's and Villain's info sets don't collide at
    // the same (combo, action-history).
    h ^= match player {
        Player::Hero => 0x11,
        Player::Villain => 0x22,
    };
    h = h.wrapping_mul(FNV_PRIME);

    // Mix actions (river-only for v0.1, but we fold in any street tag
    // too to be robust if the caller ever adds prior-street entries).
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

/// River-specific street-closure check.
///
/// [`ActionLog::is_street_closed`] operates on the *current* street
/// — which is always River by construction inside [`NlheSubgame`],
/// since we only ever push river entries. But we implement it
/// directly here so this module is robust to that invariant drifting.
fn river_is_closed(actions: &ActionLog) -> bool {
    let acts: Vec<Action> = actions.iter_street(Street::River).collect();
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
    // Trailing Check, Call, or AllIn.
    match last {
        Action::Check => {
            // Check-check closes; a lone Check does not.
            acts.len() >= 2 && matches!(acts[acts.len() - 2], Action::Check)
        }
        Action::Call => true,
        Action::AllIn => {
            // An AllIn by the opener (no prior aggression) re-opens the
            // street and is NOT a terminal — the opponent owes a
            // response. An AllIn by the responder (after a bet/raise)
            // is a raise and also re-opens. In both cases, the AllIn
            // is "aggression" and closes only when the opponent
            // eventually Calls or Folds. So AllIn alone does not close.
            false
        }
        Action::Fold | Action::Bet(_) | Action::Raise(_) => {
            unreachable!("handled above")
        }
    }
}

/// Build the `1326 × 1326` showdown sign matrix for this board.
fn build_showdown_matrix(board: &Board) -> Box<ShowdownMatrix> {
    // Allocate on the heap — this is ~1.76 MB (1326² bytes).
    //
    // We allocate a `Vec<i8>` of NUM_COMBOS² zeros, then reinterpret
    // it as a `Box<ShowdownMatrix>`. Going through `Vec<u8>` avoids
    // any stack-resident intermediate array: the vec lives on the
    // heap from the moment it's constructed. The `into_boxed_slice()`
    // + `TryInto<Box<[[i8; N]; N]>>` route can hit a stack blow-up on
    // some toolchains because the cast moves the array on the stack
    // transiently.
    let flat: Vec<i8> = vec![0i8; NUM_COMBOS * NUM_COMBOS];
    let boxed_slice = flat.into_boxed_slice();
    // SAFETY: we allocated exactly NUM_COMBOS * NUM_COMBOS * 1 = 1.76 MB
    // bytes of i8. `ShowdownMatrix` is `[[i8; NUM_COMBOS]; NUM_COMBOS]`,
    // which has the same layout (row-major, tightly packed, no padding
    // — i8 arrays are trivially alignment-1). So the pointer is a
    // valid `Box<ShowdownMatrix>` after reinterpretation.
    let ptr = Box::into_raw(boxed_slice) as *mut ShowdownMatrix;
    let mut mat: Box<ShowdownMatrix> = unsafe { Box::from_raw(ptr) };

    let board_mask = board_card_mask(board);

    // Precompute each combo's rank (or None if combo conflicts with board).
    let mut ranks: Vec<Option<solver_eval::eval::HandRank>> = Vec::with_capacity(NUM_COMBOS);
    for i in 0..NUM_COMBOS {
        let (a, b) = index_to_combo(i);
        if (board_mask & (card_mask(a) | card_mask(b))) != 0 {
            ranks.push(None);
        } else {
            let hand = Hand::new(a, b);
            ranks.push(Some(eval_7(&hand, board)));
        }
    }

    for i in 0..NUM_COMBOS {
        let (ai, bi) = index_to_combo(i);
        let i_mask = card_mask(ai) | card_mask(bi);
        let ri = match ranks[i] {
            Some(r) => r,
            None => continue, // row stays all zeros
        };
        for (j, rj_opt) in ranks.iter().enumerate().take(NUM_COMBOS) {
            if i == j {
                // Same combo: impossible deal (shared cards).
                continue;
            }
            let rj = match *rj_opt {
                Some(r) => r,
                None => continue,
            };
            let (aj, bj) = index_to_combo(j);
            if (i_mask & (card_mask(aj) | card_mask(bj))) != 0 {
                // Hero and villain share a card — impossible deal.
                continue;
            }
            use std::cmp::Ordering;
            mat[i][j] = match ri.cmp(&rj) {
                Ordering::Greater => 1,
                Ordering::Less => -1,
                Ordering::Equal => 0,
            };
        }
    }

    mat
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_board() -> Board {
        // A very average-looking river: AhKh2sQc7d.
        Board::parse("AhKh2sQc7d").unwrap()
    }

    #[test]
    fn construction_succeeds_for_river() {
        let sg = NlheSubgame::new(
            sample_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        // One obvious sanity: AA vs KK on A-high board — AA wins always.
        let aa_idx = {
            // AhAs share no board cards except Ah; AcAd is the valid
            // non-conflicting AA combo family. Pick AcAd.
            let a_c = Card::parse("Ac").unwrap();
            let a_d = Card::parse("Ad").unwrap();
            solver_eval::combo::combo_index(a_c, a_d)
        };
        let kk_idx = {
            // Only KK combo not using Kh is KcKd, KcKs, KdKs. Pick KcKd.
            let k_c = Card::parse("Kc").unwrap();
            let k_d = Card::parse("Kd").unwrap();
            solver_eval::combo::combo_index(k_c, k_d)
        };
        assert_eq!(sg.showdown_sign[aa_idx][kk_idx], 1);
        assert_eq!(sg.showdown_sign[kk_idx][aa_idx], -1);
    }

    #[test]
    #[should_panic(expected = "river only")]
    fn non_river_board_panics() {
        let _ = NlheSubgame::new(
            Board::parse("AhKh2s").unwrap(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
    }

    #[test]
    fn chance_roots_skip_card_conflicts() {
        // Hero range = "AA", board contains Ah. AA combos using Ah are
        // unplayable — only AcAd, AcAs, AdAs survive (3 combos).
        let sg = NlheSubgame::new(
            sample_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let triples = sg.enumerate_combo_pairs();
        // Villain range KK on a board with Kh → 3 valid KK combos.
        // 3 × 3 = 9 non-conflicting pairs. No hero/villain card overlap
        // possible (different ranks).
        assert_eq!(triples.len(), 9);
        // All non-conflicting pairs are equally weighted (all 1.0 * 1.0).
        for (_, _, w) in &triples {
            assert!((*w - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn legal_actions_at_root() {
        // Root state with stacks > 0 — first actor can check, bet some
        // sizings, or shove all-in.
        let sg = NlheSubgame::new(
            sample_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let root = sg.initial_state();
        let actions = sg.legal_actions(&root);
        assert!(actions.contains(&Action::Check));
        assert!(actions.contains(&Action::AllIn));
        // Should also contain at least one Bet(amount).
        assert!(actions.iter().any(|a| matches!(a, Action::Bet(_))));
    }

    #[test]
    fn fold_is_terminal() {
        let sg = NlheSubgame::new(
            sample_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let mut state = sg.initial_state();
        // Hero bets 50, Villain folds.
        state.actions.push(Street::River, Action::Bet(50));
        state.actions.push(Street::River, Action::Fold);
        assert!(sg.is_terminal(&state));
        // Villain folded → Hero wins villain's total commit = pot_start/2
        // (= 50). Hero gains 50 chips.
        let u = sg.utility(&state, Player::Hero);
        assert!((u - 50.0).abs() < 1e-4, "expected +50, got {u}");
    }

    #[test]
    fn check_check_goes_to_showdown() {
        // Hero AcAd, Villain KcKd. Showdown: AA wins.
        let sg = NlheSubgame::new(
            sample_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let hero_combo =
            solver_eval::combo::combo_index(Card::parse("Ac").unwrap(), Card::parse("Ad").unwrap());
        let villain_combo =
            solver_eval::combo::combo_index(Card::parse("Kc").unwrap(), Card::parse("Kd").unwrap());
        let state = SubgameState {
            hero_combo: hero_combo as u16,
            villain_combo: villain_combo as u16,
            actions: {
                let mut a = ActionLog::new();
                a.push(Street::River, Action::Check);
                a.push(Street::River, Action::Check);
                a
            },
        };
        assert!(sg.is_terminal(&state));
        // Nobody folded. Hero wins showdown; utility = villain's total
        // committed = pot_start/2 = 50.
        let u = sg.utility(&state, Player::Hero);
        assert!(
            (u - 50.0).abs() < 1e-4,
            "expected +50 on showdown win, got {u}"
        );
    }

    #[test]
    fn info_set_hash_differs_for_player_and_combo() {
        let alog = ActionLog::new();
        let h_hero = info_set_hash(123, Player::Hero, &alog);
        let h_villain = info_set_hash(123, Player::Villain, &alog);
        assert_ne!(
            h_hero, h_villain,
            "same combo at same history must map to different info sets for Hero vs Villain"
        );
        let h_other_combo = info_set_hash(124, Player::Hero, &alog);
        assert_ne!(
            h_hero, h_other_combo,
            "different combos must map to different info sets"
        );
    }

    #[test]
    fn info_set_hash_is_deterministic() {
        // Same inputs → same output across calls.
        let mut alog = ActionLog::new();
        alog.push(Street::River, Action::Bet(66));
        alog.push(Street::River, Action::Call);
        let h1 = info_set_hash(500, Player::Hero, &alog);
        let h2 = info_set_hash(500, Player::Hero, &alog);
        assert_eq!(h1, h2);
    }

    #[test]
    fn river_is_closed_semantics() {
        let mut al = ActionLog::new();
        assert!(!river_is_closed(&al));
        al.push(Street::River, Action::Check);
        assert!(!river_is_closed(&al), "single check does not close");
        al.push(Street::River, Action::Check);
        assert!(river_is_closed(&al), "check-check closes");

        let mut al = ActionLog::new();
        al.push(Street::River, Action::Bet(50));
        assert!(!river_is_closed(&al));
        al.push(Street::River, Action::Call);
        assert!(river_is_closed(&al));

        let mut al = ActionLog::new();
        al.push(Street::River, Action::Bet(50));
        al.push(Street::River, Action::AllIn);
        // AllIn is a raise here — NOT closed.
        assert!(!river_is_closed(&al));
        al.push(Street::River, Action::Fold);
        assert!(river_is_closed(&al));
    }
}
