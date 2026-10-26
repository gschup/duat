//! Retrograde solver over single-action states.
//!
//! A table slot is `step * NUM_POSITIONS + index(board)`, so every state of a
//! turn, not just turn starts, gets an exact value. Values are from the point
//! of view of the side to move, and distances count single actions (including
//! ending the turn) until the game ends with best play: the winner wins as
//! fast as possible, the loser holds out as long as possible.
//!
//! The solver first counts each state's distinct child slots and marks the
//! states that can push an opponent off right away. It then works backwards
//! level by level: a resolved state makes each parent a win if it is good for
//! that parent's mover, and otherwise counts down the parent's remaining
//! children, making it a loss when none are left. States never resolved are
//! draws: neither side can force a win.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU8, AtomicU16, Ordering::Relaxed};

use rayon::prelude::*;

use crate::game::{Action, Color, State};
use crate::index::{NUM_POSITIONS, from_index, index, is_canonical};

/// Actions already taken in a turn: 0, 1 or 2.
const STEPS: u32 = 3;
pub const TABLE_SIZE: usize = (STEPS * NUM_POSITIONS) as usize;

// An entry's top two bits hold its status. While unresolved they are zero and
// the entry counts the children not yet known to be bad for the mover.
const STATUS: u8 = 0xC0;
const WIN: u8 = 0x40;
const LOSS: u8 = 0x80;
const UNUSED: u8 = 0xC0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    /// The side to move wins in this many actions.
    Win(u16),
    /// The side to move loses in this many actions.
    Loss(u16),
    Draw,
}

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

    let mut depth = 1;
    while !frontier.is_empty() {
        progress(depth, frontier.len());
        frontier = frontier
            .par_iter()
            .fold(
                || (Vec::new(), Vec::new()),
                |(mut parents, mut resolved), &child| {
                    let child_wins = entries[child as usize].load(Relaxed) & STATUS == WIN;
                    parent_slots(&state_at(child), &mut parents);
                    for &(p, same_mover) in &parents {
                        if update(&entries[p as usize], child_wins == same_mover) {
                            dist[p as usize].store(depth + 1, Relaxed);
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
            });
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
        let i = slot(s) as usize;
        match self.entries[i] & STATUS {
            WIN => Value::Win(self.dist[i]),
            LOSS => Value::Loss(self.dist[i]),
            _ => Value::Draw,
        }
    }

    /// An optimal action: the fastest win, the slowest loss, or one keeping a draw.
    pub fn best_action(&self, s: &State) -> Action {
        let score = |a: Action| {
            let mut c = *s;
            c.apply(a);
            if let Some(winner) = c.winner() {
                return if winner == s.side_to_move() {
                    i32::MAX
                } else {
                    i32::MIN
                };
            }
            let sign = if c.side_to_move() == s.side_to_move() {
                1
            } else {
                -1
            };
            match self.value(&c) {
                Value::Win(d) => sign * (i32::from(u16::MAX) + 1 - i32::from(d)),
                Value::Loss(d) => -sign * (i32::from(u16::MAX) + 1 - i32::from(d)),
                Value::Draw => 0,
            }
        };
        s.legal_actions()
            .max_by_key(|&a| score(a))
            .expect("no legal action")
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
    /// must have a good child one action closer and none closer, a loss only
    /// bad children with the farthest one action closer, and a draw no good
    /// child but some drawn one. Returns the first inconsistent state.
    pub fn verify(&self, samples: u64) -> Result<(), State> {
        (0..samples)
            .into_par_iter()
            .try_for_each_init(Vec::new, |children, n| {
                // A multiplicative hash spreads the samples over the table.
                let i = (n.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 20) % TABLE_SIZE as u64;
                if self.entries[i as usize] & STATUS == UNUSED {
                    return Ok(());
                }
                let s = state_at(i as u32);
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
                        match (self.value(&c), same_mover) {
                            (Value::Win(d), true) | (Value::Loss(d), false) => {
                                best_win = Some(best_win.map_or(d, |b| b.min(d)));
                            }
                            (Value::Loss(d), true) | (Value::Win(d), false) => {
                                worst_loss = worst_loss.max(d);
                            }
                            (Value::Draw, _) => any_draw = true,
                        }
                    }
                    match (self.value(&s), best_win) {
                        (Value::Win(d), Some(b)) => d == b + 1,
                        (Value::Loss(d), None) => !any_draw && d == worst_loss + 1,
                        (Value::Draw, None) => any_draw,
                        _ => false,
                    }
                };
                if consistent { Ok(()) } else { Err(s) }
            })
    }

    /// Writes the raw tables: one status byte per slot, then one
    /// little-endian `u16` distance per slot.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut w = BufWriter::new(File::create(path)?);
        w.write_all(&self.entries)?;
        for d in &self.dist {
            w.write_all(&d.to_le_bytes())?;
        }
        w.flush()
    }

    pub fn load(path: &Path) -> io::Result<Solution> {
        let mut r = BufReader::new(File::open(path)?);
        let mut entries = vec![0; TABLE_SIZE];
        r.read_exact(&mut entries)?;
        let mut raw = vec![0; 2 * TABLE_SIZE];
        r.read_exact(&mut raw)?;
        let dist = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| u16::from_le_bytes(b))
            .collect();
        Ok(Solution { entries, dist })
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
