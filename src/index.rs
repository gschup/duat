//! Index of non-terminal boards, reduced by colour and board symmetry, for
//! addressing solver tables.
//!
//! Boards are seen from the side to move ("me" vs "opponent"). The rules treat
//! both colours alike, so a board and its colour-swapped twin share one index,
//! and the rules commute with the 8 symmetries of the square, so symmetric
//! boards share one too.
//!
//! Layout: `orbit(me) * REST + rank(opp) * 4096 + facings`. `orbit(me)` numbers
//! the 77 symmetry classes of my three squares; the board is first transformed
//! so my squares are the class representative. The opponent's squares are
//! ranked among the 13 squares I leave free, and the facings are 2 bits per
//! piece, mine first, in square order. When several symmetries map my squares
//! to the representative, the smallest resulting index is used, so a few slots
//! are never used; see [`is_canonical`].

use std::sync::LazyLock;

use crate::game::{Color, Dir, NUM_SYMS, SQUARES, State, squares, sym_bitboard};

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
const ME_SETS: usize = BINOM[16][3] as usize;
/// Symmetry classes of my three squares.
pub const ORBITS: u32 = 77;
/// Ways to place the opponent's three pieces on the remaining squares.
pub const OPP_SETS: u32 = BINOM[13][3];
const REST: u32 = OPP_SETS * 4096;
pub const NUM_POSITIONS: u32 = ORBITS * REST;

struct Orbits {
    /// Per colex rank of my squares: its orbit number and a bitmask of the
    /// symmetries mapping it to the orbit representative.
    of_set: [(u32, u8); ME_SETS],
    representative: Vec<u16>,
}

static ORBIT_TABLE: LazyLock<Orbits> = LazyLock::new(|| {
    let mut of_set = [(0, 0); ME_SETS];
    let mut orbit_of_rep = [u32::MAX; ME_SETS];
    let mut representative = Vec::new();
    for (r, set_orbit) in of_set.iter_mut().enumerate() {
        let set = unrank3(r as u32);
        let images: [u16; NUM_SYMS] = std::array::from_fn(|g| sym_bitboard(g, set));
        let rep = *images.iter().min().unwrap();
        let syms = (0..NUM_SYMS)
            .filter(|&g| images[g] == rep)
            .fold(0, |acc, g| acc | 1 << g);
        let orbit = &mut orbit_of_rep[rank3(rep) as usize];
        if *orbit == u32::MAX {
            *orbit = representative.len() as u32;
            representative.push(rep);
        }
        *set_orbit = (*orbit, syms);
    }
    assert_eq!(representative.len() as u32, ORBITS);
    Orbits {
        of_set,
        representative,
    }
});

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

/// The board's index within its orbit class: opponent squares and facings.
fn rest_index(s: &State) -> u32 {
    let me = s.pieces(s.side_to_move());
    let opp = s.pieces(s.side_to_move().other());
    let facings = squares(me)
        .chain(squares(opp))
        .fold(0, |acc, sq| acc << 2 | s.dir_at(sq) as u32);
    rank3(compress(opp, me)) * 4096 + facings
}

/// The index of the board, ignoring how far into the turn the state is.
pub fn index(s: &State) -> u32 {
    debug_assert!(!s.is_terminal(), "only non-terminal boards are indexed");
    let (orbit, syms) = ORBIT_TABLE.of_set[rank3(s.pieces(s.side_to_move())) as usize];
    let rest = (0..NUM_SYMS)
        .filter(|g| syms & (1 << g) != 0)
        .map(|g| rest_index(&s.transformed(g)))
        .min()
        .unwrap();
    orbit * REST + rest
}

/// The representative board for `i`, with White to move at step 0.
pub fn from_index(i: u32) -> State {
    assert!(i < NUM_POSITIONS, "index {i} out of range");
    let (orbit, rest) = (i / REST, i % REST);
    let me = ORBIT_TABLE.representative[orbit as usize];
    let opp = expand(unrank3(rest / 4096), me);
    let facings = rest % 4096;
    let mut s = State::from_pieces(&[], &[], Color::White);
    let colored = squares(me)
        .map(|sq| (sq, Color::White))
        .chain(squares(opp).map(|sq| (sq, Color::Black)));
    for (j, (sq, c)) in colored.enumerate() {
        s.put(sq, c, Dir::from_bits(facings >> (2 * (5 - j))));
    }
    s
}

/// Whether slot `i` is the index of some board. Slots that are not are the
/// non-minimal variants of boards whose own squares are symmetric.
pub fn is_canonical(i: u32) -> bool {
    index(&from_index(i)) == i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_size() {
        assert_eq!(ORBIT_TABLE.representative.len(), 77);
        assert_eq!(NUM_POSITIONS, 90_202_112);
    }

    #[test]
    fn roundtrip_on_canonical_slots() {
        let mut canonical = 0;
        for i in (0..NUM_POSITIONS).step_by(997) {
            if is_canonical(i) {
                canonical += 1;
                assert_eq!(index(&from_index(i)), i);
            } else {
                // A non-canonical slot still decodes to a board of a smaller slot.
                assert!(index(&from_index(i)) < i);
            }
        }
        assert!(canonical > 0);
    }

    #[test]
    fn symmetric_and_colour_swapped_boards_share_index() {
        for s in State::start()
            .successors()
            .into_iter()
            .filter(|s| !s.is_terminal())
        {
            let i = index(&s);
            for g in 0..NUM_SYMS {
                assert_eq!(index(&s.transformed(g)), i);
            }
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
            assert_eq!(index(&swapped), i);
        }
    }
}
