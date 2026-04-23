//! NLHE actions and action history.
//!
//! See `docs/POKER.md` for domain context.

use smallvec::SmallVec;
use solver_core::Player;

/// The four streets of a NLHE hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Street {
    /// Before any community cards.
    Preflop = 0,
    /// After the 3-card flop.
    Flop = 1,
    /// After the turn (4th community card).
    Turn = 2,
    /// After the river (5th community card).
    River = 3,
}

/// A player action at a decision node.
///
/// Bet amounts are in chips, not pot fractions — the bet tree translates
/// pot-fractions to absolute chips when the subgame is constructed.
///
/// `Bet(x)` and `Raise(x)` both encode the **total chip amount for this
/// street**, not the increment. `Bet(50)` followed by `Raise(150)` means
/// the raiser put 150 chips in on this street, of which 100 is "new"
/// money beyond the call of 50.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Give up the hand.
    Fold,
    /// Pass (no bet to face).
    Check,
    /// Match the current bet.
    Call,
    /// Bet a specific chip amount (must correspond to a bet-tree bucket).
    Bet(u32),
    /// Raise to a specific chip amount.
    Raise(u32),
    /// All-in (bet effective stack).
    AllIn,
}

/// An ordered sequence of actions taken so far in a hand.
///
/// Stores `(street, action)` tuples in deal order. Used by the subgame
/// tree builder to identify decision nodes and to reconstruct pot / stack
/// state from scratch.
///
/// ## What is (and isn't) in the log
///
/// Forced bets (small blind / big blind posts) are **not** stored as
/// `Action` entries — they're part of the static subgame setup. The log
/// only records *voluntary* actions. However, [`Self::pot_contributions_on`]
/// folds the blind posts into the preflop contribution totals because
/// downstream code wants the real chips-in-pot-this-street number.
///
/// ## Player convention
///
/// v0.1 is heads-up. Hero is the SB (and preflop opener / postflop last-
/// to-act by the standard HU convention). The helpers here that talk about
/// "who acts" use a simpler abstraction — see [`Self::to_act`] — and the
/// subgame builder is expected to map that onto HU rules at construction
/// time.
#[derive(Debug, Clone, Default)]
pub struct ActionLog {
    entries: SmallVec<[(Street, Action); 16]>,
}

impl ActionLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of entries in the log.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if no actions have been logged yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Append an action taken on `street`.
    pub fn push(&mut self, street: Street, action: Action) {
        self.entries.push((street, action));
    }

    /// Remove and return the most recent entry, if any.
    pub fn pop(&mut self) -> Option<(Street, Action)> {
        self.entries.pop()
    }

    /// Iterate all `(street, action)` pairs in deal order.
    pub fn iter(&self) -> impl Iterator<Item = (Street, Action)> + '_ {
        self.entries.iter().copied()
    }

    /// Iterate just the entries on `street`, preserving deal order.
    pub fn iter_street(&self, street: Street) -> impl Iterator<Item = Action> + '_ {
        self.entries
            .iter()
            .filter(move |(s, _)| *s == street)
            .map(|(_, a)| *a)
    }

    /// The street of the last entry, or [`Street::Preflop`] if the log is
    /// empty.
    ///
    /// Note: this does **not** auto-advance to the next street when the
    /// current street's betting has closed — callers that care about that
    /// should combine this with [`Self::is_street_closed`]. The subgame
    /// tree builder advances the street explicitly at chance nodes.
    pub fn current_street(&self) -> Street {
        self.entries
            .last()
            .map(|(s, _)| *s)
            .unwrap_or(Street::Preflop)
    }

    /// Total chips put in the pot on `street` by each player.
    ///
    /// Returned as `(sb_contribution, bb_contribution)`, i.e.
    /// `(hero_contribution, villain_contribution)` under the v0.1 HU
    /// convention where Hero is the SB.
    ///
    /// On [`Street::Preflop`] this includes the blind posts (0.5 bb for
    /// SB, 1 bb for BB), scaled to chips: `sb_blind = 1`, `bb_blind = 2`
    /// in the standard "1/2" chip representation used throughout the
    /// solver. The actual chip values for blinds are baked in here; the
    /// caller shouldn't try to post blinds as actions.
    ///
    /// For postflop streets, only the actions on that street count.
    ///
    /// `Action::AllIn` is treated as a call of whatever the opponent has
    /// put in on the street (i.e., the common case of "shove over the top"
    /// is `Raise(stack_total)` — AllIn is used specifically for the
    /// "call-for-less" / "shove when facing nothing" degenerate shapes,
    /// which the subgame builder will constrain).
    pub fn pot_contributions_on(&self, street: Street) -> (u32, u32) {
        // For preflop, seed with blind posts. SB puts 1, BB puts 2.
        let (mut sb, mut bb) = match street {
            Street::Preflop => (SB_BLIND, BB_BLIND),
            _ => (0, 0),
        };

        // On preflop, SB acts first, so the first voluntary actor is SB.
        // On postflop, BB acts first. This matches HU convention.
        let sb_opens = matches!(street, Street::Preflop);
        let mut actor_is_sb = sb_opens;

        for (s, action) in self.entries.iter() {
            if *s != street {
                continue;
            }
            let (my, their) = if actor_is_sb {
                (&mut sb, &mut bb)
            } else {
                (&mut bb, &mut sb)
            };
            match action {
                Action::Fold | Action::Check => {
                    // No chips change hands.
                }
                Action::Call => {
                    // Match opponent's current street contribution.
                    *my = *their;
                }
                Action::Bet(amt) | Action::Raise(amt) => {
                    // Bet/raise TO that total on this street.
                    *my = *amt;
                }
                Action::AllIn => {
                    // Without stack info, treat as "at least match" the
                    // opponent. The subgame builder will replace this with
                    // a concrete Raise(stack) once it knows stack sizes.
                    if *my < *their {
                        *my = *their;
                    }
                }
            }
            actor_is_sb = !actor_is_sb;
        }
        (sb, bb)
    }

    /// Has the current street's betting round closed?
    ///
    /// A street is closed when both players have acted and no player has
    /// an outstanding decision. The canonical closing shapes:
    ///
    /// - **check–check** (postflop, neither player bet)
    /// - **bet–call**
    /// - **bet–fold** (entire hand ends, but the street is trivially closed)
    /// - **bet–raise–call**
    /// - any longer raising chain ending in a non-raise response (call / fold)
    ///
    /// A street is **not** closed when:
    ///
    /// - no action yet (betting hasn't started)
    /// - only one player has acted (the other still owes a response)
    /// - the last action was a bet or raise (the non-aggressor owes a response)
    ///
    /// Preflop has a special wrinkle: the BB's post is a "live" bet, so
    /// if preflop has `Call` (SB limps) as the only action, BB still has
    /// option — not closed. A `Check` by BB in that state closes.
    pub fn is_street_closed(&self) -> bool {
        let street = self.current_street();
        let actions: SmallVec<[Action; 8]> = self.iter_street(street).collect();

        if actions.is_empty() {
            return false;
        }

        // A fold always ends the hand; for street-closure purposes it also
        // closes the street.
        if actions.iter().any(|a| matches!(a, Action::Fold)) {
            return true;
        }

        let last = *actions.last().unwrap();

        // If the last action re-opened betting (bet or raise), street is
        // open — opponent owes a response.
        if matches!(last, Action::Bet(_) | Action::Raise(_)) {
            return false;
        }

        // Last action is Check, Call, or AllIn.
        match street {
            Street::Preflop => {
                // Preflop opens with SB acting into a live BB blind. The
                // first player is effectively "facing" 1 bb already.
                //
                // Sequences and closure:
                //   [Call]              — SB limps, BB has option, NOT closed
                //   [Call, Check]       — BB checks option, closed
                //   [Call, Raise, Call] — closed (standard limp-raise-call)
                //   [Raise, Call]       — closed (open-call)
                //   [Raise, Raise, Call]— closed
                //   [Raise, Fold]       — closed (handled above by fold check)
                //
                // So preflop closure requires both players to have acted
                // in a context where the last action was not an aggression.
                // That's equivalent to: either (a) len >= 2 and last is
                // Call/Check/AllIn, or (b) len == 1 and that action is a
                // Check (which can't happen preflop — first actor faces a
                // bet).
                actions.len() >= 2
            }
            _ => {
                // Postflop streets open with both players unbet. First
                // actor's options are Check or Bet. Second actor responds.
                // Shapes:
                //   [Check]              — first actor checked, NOT closed
                //   [Check, Check]       — closed
                //   [Check, Bet, Call]   — closed
                //   [Bet, Call]          — closed
                //   [Bet, Raise, Call]   — closed
                //
                // After a non-aggressive last action, closure requires
                // that we've seen at least one round-trip — both players
                // have acted at the most recent bet level.
                //
                // Concretely, since we already returned early on a
                // trailing Bet/Raise, a trailing Call after any aggression
                // closes, and a trailing Check only closes if the prior
                // action was also a Check (check-check).
                match last {
                    Action::Check => {
                        // Check-check closes; single check does not.
                        actions.len() >= 2 && matches!(actions[actions.len() - 2], Action::Check)
                    }
                    Action::Call | Action::AllIn => {
                        // Call after Bet/Raise closes. If there was no
                        // Bet/Raise yet on this street, a Call shouldn't
                        // have been legal — trust the caller and return
                        // true.
                        true
                    }
                    Action::Bet(_) | Action::Raise(_) | Action::Fold => {
                        // Unreachable — handled above.
                        unreachable!("trailing bet/raise/fold handled earlier");
                    }
                }
            }
        }
    }

    /// Who acts next on the current street, assuming the player who opens
    /// the street is Hero.
    ///
    /// With the default assumption that Hero opens, the `n`th voluntary
    /// actor on a street is Hero if `n` is even and Villain if `n` is
    /// odd. If the street has closed the helper still returns who *would*
    /// act next if the street continued — callers should guard with
    /// [`Self::is_street_closed`] first.
    ///
    /// In HU NLHE, Hero (SB) opens preflop and Villain (BB) opens
    /// postflop — so the subgame builder will either trust this directly
    /// (preflop) or flip the result (postflop). The helper is kept simple
    /// and stateless to keep `ActionLog` free of positional awareness.
    pub fn to_act(&self) -> Player {
        let street = self.current_street();
        let n = self.iter_street(street).count();
        if n % 2 == 0 {
            Player::Hero
        } else {
            Player::Villain
        }
    }
}

/// The small blind's forced post on preflop, in chips.
///
/// We denominate the solver's internal unit in half-bb "chips," so
/// SB = 1, BB = 2. This keeps pot/stack math integer-only on hot paths.
pub const SB_BLIND: u32 = 1;

/// The big blind's forced post on preflop, in chips.
pub const BB_BLIND: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_log() {
        let log = ActionLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert_eq!(log.current_street(), Street::Preflop);
        // With no actions, the street has not closed — preflop is "live"
        // against the BB.
        assert!(!log.is_street_closed());
        // First actor is Hero.
        assert_eq!(log.to_act(), Player::Hero);
    }

    #[test]
    fn preflop_limp_does_not_close() {
        // SB calls 1 more to match the BB. BB still has option.
        let mut log = ActionLog::new();
        log.push(Street::Preflop, Action::Call);
        assert_eq!(log.len(), 1);
        assert_eq!(log.current_street(), Street::Preflop);
        assert!(!log.is_street_closed());
        // Next to act (ignoring positional flipping at this level) is
        // Villain.
        assert_eq!(log.to_act(), Player::Villain);
    }

    #[test]
    fn preflop_limp_check_closes() {
        let mut log = ActionLog::new();
        log.push(Street::Preflop, Action::Call);
        log.push(Street::Preflop, Action::Check);
        assert!(log.is_street_closed());
    }

    #[test]
    fn postflop_check_check_closes() {
        let mut log = ActionLog::new();
        log.push(Street::Flop, Action::Check);
        assert!(!log.is_street_closed(), "single check should not close");
        log.push(Street::Flop, Action::Check);
        assert!(log.is_street_closed(), "check-check should close");
    }

    #[test]
    fn postflop_bet_call_closes() {
        let mut log = ActionLog::new();
        log.push(Street::Flop, Action::Bet(50));
        assert!(!log.is_street_closed());
        log.push(Street::Flop, Action::Call);
        assert!(log.is_street_closed());
    }

    #[test]
    fn postflop_bet_raise_reopens() {
        let mut log = ActionLog::new();
        log.push(Street::Flop, Action::Bet(50));
        log.push(Street::Flop, Action::Raise(150));
        assert!(
            !log.is_street_closed(),
            "raiser gets action back — street not closed"
        );
        log.push(Street::Flop, Action::Call);
        assert!(log.is_street_closed(), "bet-raise-call closes");
    }

    #[test]
    fn fold_closes_the_street() {
        let mut log = ActionLog::new();
        log.push(Street::Flop, Action::Bet(50));
        log.push(Street::Flop, Action::Fold);
        assert!(log.is_street_closed());
    }

    #[test]
    fn iter_preserves_order() {
        let mut log = ActionLog::new();
        log.push(Street::Preflop, Action::Raise(6));
        log.push(Street::Preflop, Action::Call);
        log.push(Street::Flop, Action::Check);
        log.push(Street::Flop, Action::Bet(10));
        let got: Vec<_> = log.iter().collect();
        assert_eq!(
            got,
            vec![
                (Street::Preflop, Action::Raise(6)),
                (Street::Preflop, Action::Call),
                (Street::Flop, Action::Check),
                (Street::Flop, Action::Bet(10)),
            ]
        );
    }

    #[test]
    fn iter_street_filters() {
        let mut log = ActionLog::new();
        log.push(Street::Preflop, Action::Raise(6));
        log.push(Street::Preflop, Action::Call);
        log.push(Street::Flop, Action::Check);
        log.push(Street::Flop, Action::Bet(10));
        log.push(Street::Flop, Action::Call);
        log.push(Street::Turn, Action::Check);

        let preflop: Vec<_> = log.iter_street(Street::Preflop).collect();
        assert_eq!(preflop, vec![Action::Raise(6), Action::Call]);

        let flop: Vec<_> = log.iter_street(Street::Flop).collect();
        assert_eq!(flop, vec![Action::Check, Action::Bet(10), Action::Call]);

        let turn: Vec<_> = log.iter_street(Street::Turn).collect();
        assert_eq!(turn, vec![Action::Check]);

        let river: Vec<_> = log.iter_street(Street::River).collect();
        assert_eq!(river, vec![]);
    }

    #[test]
    fn current_street_follows_last_entry() {
        let mut log = ActionLog::new();
        log.push(Street::Preflop, Action::Call);
        log.push(Street::Preflop, Action::Check);
        assert_eq!(log.current_street(), Street::Preflop);
        log.push(Street::Flop, Action::Check);
        assert_eq!(log.current_street(), Street::Flop);
        log.push(Street::Flop, Action::Bet(5));
        log.push(Street::Flop, Action::Call);
        log.push(Street::Turn, Action::Check);
        assert_eq!(log.current_street(), Street::Turn);
    }

    #[test]
    fn pot_contributions_preflop_limp_check() {
        // SB limps to match BB, BB checks. Both now have 2 in.
        let mut log = ActionLog::new();
        log.push(Street::Preflop, Action::Call);
        log.push(Street::Preflop, Action::Check);
        assert_eq!(log.pot_contributions_on(Street::Preflop), (2, 2));
    }

    #[test]
    fn pot_contributions_preflop_raise_call() {
        // SB raises to 6, BB calls. Both have 6 in.
        let mut log = ActionLog::new();
        log.push(Street::Preflop, Action::Raise(6));
        log.push(Street::Preflop, Action::Call);
        assert_eq!(log.pot_contributions_on(Street::Preflop), (6, 6));
    }

    #[test]
    fn pot_contributions_preflop_three_bet() {
        // SB raises to 6, BB 3-bets to 18, SB calls. Both 18 in.
        let mut log = ActionLog::new();
        log.push(Street::Preflop, Action::Raise(6));
        log.push(Street::Preflop, Action::Raise(18));
        log.push(Street::Preflop, Action::Call);
        assert_eq!(log.pot_contributions_on(Street::Preflop), (18, 18));
    }

    #[test]
    fn pot_contributions_postflop_bet_call() {
        // Postflop: BB acts first here (per HU convention) = non-SB actor.
        // BB bets 30, SB calls. Both 30 in on this street.
        let mut log = ActionLog::new();
        log.push(Street::Flop, Action::Bet(30));
        log.push(Street::Flop, Action::Call);
        assert_eq!(log.pot_contributions_on(Street::Flop), (30, 30));
    }

    #[test]
    fn pot_contributions_postflop_check_bet_call() {
        // BB checks, SB bets 40, BB calls. Both 40 in.
        let mut log = ActionLog::new();
        log.push(Street::Flop, Action::Check);
        log.push(Street::Flop, Action::Bet(40));
        log.push(Street::Flop, Action::Call);
        assert_eq!(log.pot_contributions_on(Street::Flop), (40, 40));
    }

    #[test]
    fn pot_contributions_check_check_empty() {
        let mut log = ActionLog::new();
        log.push(Street::Flop, Action::Check);
        log.push(Street::Flop, Action::Check);
        assert_eq!(log.pot_contributions_on(Street::Flop), (0, 0));
    }

    #[test]
    fn pot_contributions_empty_street_is_zero() {
        let log = ActionLog::new();
        assert_eq!(log.pot_contributions_on(Street::Flop), (0, 0));
        assert_eq!(log.pot_contributions_on(Street::Turn), (0, 0));
        assert_eq!(log.pot_contributions_on(Street::River), (0, 0));
        // Preflop with no actions is just the blinds.
        assert_eq!(log.pot_contributions_on(Street::Preflop), (1, 2));
    }

    #[test]
    fn pop_works() {
        let mut log = ActionLog::new();
        log.push(Street::Preflop, Action::Raise(6));
        log.push(Street::Preflop, Action::Call);
        assert_eq!(log.pop(), Some((Street::Preflop, Action::Call)));
        assert_eq!(log.pop(), Some((Street::Preflop, Action::Raise(6))));
        assert_eq!(log.pop(), None);
    }

    #[test]
    fn to_act_alternates() {
        let mut log = ActionLog::new();
        // Under the "Hero opens" assumption, nth actor alternates.
        assert_eq!(log.to_act(), Player::Hero);
        log.push(Street::Preflop, Action::Raise(6));
        assert_eq!(log.to_act(), Player::Villain);
        log.push(Street::Preflop, Action::Call);
        assert_eq!(log.to_act(), Player::Hero);

        // Moving to a new street resets the count.
        log.push(Street::Flop, Action::Check);
        assert_eq!(log.to_act(), Player::Villain);
    }
}
