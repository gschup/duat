//! Perfect index for non-terminal positions at the start of a turn, for
//! addressing solver tables.
//!
//! Positions are seen from the side to move ("me" vs "opponent"). The rules
//! treat both colours alike, so a position and its colour-swapped twin share
//! one index. Layout: `(rank(me) * OPP_SETS + rank(opp)) * 4096 + facings`,
//! where the opponent's squares are ranked among the 13 squares I leave free
//! and the facings are 2 bits per piece, mine first, in square order.

use crate::game::{Color, Dir, SQUARES, State, squares};

const BINOM: [[u32; 4]; SQUARES + 1] = {
    let mut t = [[0; 4]; SQUARES + 1];
    let mut n = 0;
    while n <= SQUARES {
        t[n][0] = 1;
        let mut k = 1;
        while k < 4 {
            t[n][k] = if n == 0 {
                0
            } else {
                t[n - 1][k - 1] + t[n - 1][k]
            };
            k += 1;
        }
        n += 1;
    }
    t
};

/// Ways to place my three pieces.
pub const ME_SETS: u32 = BINOM[16][3];
/// Ways to place the opponent's three pieces on the remaining squares.
pub const OPP_SETS: u32 = BINOM[13][3];
pub const NUM_POSITIONS: u32 = ME_SETS * OPP_SETS * 4096;

/// Colex rank of a 3-element set given as a bitmask.
fn rank3(set: u16) -> u32 {
    squares(set)
        .enumerate()
        .map(|(k, sq)| BINOM[sq as usize][k + 1])
        .sum()
}

fn unrank3(mut r: u32) -> u16 {
    let mut set = 0;
    for k in (1..=3).rev() {
        let mut x = k - 1;
        while BINOM[x + 1][k] <= r {
            x += 1;
        }
        r -= BINOM[x][k];
        set |= 1 << x;
    }
    set
}

/// Renumbers `set` to positions among the squares not in `taken`.
fn compress(set: u16, taken: u16) -> u16 {
    squares(set)
        .map(|sq| 1 << (sq - (taken & ((1 << sq) - 1)).count_ones() as u8))
        .fold(0, |acc, b| acc | b)
}

fn expand(set: u16, taken: u16) -> u16 {
    squares(!taken)
        .enumerate()
        .filter(|&(i, _)| set & (1 << i) != 0)
        .fold(0, |acc, (_, sq)| acc | 1 << sq)
}

pub fn index(s: &State) -> u32 {
    debug_assert!(
        s.step() == 0 && !s.is_terminal(),
        "only turn-start positions are indexed"
    );
    let me = s.pieces(s.side_to_move());
    let opp = s.pieces(s.side_to_move().other());
    let sets = rank3(me) * OPP_SETS + rank3(compress(opp, me));
    let facings = squares(me)
        .chain(squares(opp))
        .fold(0, |acc, sq| acc << 2 | s.dir_at(sq) as u32);
    sets * 4096 + facings
}

/// The position for `i`, with White as the side to move.
pub fn from_index(i: u32) -> State {
    assert!(i < NUM_POSITIONS, "index {i} out of range");
    let (sets, facings) = (i / 4096, i % 4096);
    let me = unrank3(sets / OPP_SETS);
    let opp = expand(unrank3(sets % OPP_SETS), me);
    let mut s = State::from_pieces(&[], &[], Color::White);
    let colored = squares(me)
        .map(|sq| (sq, Color::White))
        .chain(squares(opp).map(|sq| (sq, Color::Black)));
    for (j, (sq, c)) in colored.enumerate() {
        s.put(sq, c, Dir::from_bits(facings >> (2 * (5 - j))));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_size() {
        assert_eq!(NUM_POSITIONS, 656_015_360);
    }

    #[test]
    fn roundtrip() {
        for i in (0..NUM_POSITIONS).step_by(9973) {
            assert_eq!(index(&from_index(i)), i);
        }
        assert_eq!(index(&from_index(NUM_POSITIONS - 1)), NUM_POSITIONS - 1);
    }

    #[test]
    fn colour_swap_shares_index() {
        for s in State::start()
            .successors()
            .into_iter()
            .filter(|s| !s.is_terminal())
        {
            let piece_list = |c| {
                squares(s.pieces(c))
                    .map(|sq| (sq, s.dir_at(sq)))
                    .collect::<Vec<_>>()
            };
            let swapped = State::from_pieces(
                &piece_list(Color::Black),
                &piece_list(Color::White),
                Color::White,
            );
            assert_eq!(index(&s), index(&swapped));
            assert_eq!(from_index(index(&s)), swapped);
        }
    }
}
