//! Compact solution: two bytes per turn-start board, small enough to ship to
//! a browser. Values in the middle of a turn are recomputed by searching the
//! rest of the turn, which is at most two more actions deep.

use std::cmp::Reverse;
use std::io::{self, Read, Write};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};

use crate::game::{Action, State};
use crate::index::{NUM_POSITIONS, index};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    /// The side to move wins in this many turns, the current one being turn 1
    /// (turns of both sides count).
    Win(u16),
    /// The side to move loses in this many turns.
    Loss(u16),
    Draw,
}

const LOSS_FLAG: u16 = 0x8000;

impl Value {
    /// Encoding: 0 for a draw, `d` for a win in `d`, `0x8000 | d` for a loss in `d`.
    pub fn encode(self) -> u16 {
        match self {
            Value::Win(d) => d,
            Value::Loss(d) => LOSS_FLAG | d,
            Value::Draw => 0,
        }
    }

    fn decode(e: u16) -> Value {
        match e {
            0 => Value::Draw,
            _ if e & LOSS_FLAG != 0 => Value::Loss(e & !LOSS_FLAG),
            _ => Value::Win(e),
        }
    }

    /// How much the side to move likes this value: faster wins and slower
    /// losses are better.
    fn score(self) -> i32 {
        match self {
            Value::Win(d) => 100_000 - d as i32,
            Value::Draw => 0,
            Value::Loss(d) => d as i32 - 100_000,
        }
    }
}

pub struct Table {
    entries: Vec<u16>,
}

impl Table {
    pub fn from_entries(entries: Vec<u16>) -> Table {
        assert_eq!(entries.len(), NUM_POSITIONS as usize, "wrong table size");
        Table { entries }
    }

    /// Writes the entries as gzipped little-endian `u16`s.
    pub fn write_gz(&self, w: impl Write) -> io::Result<()> {
        let mut encoder = GzEncoder::new(w, Compression::best());
        for chunk in self.entries.chunks(1 << 16) {
            let bytes: Vec<u8> = chunk.iter().flat_map(|e| e.to_le_bytes()).collect();
            encoder.write_all(&bytes)?;
        }
        encoder.finish()?.flush()
    }

    /// Reads a table written by [`Table::write_gz`], decoding in chunks so
    /// the decompressed bytes are never held in memory twice.
    pub fn read_gz(compressed: &[u8]) -> io::Result<Table> {
        let mut decoder = GzDecoder::new(compressed);
        let mut entries = Vec::with_capacity(NUM_POSITIONS as usize);
        let mut buf = vec![0; 1 << 17];
        while entries.len() < NUM_POSITIONS as usize {
            let n = buf.len().min(2 * (NUM_POSITIONS as usize - entries.len()));
            decoder.read_exact(&mut buf[..n])?;
            let (pairs, _) = buf[..n].as_chunks::<2>();
            entries.extend(pairs.iter().map(|&p| u16::from_le_bytes(p)));
        }
        Ok(Table::from_entries(entries))
    }

    /// The value of a non-terminal state for its side to move.
    pub fn value(&self, s: &State) -> Value {
        if s.step() == 0 {
            return Value::decode(self.entries[index(s) as usize]);
        }
        self.search(s).0
    }

    /// The value of taking action `a` in `s`, for the side to move in `s`.
    pub fn value_after(&self, s: &State, a: Action) -> Value {
        self.after(s, a).0
    }

    /// An optimal action: the fastest win, the slowest loss, or one keeping a
    /// draw. Among equally good actions, it takes the one finishing the turn
    /// in the fewest actions, so a single rotation beats three the other way.
    pub fn best_action(&self, s: &State) -> Action {
        s.legal_actions()
            .max_by_key(|&a| preference(self.after(s, a)))
            .expect("no legal action")
    }

    /// The value of a non-terminal state and the fewest actions that finish
    /// the current turn while keeping that value.
    fn search(&self, s: &State) -> (Value, u8) {
        s.legal_actions()
            .map(|a| self.after(s, a))
            .max_by_key(|&o| preference(o))
            .expect("no legal action")
    }

    /// Like [`Table::search`], for the state after action `a` in `s`.
    fn after(&self, s: &State, a: Action) -> (Value, u8) {
        let mut c = *s;
        c.apply(a);
        if let Some(winner) = c.winner() {
            let v = if winner == s.side_to_move() {
                Value::Win(1)
            } else {
                Value::Loss(1)
            };
            return (v, 1);
        }
        if c.side_to_move() == s.side_to_move() {
            let (v, n) = self.search(&c);
            return (v, n + 1);
        }
        let v = match self.value(&c) {
            Value::Win(d) => Value::Loss(d + 1),
            Value::Loss(d) => Value::Win(d + 1),
            Value::Draw => Value::Draw,
        };
        (v, 1)
    }
}

/// Better values first, then shorter turns.
fn preference((v, n): (Value, u8)) -> (i32, Reverse<u8>) {
    (v.score(), Reverse(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_roundtrip() {
        for v in [
            Value::Draw,
            Value::Win(1),
            Value::Win(300),
            Value::Loss(1),
            Value::Loss(300),
        ] {
            assert_eq!(Value::decode(v.encode()), v);
        }
    }
}
