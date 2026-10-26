//! Retrograde solver over single-action states.
//!
//! A table slot is `step * NUM_POSITIONS + index(board)`, so every state of a
//! turn, not just turn starts, gets an exact value. Values are from the point
//! of view of the side to move, and distances count turns until the game ends
//! with best play, the current turn being turn 1: the winner wins as fast as
//! possible, the loser holds out as long as possible.
//!
//! The solver first counts each state's distinct child slots and marks the
//! states that can push an opponent off right away. It then works backwards
//! level by level: a resolved state makes each parent a win if it is good for
//! that parent's mover, and otherwise counts down the parent's remaining
//! children, making it a loss when none are left. An action within a turn
//! keeps the distance, so each level first resolves the earlier steps of the
//! same turns and only then the turns before. States never resolved are
//! draws: neither side can force a win.

use std::sync::atomic::{AtomicU8, AtomicU16, Ordering::Relaxed};

use rayon::prelude::*;

use crate::game::{Color, State};
use crate::index::{NUM_POSITIONS, from_index, index, is_canonical};
use crate::table::{Table, Value};

/// Actions already taken in a turn: 0, 1 or 2.
const STEPS: u32 = 3;
pub const TABLE_SIZE: usize = (STEPS * NUM_POSITIONS) as usize;

// An entry's top two bits hold its status. While unresolved they are zero and
// the entry counts the children not yet known to be bad for the mover.
const STATUS: u8 = 0xC0;
const WIN: u8 = 0x40;
const LOSS: u8 = 0x80;
const UNUSED: u8 = 0xC0;

#[derive(Debug, Default)]
pub struct Stats {
    pub wins: u64,
    pub losses: u64,
    pub draws: u64,
    pub max_dist: u16,
}

pub struct Solution {
    entries: Vec<u8>,
    dist: Vec<u16>,
}

pub fn slot(s: &State) -> u32 {
    s.step() as u32 * NUM_POSITIONS + index(s)
}

/// Spreads sample number `n` pseudo-randomly over the table.
fn sample_slot(n: u64) -> u32 {
    ((n.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 20) % TABLE_SIZE as u64) as u32
}

fn state_at(slot: u32) -> State {
    from_index(slot % NUM_POSITIONS).with_turn(Color::White, (slot / NUM_POSITIONS) as u8)
}

/// Collects the distinct child slots of a non-terminal state. Returns true
/// instead if some action pushes an opponent piece off the board.
fn child_slots(s: &State, out: &mut Vec<u32>) -> bool {
    out.clear();
    for a in s.legal_actions() {
        let mut c = *s;
        c.apply(a);
        match c.winner() {
            Some(w) if w == s.side_to_move() => return true,
            // Pushing your own piece off loses at once, so it is never better
            // than any other option and need not be counted.
            Some(_) => {}
            None => out.push(slot(&c)),
        }
    }
    out.sort_unstable();
    out.dedup();
    false
}

/// Collects the distinct parent slots of a state, each with whether the
/// parent's mover is also the child's mover (the turn is still going on).
fn parent_slots(c: &State, out: &mut Vec<(u32, bool)>) {
    out.clear();
    let same_mover = c.step() != 0;
    let mover = if same_mover {
        c.side_to_move()
    } else {
        // A turn just ended, either by choice after one action...
        let mover = c.side_to_move().other();
        out.push((slot(&c.with_turn(mover, 1)), false));
        mover
    };
    // ...or by the action that led here: step 1 and 2 follow steps 0 and 1,
    // and a turn start follows a third action taken at step 2.
    let prev_step = (c.step() + 2) % 3;
    c.for_each_unaction(mover, |p| {
        out.push((slot(&p.with_turn(mover, prev_step)), same_mover))
    });
    out.sort_unstable();
    out.dedup();
}

/// Records one resolved child of `entry` that is good or bad for its mover.
/// Returns true if this resolves the entry.
fn update(entry: &AtomicU8, good: bool) -> bool {
    let mut cur = entry.load(Relaxed);
    loop {
        if cur & STATUS != 0 {
            return false;
        }
        let new = match (good, cur) {
            (true, _) => WIN,
            (false, 1) => LOSS,
            (false, n) => n - 1,
        };
        match entry.compare_exchange_weak(cur, new, Relaxed, Relaxed) {
            Ok(_) => return new & STATUS != 0,
            Err(actual) => cur = actual,
        }
    }
}

/// Solves every state. `progress` is called with each finished distance and
/// the number of states resolved at it.
pub fn solve(progress: impl Fn(u16, usize) + Sync) -> Solution {
    let entries: Vec<AtomicU8> = (0..TABLE_SIZE)
        .into_par_iter()
        .map(|_| AtomicU8::new(0))
        .collect();
    let dist: Vec<AtomicU16> = (0..TABLE_SIZE)
        .into_par_iter()
        .map(|_| AtomicU16::new(0))
        .collect();

    let mut frontier: Vec<u32> = (0..TABLE_SIZE as u32)
        .into_par_iter()
        .map_init(Vec::new, |children, i| {
            let entry = &entries[i as usize];
            if !is_canonical(i % NUM_POSITIONS) {
                entry.store(UNUSED, Relaxed);
                None
            } else if child_slots(&state_at(i), children) {
                entry.store(WIN, Relaxed);
                dist[i as usize].store(1, Relaxed);
                Some(i)
            } else {
                entry.store(children.len() as u8, Relaxed);
                None
            }
        })
        .flatten()
        .collect();

    // Expands resolved states into the parents they resolve, at `depth`.
    let expand = |children: &[u32], depth: u16| -> Vec<u32> {
        children
            .par_iter()
            .fold(
                || (Vec::new(), Vec::new()),
                |(mut parents, mut resolved), &child| {
                    let child_wins = entries[child as usize].load(Relaxed) & STATUS == WIN;
                    parent_slots(&state_at(child), &mut parents);
                    for &(p, same_mover) in &parents {
                        if update(&entries[p as usize], child_wins == same_mover) {
                            dist[p as usize].store(depth, Relaxed);
                            resolved.push(p);
                        }
                    }
                    (parents, resolved)
                },
            )
            .map(|(_, resolved)| resolved)
            .reduce(Vec::new, |mut a, b| {
                a.extend(b);
                a
            })
    };
    let is_turn_start = |i: &u32| *i < NUM_POSITIONS;

    let mut depth = 1;
    while !frontier.is_empty() {
        // Mid-turn states resolve the earlier steps of their turn at the same
        // distance (steps only go down, so this ends within two rounds)...
        let mut new: Vec<u32> = frontier.clone();
        loop {
            let mid_turn: Vec<u32> = new.into_iter().filter(|i| !is_turn_start(i)).collect();
            new = expand(&mid_turn, depth);
            if new.is_empty() {
                break;
            }
            frontier.extend_from_slice(&new);
        }
        progress(depth, frontier.len());
        // ...and then turn starts resolve the turns before, one turn farther.
        let turn_starts: Vec<u32> = frontier.into_iter().filter(is_turn_start).collect();
        frontier = expand(&turn_starts, depth + 1);
        depth += 1;
    }

    Solution {
        entries: entries.into_iter().map(AtomicU8::into_inner).collect(),
        dist: dist.into_iter().map(AtomicU16::into_inner).collect(),
    }
}

impl Solution {
    /// The value of a non-terminal state for its side to move.
    pub fn value(&self, s: &State) -> Value {
        self.value_at(slot(s) as usize)
    }

    fn value_at(&self, i: usize) -> Value {
        match self.entries[i] & STATUS {
            WIN => Value::Win(self.dist[i]),
            LOSS => Value::Loss(self.dist[i]),
            _ => Value::Draw,
        }
    }

    /// The compact table of turn-start values. Unused slots read as draws.
    pub fn table(&self) -> Table {
        Table::from_entries(
            (0..NUM_POSITIONS as usize)
                .into_par_iter()
                .map(|i| self.value_at(i).encode())
                .collect(),
        )
    }

    pub fn stats(&self) -> Stats {
        self.entries
            .par_iter()
            .zip(&self.dist)
            .fold(Stats::default, |mut st, (&e, &d)| {
                match e & STATUS {
                    WIN => st.wins += 1,
                    LOSS => st.losses += 1,
                    UNUSED => {}
                    _ => st.draws += 1,
                }
                st.max_dist = st.max_dist.max(d);
                st
            })
            .reduce(Stats::default, |a, b| Stats {
                wins: a.wins + b.wins,
                losses: a.losses + b.losses,
                draws: a.draws + b.draws,
                max_dist: a.max_dist.max(b.max_dist),
            })
    }

    /// Checks `samples` pseudo-random states against their children: a win
    /// must have a good child at its distance and none closer, a loss only bad
    /// children with the farthest at its distance, and a draw no good child
    /// but some drawn one. A child in the next turn is one turn closer.
    /// Returns the first inconsistent state.
    pub fn verify(&self, samples: u64) -> Result<(), State> {
        (0..samples)
            .into_par_iter()
            .try_for_each_init(Vec::new, |children, n| {
                let i = sample_slot(n);
                if self.entries[i as usize] & STATUS == UNUSED {
                    return Ok(());
                }
                let s = state_at(i);
                let consistent = if child_slots(&s, children) {
                    self.value(&s) == Value::Win(1)
                } else {
                    let mut best_win = None::<u16>;
                    let mut worst_loss = 0;
                    let mut any_draw = false;
                    for a in s.legal_actions() {
                        let mut c = s;
                        c.apply(a);
                        if c.is_terminal() {
                            continue;
                        }
                        let same_mover = c.side_to_move() == s.side_to_move();
                        let turn = !same_mover as u16;
                        match (self.value(&c), same_mover) {
                            (Value::Win(d), true) | (Value::Loss(d), false) => {
                                let d = d + turn;
                                best_win = Some(best_win.map_or(d, |b| b.min(d)));
                            }
                            (Value::Loss(d), true) | (Value::Win(d), false) => {
                                worst_loss = worst_loss.max(d + turn);
                            }
                            (Value::Draw, _) => any_draw = true,
                        }
                    }
                    match (self.value(&s), best_win) {
                        (Value::Win(d), Some(b)) => d == b,
                        (Value::Loss(d), None) => !any_draw && d == worst_loss,
                        (Value::Draw, None) => any_draw,
                        _ => false,
                    }
                };
                if consistent { Ok(()) } else { Err(s) }
            })
    }

    /// Checks that `table` reproduces this solution's values, including in
    /// the middle of turns, on `samples` pseudo-random states.
    pub fn verify_table(&self, table: &Table, samples: u64) -> Result<(), State> {
        (0..samples).into_par_iter().try_for_each(|n| {
            let i = sample_slot(n);
            if self.entries[i as usize] & STATUS == UNUSED {
                return Ok(());
            }
            let s = state_at(i);
            if table.value(&s) == self.value(&s) {
                Ok(())
            } else {
                Err(s)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// States reachable from the start within two turns, at every step.
    fn sample_states() -> Vec<State> {
        let mut states = Vec::new();
        let mut layer = vec![State::start()];
        for _ in 0..4 {
            let mut next = Vec::new();
            for s in &layer {
                for a in s.legal_actions() {
                    let mut c = *s;
                    c.apply(a);
                    if !c.is_terminal() {
                        next.push(c);
                    }
                }
            }
            next.sort_unstable();
            next.dedup();
            states.extend_from_slice(&next);
            layer = next;
        }
        states
    }

    #[test]
    fn parents_and_children_agree() {
        let (mut children, mut parents) = (Vec::new(), Vec::new());
        for s in sample_states() {
            let wins_now = child_slots(&s, &mut children);
            for &c in children.iter().filter(|_| !wins_now) {
                // The canonical child may be a symmetric image of the actual
                // one, so look for the parent's slot among its parents.
                parent_slots(&state_at(c), &mut parents);
                assert!(
                    parents.iter().any(|&(p, _)| p == slot(&s)),
                    "missing parent\n{s}"
                );
            }
            parent_slots(&s, &mut parents);
            for &(p, _) in &parents {
                // A parent that wins outright has its children cut short.
                let wins_now = child_slots(&state_at(p), &mut children);
                assert!(
                    wins_now || children.contains(&slot(&s)),
                    "missing child\n{s}"
                );
            }
        }
    }
}
