//! `NlheSubgameVector` — [`solver_core::VectorGame`] impl for NLHE river
//! subgames.
//!
//! Wraps the same domain data as [`crate::NlheSubgame`] (board, ranges,
//! stacks, bet tree, showdown-sign matrix) but exposes an action-only
//! game tree for the Vector CFR solver. The combo axis (1326 NLHE
//! combos) becomes a SIMD-vectorized lane dimension instead of a
//! per-root walk driver.
//!
//! # Why a sibling type
//!
//! `NlheSubgame` implements [`solver_core::Game`], which keys info sets
//! on `(acting_player_combo, action_history)` — the scalar formulation.
//! Vector CFR keys on `(acting_player, action_history)` only, with the
//! combo as a lane. The two info-set IDs are incompatible, so the
//! types are kept separate rather than overloading `NlheSubgame` with
//! two info-set conventions.
//!
//! A vector-side subgame shares the **same** domain-level parameters
//! as its scalar twin: same board, same ranges, same pot/stack, same
//! bet tree, same showdown-sign matrix (built once). So we reuse the
//! scalar subgame's construction and borrow the heavy state where
//! possible.

use smallvec::SmallVec;
use solver_core::{InfoSetId, Player, VectorGame};
use solver_eval::board::Board;
use solver_eval::combo::{index_to_combo, NUM_COMBOS};

use crate::action::{Action, ActionLog, Street};
use crate::bet_tree::BetTree;
use crate::range::Range;

/// NLHE river subgame for Vector CFR.
///
/// Holds the same domain data as [`crate::NlheSubgame`] but exposes
/// an action-only tree through the [`VectorGame`] trait. The
/// `showdown_sign` matrix is built eagerly at construction time
/// (O(N²), ~1.76 MB).
pub struct NlheSubgameVector {
    board: Board,
    hero_range: Range,
    villain_range: Range,
    pot_start: u32,
    stack_start: u32,
    first_to_act: Player,
    bet_tree: BetTree,

    /// `showdown_sign[hero_combo][villain_combo]`:
    /// `+1` = hero beats villain, `-1` = villain beats hero, `0` = tie
    /// or dead-card conflict.
    showdown_sign: Box<ShowdownMatrix>,

    /// Per-combo initial reach for each side: the range weight with
    /// board-conflicting combos zeroed out. Product `hero[h] *
    /// villain[v]` = the chance-layer weight for pair `(h, v)` (when
    /// `h != v` and they don't share a card — the `fill_terminal_utility`
    /// + `cf_reach` math enforces that).
    hero_initial: Box<[f32; NUM_COMBOS]>,
    villain_initial: Box<[f32; NUM_COMBOS]>,

    /// Normalizer for the chance-layer prior. The scalar path's
    /// `chance_roots` enumerates pairs and normalizes weights to sum
    /// to 1; we fold that same normalizer in here so the per-pair
    /// effective weight `hero_initial[h] * villain_initial[v] /
    /// chance_norm` matches the scalar's per-root weight.
    chance_norm: f32,

    /// Per-hero-combo bitmask of the 2-card combo (hero's cards).
    /// Used by the fold-terminal / cf_reach helpers to enforce
    /// pair-validity (no shared cards) efficiently.
    hero_card_mask: Box<[u64; NUM_COMBOS]>,

    /// Indices of hero combos with non-zero initial reach.
    ///
    /// The walker outputs a 1326-wide utility per decision node, but
    /// only active hero combos contribute to the average strategy.
    /// Inactive lanes are zero-reach throughout, so all their
    /// regret/strategy-sum updates produce zero. Recording the active
    /// set lets the terminal hot loops skip inactive lanes entirely.
    hero_active: Vec<u16>,

    /// Same for villain combos.
    villain_active: Vec<u16>,
}

type ShowdownMatrix = [[i8; NUM_COMBOS]; NUM_COMBOS];

/// Action-only state for Vector CFR walks.
///
/// The combo fields in [`crate::subgame::SubgameState`] are gone — the
/// combo is a lane in the vector walk, not part of the state.
#[derive(Debug, Clone, Default)]
pub struct ActionState {
    /// Actions on the river street.
    pub actions: ActionLog,
}

impl ActionState {
    /// Create an empty action state (root of the action-only tree).
    pub fn new() -> Self {
        Self::default()
    }
}

impl NlheSubgameVector {
    /// Build a new vector-CFR river subgame.
    ///
    /// # Panics
    ///
    /// Panics if `board.len != 5`.
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
            "NlheSubgameVector::new: v0.1 handles river only (got {})",
            board.len
        );

        let board_mask = board_card_mask(&board);

        // Per-combo card mask (0 for board-conflicting combos, so they
        // never contribute). Also zeroes out hero/villain initial reach
        // for the board-conflict case.
        let mut hero_initial = Box::new([0.0f32; NUM_COMBOS]);
        let mut villain_initial = Box::new([0.0f32; NUM_COMBOS]);
        let mut hero_card_mask = Box::new([0u64; NUM_COMBOS]);
        for combo in 0..NUM_COMBOS {
            let (a, b) = index_to_combo(combo);
            let mask = (1u64 << a.0) | (1u64 << b.0);
            if (board_mask & mask) == 0 {
                hero_card_mask[combo] = mask;
                hero_initial[combo] = hero_range.weights[combo];
                villain_initial[combo] = villain_range.weights[combo];
            }
            // Else: leave initials at 0.0 and mask at 0 — the combo is
            // dead (shares a card with the board).
        }

        // Chance-layer normalization: sum over all valid (h, v) of
        // hero_weight[h] * villain_weight[v].
        let mut total_pair_weight = 0.0f32;
        for h in 0..NUM_COMBOS {
            let hw = hero_initial[h];
            if hw <= 0.0 {
                continue;
            }
            let h_mask = hero_card_mask[h];
            for v in 0..NUM_COMBOS {
                let vw = villain_initial[v];
                if vw <= 0.0 {
                    continue;
                }
                let v_mask = hero_card_mask[v];
                if (h_mask & v_mask) != 0 {
                    continue;
                }
                total_pair_weight += hw * vw;
            }
        }
        let chance_norm = if total_pair_weight > 0.0 {
            total_pair_weight
        } else {
            1.0
        };

        let showdown_sign = build_showdown_matrix(&board, &hero_card_mask);

        let hero_active: Vec<u16> = (0..NUM_COMBOS as u16)
            .filter(|&i| hero_initial[i as usize] > 0.0)
            .collect();
        let villain_active: Vec<u16> = (0..NUM_COMBOS as u16)
            .filter(|&i| villain_initial[i as usize] > 0.0)
            .collect();

        Self {
            board,
            hero_range,
            villain_range,
            pot_start,
            stack_start,
            first_to_act,
            bet_tree,
            showdown_sign,
            hero_initial,
            villain_initial,
            chance_norm,
            hero_card_mask,
            hero_active,
            villain_active,
        }
    }

    /// Hero's active combo indices (combos with non-zero range weight
    /// and no board conflict).
    pub fn hero_active(&self) -> &[u16] {
        &self.hero_active
    }

    /// Villain's active combo indices.
    pub fn villain_active(&self) -> &[u16] {
        &self.villain_active
    }

    /// Borrow the board.
    pub fn board(&self) -> &Board {
        &self.board
    }

    /// Effective stack on the river.
    pub fn stack_start(&self) -> u32 {
        self.stack_start
    }

    /// First-to-act on the river.
    pub fn first_to_act(&self) -> Player {
        self.first_to_act
    }

    /// Bet-tree profile.
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

    // ---- Tree-walk helpers (mirrors of NlheSubgame's scalar helpers) ----

    fn street_contributions(&self, state: &ActionState) -> (u32, u32) {
        let (sb_slot, bb_slot) = state.actions.pot_contributions_on(Street::River);
        let (first_actor, second_actor) = (bb_slot, sb_slot);
        match self.first_to_act {
            Player::Hero => (first_actor, second_actor),
            Player::Villain => (second_actor, first_actor),
        }
    }

    fn committed_totals(&self, state: &ActionState) -> (u32, u32) {
        let (hs, vs) = self.street_contributions(state);
        let half = self.pot_start / 2;
        let other_half = self.pot_start - half;
        (half + hs, other_half + vs)
    }

    fn river_current_player(&self, state: &ActionState) -> Player {
        let n = state.actions.iter_street(Street::River).count();
        if n % 2 == 0 {
            self.first_to_act
        } else {
            self.first_to_act.opponent()
        }
    }

    fn legal_river_actions(&self, state: &ActionState) -> SmallVec<[Action; 8]> {
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
            let to_call = opp_street - my_street;
            let mut actions: SmallVec<[Action; 8]> = SmallVec::new();
            actions.push(Action::Fold);
            if my_stack_remaining >= to_call {
                actions.push(Action::Call);
            }
            let last_was_allin_raise = matches!(
                state.actions.iter_street(Street::River).last(),
                Some(Action::AllIn) | Some(Action::Raise(_))
            );
            if !last_was_allin_raise && my_stack_remaining > to_call && my_stack_remaining > 0 {
                actions.push(Action::AllIn);
            }
            actions
        } else {
            let mut actions: SmallVec<[Action; 8]> = SmallVec::new();
            actions.push(Action::Check);
            if my_stack_remaining == 0 {
                return actions;
            }
            for &fraction in self.bet_tree.sizings_for(Street::River) {
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
        match last {
            Action::Check => acts.len() >= 2 && matches!(acts[acts.len() - 2], Action::Check),
            Action::Call => true,
            Action::AllIn => false,
            Action::Fold | Action::Bet(_) | Action::Raise(_) => {
                unreachable!("handled above")
            }
        }
    }

    /// Inner helper: write per-combo utility for `update_player` at a
    /// FOLD terminal. Fold amount depends on who folded. The
    /// (my_combo, opp_combo) pair-validity (no shared cards) is the
    /// hot loop here; we enforce it via the per-combo card masks.
    fn fill_fold_terminal_utility(
        &self,
        state: &ActionState,
        update_player: Player,
        reach_opp: &[f32],
        out: &mut [f32],
    ) {
        // Fold utility is player-independent of the specific combos
        // beyond "did hero or villain fold". Who folded is recovered
        // from the action log's fold position.
        let fold_idx = state
            .actions
            .iter_street(Street::River)
            .position(|a| matches!(a, Action::Fold))
            .expect("fill_fold_terminal_utility: no fold in history");
        let folder = if fold_idx % 2 == 0 {
            self.first_to_act
        } else {
            self.first_to_act.opponent()
        };
        let (hero_total, villain_total) = self.committed_totals(state);

        // hero_from_hero_pov: if folder == hero, hero loses hero_total;
        // else hero wins villain_total. We flip sign for update_player.
        let hero_u = match folder {
            Player::Hero => -(hero_total as f32),
            Player::Villain => villain_total as f32,
        };
        let u_for_update = match update_player {
            Player::Hero => hero_u,
            Player::Villain => -hero_u,
        };

        // For each `my` (update_player's combo), out[my] = u_for_update
        // * (sum over valid opp of reach_opp[opp] / chance_norm).
        //
        // The pair-validity check — my's cards ∩ opp's cards = ∅ — is a
        // bitmask AND. We use the precomputed `hero_card_mask` (same
        // per-combo mask regardless of role: it's just "which 2 cards
        // does combo c hold").
        //
        // Optimization: compute `total_reach = Σ_v reach_opp[v]` once,
        // then for each `my` subtract the reaches of conflicting
        // opp combos. Each `my` has at most 99 conflicting opp combos
        // (combos using hero's 2 cards: C(52-0, 1) * 2 - 1 ≈ 99);
        // iterating those is far cheaper than iterating all 1326 for
        // every `my`.
        let active_opp: &[u16] = match update_player {
            Player::Hero => &self.villain_active,
            Player::Villain => &self.hero_active,
        };
        let total_reach: f32 = active_opp.iter().map(|&i| reach_opp[i as usize]).sum();
        let scale = u_for_update / self.chance_norm;

        let my_active: &[u16] = match update_player {
            Player::Hero => &self.hero_active,
            Player::Villain => &self.villain_active,
        };
        // Zero all lanes first.
        for slot in out.iter_mut() {
            *slot = 0.0;
        }
        for &my_u in my_active {
            let my = my_u as usize;
            let my_mask = self.hero_card_mask[my];
            // Subtract reaches of conflicting opp combos.
            let mut conflict_reach = 0.0f32;
            for &opp_u in active_opp {
                let opp = opp_u as usize;
                if (my_mask & self.hero_card_mask[opp]) != 0 {
                    conflict_reach += reach_opp[opp];
                }
            }
            out[my] = scale * (total_reach - conflict_reach);
        }
    }

    /// Inner helper: write per-combo utility at a SHOWDOWN terminal.
    ///
    /// The showdown sign matrix encodes (hero beats villain = +1,
    /// villain beats hero = -1, tie or conflict = 0). The output
    /// per-combo utility factors into:
    ///
    ///   out[my] = (1 / chance_norm) * Σ_v reach_opp[v] *
    ///       [ max(sign, 0) * +win_chips
    ///       + min(sign, 0) * -lose_chips ]
    ///
    /// where `sign = signs[my][v]` for update=Hero (and `signs[v][my]`
    /// for update=Villain), `win_chips` is the chips the update player
    /// wins on a victory, and `lose_chips` is what they lose on a
    /// defeat. The `max/min` trick removes the branch and lets us
    /// vectorize.
    ///
    /// The inner loop is SIMD-widened by 16 (i8×16 load per chunk,
    /// unpacked to two f32×8 halves), which is where most of the gain
    /// for the v0.2 perf target comes from — the terminal showdown
    /// integral is the hot path.
    fn fill_showdown_terminal_utility(
        &self,
        state: &ActionState,
        update_player: Player,
        reach_opp: &[f32],
        out: &mut [f32],
    ) {
        let (hero_total, villain_total) = self.committed_totals(state);
        let hero_chips = hero_total as f32;
        let villain_chips = villain_total as f32;
        let inv_norm = 1.0 / self.chance_norm;

        // Win/lose chips from the update player's perspective:
        //   update=Hero:    sign=+1 -> +villain_chips; sign=-1 -> -hero_chips
        //   update=Villain: sign=+1 -> -hero_chips;    sign=-1 -> +villain_chips
        //
        // Using the max/min rewrite:
        //   contrib = max(sign, 0) * A + min(sign, 0) * B
        //   - update=Hero:    A = +villain_chips, B = +hero_chips
        //     (max*A = sign*villain_chips for sign=+1; min*B = -hero_chips for sign=-1)
        //   - update=Villain: sign at (hero=v, villain=my) is negated
        //     relative to villain's perspective; we can instead look up
        //     the column (signs[v][my]) and flip A/B:
        //     A = -hero_chips, B = -villain_chips (so min*(-villain_chips) = +villain_chips for sign=-1,
        //     and max*(-hero_chips) = -hero_chips for sign=+1 — hero won → villain lost).

        match update_player {
            Player::Hero => {
                self.showdown_matmul_rows(
                    reach_opp,
                    out,
                    /* win_coeff = */ villain_chips,
                    /* lose_coeff = */ hero_chips,
                    inv_norm,
                );
            }
            Player::Villain => {
                // Villain lane `my` integrates over hero lane `v` using
                // signs[v][my] — a COLUMN of the matrix. To avoid a
                // column-major walk (cache-hostile), we transpose by
                // iterating `v` outer and accumulating into `out[my]`
                // via a different kernel.
                self.showdown_matmul_cols(
                    reach_opp,
                    out,
                    /* win_coeff_for_villain_loss */ hero_chips,
                    /* lose_coeff_for_villain_win */ villain_chips,
                    inv_norm,
                );
            }
        }
    }

    /// Row-major showdown matmul for update=Hero.
    ///
    /// For each hero combo `my`, compute:
    ///   out[my] = inv_norm * Σ_v reach_opp[v] *
    ///       (max(signs[my][v], 0) * win_coeff
    ///      - max(-signs[my][v], 0) * lose_coeff)
    ///
    /// SIMD: process 8 villain combos at a time. signs[my][v..v+8] is
    /// 8 i8 bytes; convert to f32x8; multiply by reach_opp[v..v+8]
    /// (as f32x8); accumulate two partial sums (positive-contribs and
    /// negative-contribs). Finalize per row.
    fn showdown_matmul_rows(
        &self,
        reach_opp: &[f32],
        out: &mut [f32],
        win_coeff: f32,
        lose_coeff: f32,
        inv_norm: f32,
    ) {
        // Zero the output first, then fill only active hero combos.
        for slot in out.iter_mut() {
            *slot = 0.0;
        }

        for &my_u in &self.hero_active {
            let my = my_u as usize;
            let row = &self.showdown_sign[my];
            let (pos_sum, neg_sum) = showdown_row_pos_neg(row, reach_opp);
            out[my] = (pos_sum * win_coeff - neg_sum * lose_coeff) * inv_norm;
        }
    }

    // showdown_matmul_cols dispatches through `showdown_row_scatter_pos_neg`
    // (same NEON/wide dispatch as rows).

    /// Column-major showdown matmul for update=Villain.
    ///
    /// For each villain combo `my`, integrates over hero combos `v`:
    ///   out[my] = inv_norm * Σ_v reach_opp[v] *
    ///       (max(-signs[v][my], 0) * win_coeff   [sign=-1 => villain won]
    ///      - max( signs[v][my], 0) * lose_coeff) [sign=+1 => villain lost]
    ///
    /// A column-major access pattern on `signs[v][my]` would be cache-
    /// hostile (striding by NUM_COMBOS bytes). We instead swap loop
    /// order: iterate `v` outer, scatter-accumulate into `out` via the
    /// row `signs[v]` (cache-friendly) with `reach_opp[v]` as a scalar
    /// multiplier. This is an outer-product-style pass.
    fn showdown_matmul_cols(
        &self,
        reach_opp: &[f32],
        out: &mut [f32],
        win_coeff: f32,
        lose_coeff: f32,
        inv_norm: f32,
    ) {
        // Allocate a scratch for the "neg" accumulator alongside
        // `out` (which doubles as the "pos" accumulator). This vec
        // allocation per call is the cost we trade for not holding a
        // persistent per-subgame scratch; the O(N²) inner loop
        // dominates. If allocation ever shows up in a profile, lift
        // this into the scratch pool.
        let mut neg_out = vec![0.0f32; NUM_COMBOS];
        for slot in out.iter_mut() {
            *slot = 0.0;
        }

        // Only iterate over villain's active combos AND only when their
        // reach is non-zero. reach_opp[v] == 0 for many lanes even in
        // the active set (e.g., after heavy villain-strategy pruning).
        for &v_u in &self.hero_active {
            let v = v_u as usize;
            let r = reach_opp[v];
            if r == 0.0 {
                continue;
            }
            let row = &self.showdown_sign[v];
            showdown_row_scatter_pos_neg(row, r, out, &mut neg_out);
        }

        // Finalize: out[my] = (-pos[my] * lose_coeff + neg[my] * win_coeff) * inv_norm
        //
        // Our `out` currently holds `pos[my]` (villain-loss pairs) and
        // `neg_out` holds `neg[my]` (villain-win pairs). Only villain's
        // active combos matter downstream; zero-reach villain combos
        // don't affect subsequent walks (their own_reach is 0 in any
        // strategy_sum update), but we still need `out[my]` to be well-
        // defined for the walker's bookkeeping, so we write across all
        // combos to keep the contract simple.
        for my in 0..NUM_COMBOS {
            if self.hero_card_mask[my] == 0 {
                out[my] = 0.0;
                continue;
            }
            out[my] = (neg_out[my] * win_coeff - out[my] * lose_coeff) * inv_norm;
        }
    }
}

impl VectorGame for NlheSubgameVector {
    type State = ActionState;
    type Action = Action;

    fn combo_width(&self) -> usize {
        NUM_COMBOS
    }

    fn root(&self) -> ActionState {
        ActionState::new()
    }

    fn initial_reach(&self, player: Player, out: &mut [f32]) {
        debug_assert_eq!(out.len(), NUM_COMBOS);
        let src: &[f32; NUM_COMBOS] = match player {
            Player::Hero => &self.hero_initial,
            Player::Villain => &self.villain_initial,
        };
        out.copy_from_slice(src);
    }

    fn is_terminal(&self, state: &ActionState) -> bool {
        let has_fold = state
            .actions
            .iter_street(Street::River)
            .any(|a| matches!(a, Action::Fold));
        has_fold || Self::river_is_closed(&state.actions)
    }

    fn current_player(&self, state: &ActionState) -> Player {
        self.river_current_player(state)
    }

    fn legal_actions(&self, state: &ActionState) -> SmallVec<[Action; 8]> {
        self.legal_river_actions(state)
    }

    fn apply(&self, state: &ActionState, action: &Action) -> ActionState {
        let mut next = state.clone();
        // Substitute AllIn with Bet(stack_start) / Raise(stack_start)
        // so the action log's chip-total accounting works. Mirrors
        // NlheSubgame::apply.
        let resolved = match action {
            Action::AllIn => {
                let (hs, vs) = self.street_contributions(state);
                let current = self.river_current_player(state);
                let (my_street, opp_street) = match current {
                    Player::Hero => (hs, vs),
                    Player::Villain => (vs, hs),
                };
                if opp_street > my_street {
                    Action::Raise(self.stack_start)
                } else {
                    Action::Bet(self.stack_start)
                }
            }
            other => *other,
        };
        next.actions.push(Street::River, resolved);
        next
    }

    fn info_set_id(&self, state: &ActionState, player: Player) -> InfoSetId {
        // Action-only info-set ID: hash of (player, action_history).
        // Combo is a lane, not part of the key.
        InfoSetId(action_only_info_set_hash(player, &state.actions))
    }

    fn fill_terminal_utility(
        &self,
        state: &ActionState,
        update_player: Player,
        reach_opp: &[f32],
        out: &mut [f32],
    ) {
        let has_fold = state
            .actions
            .iter_street(Street::River)
            .any(|a| matches!(a, Action::Fold));
        if has_fold {
            self.fill_fold_terminal_utility(state, update_player, reach_opp, out);
        } else {
            self.fill_showdown_terminal_utility(state, update_player, reach_opp, out);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn board_card_mask(board: &Board) -> u64 {
    let mut m = 0u64;
    for c in board.as_slice() {
        m |= 1u64 << c.0;
    }
    m
}

/// Row-inner kernel for `showdown_matmul_rows`: returns `(pos, neg)`
/// where `pos = Σ_j max(sign_row[j] as f32 * reach_opp[j], 0)` and
/// `neg = Σ_j max(-sign_row[j] as f32 * reach_opp[j], 0)`.
///
/// On aarch64 dispatches to the hand-rolled NEON kernel; elsewhere
/// uses the `wide::f32x8` fallback kept for x86/scalar targets.
#[inline]
fn showdown_row_pos_neg(sign_row: &[i8; NUM_COMBOS], reach_opp: &[f32]) -> (f32, f32) {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is always available on aarch64 macOS (and on
        // all aarch64 targets Rust supports). `reach_opp` is
        // length-NUM_COMBOS per the matmul contract (debug-asserted
        // inside the kernel).
        unsafe {
            return crate::subgame_vector_neon::showdown_row_pos_neg_neon(
                sign_row.as_slice(),
                reach_opp,
            );
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        showdown_row_pos_neg_wide(sign_row, reach_opp)
    }
}

/// Scatter-inner kernel for `showdown_matmul_cols`: for each lane `j`
/// in `0..NUM_COMBOS`, does
///   `pos_out[j] += max(sign_row[j] as f32 * r, 0)` and
///   `neg_out[j] += max(-sign_row[j] as f32 * r, 0)`.
///
/// On aarch64 dispatches to the hand-rolled NEON kernel; elsewhere
/// uses the `wide::f32x8` fallback.
#[inline]
fn showdown_row_scatter_pos_neg(
    sign_row: &[i8; NUM_COMBOS],
    r: f32,
    pos_out: &mut [f32],
    neg_out: &mut [f32],
) {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: pos_out / neg_out are length-NUM_COMBOS per matmul
        // contract (debug-asserted inside the kernel).
        unsafe {
            crate::subgame_vector_neon::showdown_row_scatter_pos_neg_neon(
                sign_row.as_slice(),
                r,
                pos_out,
                neg_out,
            );
            return;
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        showdown_row_scatter_pos_neg_wide(sign_row, r, pos_out, neg_out)
    }
}

/// Portable `wide`-based fallback for the row-inner kernel. Kept in
/// sync with the NEON version so x86/scalar builds stay green.
#[cfg(not(target_arch = "aarch64"))]
#[inline]
fn showdown_row_pos_neg_wide(sign_row: &[i8; NUM_COMBOS], reach_opp: &[f32]) -> (f32, f32) {
    use wide::f32x8;
    let chunks = NUM_COMBOS / 8;
    let tail_start = chunks * 8;
    let zero8 = f32x8::splat(0.0);

    let mut pos_acc = zero8;
    let mut neg_acc = zero8;
    for c in 0..chunks {
        let base = c * 8;
        let s8: [f32; 8] = [
            sign_row[base] as f32,
            sign_row[base + 1] as f32,
            sign_row[base + 2] as f32,
            sign_row[base + 3] as f32,
            sign_row[base + 4] as f32,
            sign_row[base + 5] as f32,
            sign_row[base + 6] as f32,
            sign_row[base + 7] as f32,
        ];
        let s_v = f32x8::from(s8);
        let r_slice: [f32; 8] = reach_opp[base..base + 8].try_into().unwrap();
        let r_v = f32x8::from(r_slice);
        let rs = s_v * r_v;
        pos_acc += rs.fast_max(zero8);
        neg_acc += (-rs).fast_max(zero8);
    }
    let mut pos_sum = pos_acc.reduce_add();
    let mut neg_sum = neg_acc.reduce_add();
    for v in tail_start..NUM_COMBOS {
        let rs = sign_row[v] as f32 * reach_opp[v];
        if rs > 0.0 {
            pos_sum += rs;
        } else if rs < 0.0 {
            neg_sum += -rs;
        }
    }
    (pos_sum, neg_sum)
}

/// Portable `wide`-based fallback for the scatter kernel.
#[cfg(not(target_arch = "aarch64"))]
#[inline]
fn showdown_row_scatter_pos_neg_wide(
    sign_row: &[i8; NUM_COMBOS],
    r: f32,
    pos_out: &mut [f32],
    neg_out: &mut [f32],
) {
    use wide::f32x8;
    let chunks = NUM_COMBOS / 8;
    let tail_start = chunks * 8;
    let zero8 = f32x8::splat(0.0);
    let r_v = f32x8::splat(r);

    for c in 0..chunks {
        let base = c * 8;
        let s8: [f32; 8] = [
            sign_row[base] as f32,
            sign_row[base + 1] as f32,
            sign_row[base + 2] as f32,
            sign_row[base + 3] as f32,
            sign_row[base + 4] as f32,
            sign_row[base + 5] as f32,
            sign_row[base + 6] as f32,
            sign_row[base + 7] as f32,
        ];
        let s_v = f32x8::from(s8);
        let rs = s_v * r_v;
        let p_slice: [f32; 8] = pos_out[base..base + 8].try_into().unwrap();
        let n_slice: [f32; 8] = neg_out[base..base + 8].try_into().unwrap();
        let p = f32x8::from(p_slice);
        let n = f32x8::from(n_slice);
        let p_new = p + rs.fast_max(zero8);
        let n_new = n + (-rs).fast_max(zero8);
        let p_arr: [f32; 8] = p_new.into();
        let n_arr: [f32; 8] = n_new.into();
        pos_out[base..base + 8].copy_from_slice(&p_arr);
        neg_out[base..base + 8].copy_from_slice(&n_arr);
    }
    for my in tail_start..NUM_COMBOS {
        let s = sign_row[my] as f32;
        let rs = s * r;
        if rs > 0.0 {
            pos_out[my] += rs;
        } else if rs < 0.0 {
            neg_out[my] += -rs;
        }
    }
}

/// FNV-1a style hash over `(player, action_history)` only — no combo.
fn action_only_info_set_hash(player: Player, actions: &ActionLog) -> u32 {
    const FNV_OFFSET: u32 = 0x811c9dc5;
    const FNV_PRIME: u32 = 0x01000193;
    let mut h: u32 = FNV_OFFSET;
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

/// Build the 1326×1326 showdown-sign matrix with pair-validity (same
/// cards — either with board or with the other combo — yields 0 so
/// the matmul naturally skips those entries).
fn build_showdown_matrix(board: &Board, hero_card_mask: &[u64; NUM_COMBOS]) -> Box<ShowdownMatrix> {
    use solver_eval::eval::eval_7;
    use solver_eval::hand::Hand;

    let flat: Vec<i8> = vec![0i8; NUM_COMBOS * NUM_COMBOS];
    let boxed_slice = flat.into_boxed_slice();
    // SAFETY: byte layout matches [[i8; N]; N], alignment-1.
    let ptr = Box::into_raw(boxed_slice) as *mut ShowdownMatrix;
    let mut mat: Box<ShowdownMatrix> = unsafe { Box::from_raw(ptr) };

    // Precompute each combo's hand rank (or None if it conflicts with
    // the board — hero_card_mask[combo] == 0 already encodes that).
    let mut ranks: Vec<Option<solver_eval::eval::HandRank>> = Vec::with_capacity(NUM_COMBOS);
    for (i, &m) in hero_card_mask.iter().enumerate() {
        if m == 0 {
            ranks.push(None);
            continue;
        }
        let (a, b) = index_to_combo(i);
        let hand = Hand::new(a, b);
        ranks.push(Some(eval_7(&hand, board)));
    }

    for i in 0..NUM_COMBOS {
        let i_mask = hero_card_mask[i];
        let ri = match ranks[i] {
            Some(r) => r,
            None => continue,
        };
        for (j, rj_opt) in ranks.iter().enumerate().take(NUM_COMBOS) {
            if i == j {
                continue;
            }
            let rj = match *rj_opt {
                Some(r) => r,
                None => continue,
            };
            let j_mask = hero_card_mask[j];
            if (i_mask & j_mask) != 0 {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use solver_eval::card::Card;
    use solver_eval::combo::combo_index;

    fn sample_board() -> Board {
        Board::parse("AhKh2sQc7d").unwrap()
    }

    #[test]
    fn construction_builds_showdown_matrix() {
        let sg = NlheSubgameVector::new(
            sample_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let aa_idx = combo_index(Card::parse("Ac").unwrap(), Card::parse("Ad").unwrap());
        let kk_idx = combo_index(Card::parse("Kc").unwrap(), Card::parse("Kd").unwrap());
        assert_eq!(sg.showdown_sign[aa_idx][kk_idx], 1);
        assert_eq!(sg.showdown_sign[kk_idx][aa_idx], -1);
    }

    #[test]
    fn legal_actions_at_root_mirror_scalar() {
        let sg = NlheSubgameVector::new(
            sample_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let root = sg.root();
        let actions = sg.legal_actions(&root);
        assert!(actions.contains(&Action::Check));
        assert!(actions.contains(&Action::AllIn));
        assert!(actions.iter().any(|a| matches!(a, Action::Bet(_))));
    }

    #[test]
    fn fold_terminal_matches_expectation() {
        // Hero opens bet, villain folds. At that terminal, villain has
        // given up their street contribution; hero's per-combo utility
        // is +villain_total_committed (= pot_start/2 in this no-prior-
        // action setup).
        let sg = NlheSubgameVector::new(
            sample_board(),
            Range::parse("AA").unwrap(),
            Range::parse("KK").unwrap(),
            100,
            200,
            Player::Hero,
            BetTree::default_v0_1(),
        );
        let mut state = sg.root();
        state.actions.push(Street::River, Action::Bet(50));
        state.actions.push(Street::River, Action::Fold);
        assert!(sg.is_terminal(&state));

        // Set villain reach to 1.0 for all non-conflicting combos.
        let mut reach_villain = vec![0.0f32; NUM_COMBOS];
        sg.initial_reach(Player::Villain, &mut reach_villain);
        let mut out = vec![0.0f32; NUM_COMBOS];
        sg.fill_terminal_utility(&state, Player::Hero, &reach_villain, &mut out);
        // Out must be non-negative (hero wins on fold) on all valid hero
        // combos and zero on board-conflicting combos.
        for (i, &u) in out.iter().enumerate() {
            if sg.hero_card_mask[i] == 0 {
                assert_eq!(u, 0.0);
            } else if sg.hero_initial[i] > 0.0 {
                assert!(
                    u >= 0.0,
                    "fold-win utility must be non-negative at combo {i}"
                );
            }
        }
    }

    #[test]
    fn action_only_info_set_differs_for_history() {
        let empty = ActionLog::new();
        let mut after_check = ActionLog::new();
        after_check.push(Street::River, Action::Check);

        let h1 = action_only_info_set_hash(Player::Hero, &empty);
        let h2 = action_only_info_set_hash(Player::Hero, &after_check);
        assert_ne!(h1, h2);
    }

    #[test]
    fn action_only_info_set_is_combo_independent() {
        // Confirm that the hash depends on (player, history) only —
        // not on any combo. (The hash function doesn't take a combo,
        // so this is trivially true, but this test is a regression
        // guard if someone ever adds one.)
        let mut h = ActionLog::new();
        h.push(Street::River, Action::Bet(50));
        let a = action_only_info_set_hash(Player::Hero, &h);
        let b = action_only_info_set_hash(Player::Hero, &h);
        assert_eq!(a, b);
    }
}
