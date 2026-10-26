//! Core game representation: board, pieces, actions and rules.
//!
//! Square `sq` is at row `sq / 4`, column `sq % 4`; row 0 is the top of the board.

use std::fmt;

pub const SQUARES: usize = 16;
/// Pieces per player at the start. Losing any one of them loses the game.
pub const PIECES_PER_SIDE: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Color {
    White,
    Black,
}

impl Color {
    pub fn other(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }

    pub(crate) fn idx(self) -> usize {
        self as usize
    }
}

/// Facing direction ("the sun"). Discriminants are chosen so that rotating
/// clockwise is `+1 mod 4` and the opposite direction is `^ 2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Dir {
    N = 0,
    E = 1,
    S = 2,
    W = 3,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::N, Dir::E, Dir::S, Dir::W];

    pub fn from_bits(bits: u32) -> Dir {
        Dir::ALL[(bits & 3) as usize]
    }

    pub fn cw(self) -> Dir {
        Dir::from_bits(self as u32 + 1)
    }

    pub fn ccw(self) -> Dir {
        Dir::from_bits(self as u32 + 3)
    }

    pub fn opposite(self) -> Dir {
        Dir::from_bits(self as u32 ^ 2)
    }

    fn arrow(self) -> char {
        ['↑', '→', '↓', '←'][self as usize]
    }
}

const NO_SQUARE: u8 = u8::MAX;

const NEIGHBOR: [[u8; 4]; SQUARES] = {
    let mut table = [[NO_SQUARE; 4]; SQUARES];
    let mut sq = 0;
    while sq < SQUARES {
        let (row, col) = (sq / 4, sq % 4);
        if row > 0 {
            table[sq][Dir::N as usize] = (sq - 4) as u8;
        }
        if col < 3 {
            table[sq][Dir::E as usize] = (sq + 1) as u8;
        }
        if row < 3 {
            table[sq][Dir::S as usize] = (sq + 4) as u8;
        }
        if col > 0 {
            table[sq][Dir::W as usize] = (sq - 1) as u8;
        }
        sq += 1;
    }
    table
};

/// The square next to `sq` in direction `d`, or `None` at the board edge.
pub fn neighbor(sq: u8, d: Dir) -> Option<u8> {
    let n = NEIGHBOR[sq as usize][d as usize];
    (n != NO_SQUARE).then_some(n)
}

fn bit(sq: u8) -> u16 {
    1 << sq
}

/// Iterates the squares of a bitboard in ascending order.
pub fn squares(mut bb: u16) -> impl Iterator<Item = u8> {
    std::iter::from_fn(move || {
        (bb != 0).then(|| {
            let sq = bb.trailing_zeros() as u8;
            bb &= bb - 1;
            sq
        })
    })
}

/// Number of symmetries of the square board. Symmetry `g` mirrors the columns
/// if `g & 4` is set, then turns the board `g & 3` quarter turns clockwise.
pub const NUM_SYMS: usize = 8;

const SYM_SQUARE: [[u8; SQUARES]; NUM_SYMS] = {
    let mut table = [[0; SQUARES]; NUM_SYMS];
    let mut g = 0;
    while g < NUM_SYMS {
        let mut sq = 0;
        while sq < SQUARES {
            let (mut row, mut col) = (sq / 4, sq % 4);
            if g & 4 != 0 {
                col = 3 - col;
            }
            let mut turns = 0;
            while turns < g & 3 {
                (row, col) = (col, 3 - row);
                turns += 1;
            }
            table[g][sq] = (row * 4 + col) as u8;
            sq += 1;
        }
        g += 1;
    }
    table
};

fn sym_dir(g: usize, d: Dir) -> Dir {
    let mirrored = if g & 4 != 0 { 4 - d as u32 } else { d as u32 };
    Dir::from_bits(mirrored + (g & 3) as u32)
}

/// Image of a bitboard under symmetry `g`.
pub fn sym_bitboard(g: usize, bb: u16) -> u16 {
    squares(bb).fold(0, |acc, sq| acc | bit(SYM_SQUARE[g][sq as usize]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionKind {
    Move = 0,
    RotateCw = 1,
    RotateCcw = 2,
    Push = 3,
}

/// A single action. Piece actions are encoded as `sq * 4 + kind`, ending the
/// turn as 64, so the dense index doubles as a policy-head slot for a network.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Action(u8);

pub const NUM_ACTIONS: usize = 65;

impl Action {
    /// Ends the turn after a single action. Only legal at `step == 1`.
    pub const END_TURN: Action = Action(64);

    pub fn new(sq: u8, kind: ActionKind) -> Action {
        Action(sq * 4 + kind as u8)
    }

    pub fn from_index(i: usize) -> Action {
        assert!(i < NUM_ACTIONS, "action index {i} out of range");
        Action(i as u8)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// The square and kind of a piece action, `None` for [`Action::END_TURN`].
    pub fn piece_action(self) -> Option<(u8, ActionKind)> {
        if self == Action::END_TURN {
            return None;
        }
        let kind = match self.0 & 3 {
            0 => ActionKind::Move,
            1 => ActionKind::RotateCw,
            2 => ActionKind::RotateCcw,
            _ => ActionKind::Push,
        };
        Some((self.0 >> 2, kind))
    }
}

impl fmt::Debug for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.piece_action() {
            None => write!(f, "EndTurn"),
            Some((sq, kind)) => write!(f, "{kind:?}({},{})", sq / 4, sq % 4),
        }
    }
}

/// A game state, including how far into the current turn the side to move is.
///
/// A turn is exactly one or three actions. `step` counts the actions already
/// taken this turn: at step 1 the player may end the turn, at step 2 they must
/// act again, and the third action ends the turn automatically. Modelling the
/// turn as single actions keeps the branching factor at most 13, which suits
/// MCTS and a fixed-size policy head; [`State::successors`] gives the
/// turn-level view.
///
/// Facing bits of empty squares are always zero, so derived equality and
/// hashing are exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct State {
    pub(crate) pieces: [u16; 2],
    pub(crate) dirs: u32,
    pub(crate) side: Color,
    pub(crate) step: u8,
}

impl State {
    /// The initial setup with White (top, facing down) to move.
    pub fn start() -> State {
        State::from_pieces(
            &[(0, Dir::S), (3, Dir::S), (5, Dir::S)],
            &[(12, Dir::N), (15, Dir::N), (10, Dir::N)],
            Color::White,
        )
    }

    /// Builds a position at the start of `side`'s turn.
    pub fn from_pieces(white: &[(u8, Dir)], black: &[(u8, Dir)], side: Color) -> State {
        let mut s = State {
            pieces: [0; 2],
            dirs: 0,
            side,
            step: 0,
        };
        for (color, list) in [(Color::White, white), (Color::Black, black)] {
            for &(sq, d) in list {
                assert!(s.occupied() & bit(sq) == 0, "square {sq} used twice");
                s.put(sq, color, d);
            }
        }
        s
    }

    pub fn side_to_move(&self) -> Color {
        self.side
    }

    /// Actions already taken in the current turn (0..=2).
    pub fn step(&self) -> u8 {
        self.step
    }

    pub fn pieces(&self, c: Color) -> u16 {
        self.pieces[c.idx()]
    }

    pub fn occupied(&self) -> u16 {
        self.pieces[0] | self.pieces[1]
    }

    pub fn piece_at(&self, sq: u8) -> Option<(Color, Dir)> {
        (self.occupied() & bit(sq) != 0).then(|| (self.owner(sq), self.dir_at(sq)))
    }

    /// The player who still has all their pieces after one was pushed off the
    /// board, if any. The loser is then the side to move, as in chess mate.
    pub fn winner(&self) -> Option<Color> {
        [Color::White, Color::Black]
            .into_iter()
            .find(|c| self.pieces(*c).count_ones() < PIECES_PER_SIDE)
            .map(Color::other)
    }

    pub fn is_terminal(&self) -> bool {
        self.winner().is_some()
    }

    /// Bitmask over action indices; bit `i` is set iff `Action::from_index(i)` is legal.
    pub fn legal_mask(&self) -> u128 {
        if self.is_terminal() {
            return 0;
        }
        let occ = self.occupied();
        let mut mask = 0u128;
        for sq in squares(self.pieces(self.side)) {
            let base = sq as u32 * 4;
            // Rotating is always legal, so a non-terminal state always has a move.
            mask |= 0b0110 << base;
            match neighbor(sq, self.dir_at(sq)) {
                Some(n) if occ & bit(n) == 0 => mask |= 0b0001 << base,
                Some(_) if self.push_len(sq).is_some() => mask |= 0b1000 << base,
                _ => {}
            }
        }
        if self.step == 1 {
            mask |= 1 << Action::END_TURN.index();
        }
        mask
    }

    pub fn legal_actions(&self) -> impl Iterator<Item = Action> {
        let mut mask = self.legal_mask();
        std::iter::from_fn(move || {
            (mask != 0).then(|| {
                let i = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                Action::from_index(i)
            })
        })
    }

    /// Applies a legal action. Pushing any piece off the board ends the game
    /// immediately, leaving the loser to move. Pushing off your own piece loses.
    pub fn apply(&mut self, a: Action) {
        debug_assert!(
            self.legal_mask() & (1 << a.index()) != 0,
            "illegal action {a:?}"
        );
        let Some((sq, kind)) = a.piece_action() else {
            return self.end_turn();
        };
        match kind {
            ActionKind::Move => {
                let (c, d) = self.take(sq);
                self.put(neighbor(sq, d).unwrap(), c, d);
            }
            ActionKind::RotateCw => self.set_dir(sq, self.dir_at(sq).cw()),
            ActionKind::RotateCcw => self.set_dir(sq, self.dir_at(sq).ccw()),
            ActionKind::Push => self.push(sq),
        }
        self.step += 1;
        if let Some(winner) = self.winner() {
            self.side = winner.other();
            self.step = 0;
        } else if self.step == 3 {
            self.end_turn();
        }
    }

    /// All distinct positions reachable in one full turn. Must be called at the
    /// start of a turn.
    pub fn successors(&self) -> Vec<State> {
        debug_assert_eq!(self.step, 0);
        let mut out = Vec::new();
        self.collect_turn_ends(&mut out);
        out.sort_unstable();
        out.dedup();
        out
    }

    fn collect_turn_ends(&self, out: &mut Vec<State>) {
        for a in self.legal_actions() {
            let mut next = *self;
            next.apply(a);
            if next.step == 0 {
                out.push(next);
            } else {
                next.collect_turn_ends(out);
            }
        }
    }

    /// The image of this state under board symmetry `g` (see [`NUM_SYMS`]).
    /// The rules only depend on adjacency and facing, so they commute with it.
    pub fn transformed(&self, g: usize) -> State {
        let mut t = State {
            pieces: [0; 2],
            dirs: 0,
            ..*self
        };
        for sq in squares(self.occupied()) {
            t.put(
                SYM_SQUARE[g][sq as usize],
                self.owner(sq),
                sym_dir(g, self.dir_at(sq)),
            );
        }
        t
    }

    #[cfg(feature = "solver")]
    pub(crate) fn with_turn(mut self, side: Color, step: u8) -> State {
        self.side = side;
        self.step = step;
        self
    }

    /// Calls `f` with every board from which a single piece action of `mover`
    /// produces this board. Side to move and step are left unchanged; the
    /// caller sets them. Actions that push a piece off the board end the game,
    /// so they never lead to a non-terminal board and are not undone here.
    pub fn for_each_unaction(&self, mover: Color, mut f: impl FnMut(State)) {
        let occ = self.occupied();
        for sq in squares(self.pieces(mover)) {
            let d = self.dir_at(sq);
            for prev in [d.ccw(), d.cw()] {
                let mut p = *self;
                p.set_dir(sq, prev);
                f(p);
            }
            let Some(back) = neighbor(sq, d.opposite()) else {
                continue;
            };
            if occ & bit(back) != 0 {
                continue;
            }
            let mut p = *self;
            let (c, pd) = p.take(sq);
            p.put(back, c, pd);
            f(p);
            // Undo pushes of the first k pieces of the row in front, for each k
            // until the row ends or a piece faces the pusher.
            let mut prev_sq = sq;
            let mut cur = neighbor(sq, d);
            while let Some(row_sq) = cur.filter(|&s| occ & bit(s) != 0) {
                if self.dir_at(row_sq) == d.opposite() {
                    break;
                }
                let (c, pd) = p.take(row_sq);
                p.put(prev_sq, c, pd);
                f(p);
                prev_sq = row_sq;
                cur = neighbor(row_sq, d);
            }
        }
    }

    /// Number of pieces pushed by the piece on `sq`, or `None` if it cannot push.
    fn push_len(&self, sq: u8) -> Option<usize> {
        let d = self.dir_at(sq);
        let occ = self.occupied();
        let mut cur = neighbor(sq, d)?;
        if occ & bit(cur) == 0 {
            return None;
        }
        let mut len = 0;
        loop {
            if self.dir_at(cur) == d.opposite() {
                return None; // a piece in the row faces the pusher
            }
            len += 1;
            match neighbor(cur, d) {
                Some(next) if occ & bit(next) != 0 => cur = next,
                Some(_) | None => return Some(len),
            }
        }
    }

    fn push(&mut self, sq: u8) {
        let d = self.dir_at(sq);
        let len = self.push_len(sq).expect("illegal push");
        // The pusher plus at most three pieces fit in a row of four.
        let mut line = [sq; 4];
        for i in 1..=len {
            line[i] = neighbor(line[i - 1], d).unwrap();
        }
        // Shift from the far end so every target square is already empty.
        for &from in line[..=len].iter().rev() {
            let (c, pd) = self.take(from);
            if let Some(to) = neighbor(from, d) {
                self.put(to, c, pd);
            }
        }
    }

    fn end_turn(&mut self) {
        self.side = self.side.other();
        self.step = 0;
    }

    fn owner(&self, sq: u8) -> Color {
        if self.pieces[0] & bit(sq) != 0 {
            Color::White
        } else {
            Color::Black
        }
    }

    pub(crate) fn dir_at(&self, sq: u8) -> Dir {
        Dir::from_bits(self.dirs >> (2 * sq))
    }

    fn set_dir(&mut self, sq: u8, d: Dir) {
        self.dirs = (self.dirs & !(3 << (2 * sq))) | ((d as u32) << (2 * sq));
    }

    pub(crate) fn put(&mut self, sq: u8, c: Color, d: Dir) {
        self.pieces[c.idx()] |= bit(sq);
        self.set_dir(sq, d);
    }

    fn take(&mut self, sq: u8) -> (Color, Dir) {
        let piece = (self.owner(sq), self.dir_at(sq));
        self.pieces[piece.0.idx()] &= !bit(sq);
        self.dirs &= !(3 << (2 * sq));
        piece
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "    0  1  2  3")?;
        for row in 0..4u8 {
            write!(f, "{row} ")?;
            for col in 0..4u8 {
                match self.piece_at(row * 4 + col) {
                    Some((c, d)) => write!(
                        f,
                        " {}{}",
                        if c == Color::White { 'W' } else { 'B' },
                        d.arrow()
                    )?,
                    None => write!(f, "  .")?,
                }
            }
            writeln!(f)?;
        }
        match self.winner() {
            Some(c) => write!(f, "{c:?} wins"),
            None => write!(f, "{:?} to move (step {})", self.side, self.step),
        }
    }
}

/// Counts leaf positions of the full-turn game tree, as a move-generator check.
pub fn perft(s: &State, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    s.successors().iter().map(|n| perft(n, depth - 1)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(sq: u8, kind: ActionKind) -> Action {
        Action::new(sq, kind)
    }

    fn is_legal(s: &State, a: Action) -> bool {
        s.legal_mask() & (1 << a.index()) != 0
    }

    #[test]
    fn start_position_actions() {
        let s = State::start();
        // Every piece can move forward or rotate either way; nothing can push.
        assert_eq!(s.legal_actions().count(), 9);
        assert!(!is_legal(&s, Action::END_TURN));
    }

    #[test]
    fn end_turn_only_after_one_action() {
        let mut s = State::start();
        s.apply(action(0, ActionKind::RotateCw));
        assert!(is_legal(&s, Action::END_TURN));
        s.apply(action(0, ActionKind::RotateCw));
        assert!(!is_legal(&s, Action::END_TURN));
        s.apply(action(0, ActionKind::RotateCw));
        assert_eq!((s.side_to_move(), s.step()), (Color::Black, 0));
    }

    #[test]
    fn move_needs_free_square_on_board() {
        let s = State::from_pieces(
            &[(0, Dir::N), (1, Dir::E), (4, Dir::S)],
            &[(2, Dir::N), (12, Dir::N), (15, Dir::N)],
            Color::White,
        );
        assert!(!is_legal(&s, action(0, ActionKind::Move))); // edge
        assert!(!is_legal(&s, action(1, ActionKind::Move))); // occupied
        assert!(is_legal(&s, action(4, ActionKind::Move)));
    }

    #[test]
    fn push_shifts_whole_row() {
        let mut s = State::from_pieces(
            &[(4, Dir::E), (0, Dir::S), (15, Dir::W)],
            &[(5, Dir::E), (6, Dir::N), (12, Dir::N)],
            Color::White,
        );
        s.apply(action(4, ActionKind::Push));
        assert_eq!(s.piece_at(4), None);
        assert_eq!(s.piece_at(5), Some((Color::White, Dir::E)));
        assert_eq!(s.piece_at(6), Some((Color::Black, Dir::E)));
        assert_eq!(s.piece_at(7), Some((Color::Black, Dir::N)));
        assert!(!s.is_terminal());
    }

    #[test]
    fn push_blocked_by_piece_facing_pusher() {
        let s = State::from_pieces(
            &[(4, Dir::E), (0, Dir::S), (15, Dir::W)],
            &[(5, Dir::E), (6, Dir::W), (12, Dir::N)],
            Color::White,
        );
        assert!(!is_legal(&s, action(4, ActionKind::Push)));
    }

    #[test]
    fn pushing_opponent_off_wins_immediately() {
        let mut s = State::from_pieces(
            &[(4, Dir::E), (0, Dir::S), (15, Dir::W)],
            &[(5, Dir::E), (6, Dir::S), (7, Dir::N)],
            Color::White,
        );
        s.apply(action(4, ActionKind::Push));
        assert_eq!(s.winner(), Some(Color::White));
        assert_eq!(s.side_to_move(), Color::Black);
        assert_eq!(s.legal_mask(), 0);
    }

    #[test]
    fn pushing_own_piece_off_loses() {
        let mut s = State::from_pieces(
            &[(4, Dir::E), (5, Dir::N), (7, Dir::S)],
            &[(6, Dir::S), (12, Dir::N), (15, Dir::N)],
            Color::White,
        );
        s.apply(action(4, ActionKind::Push));
        assert_eq!(s.winner(), Some(Color::Black));
        assert_eq!(s.side_to_move(), Color::White);
        assert_eq!(s.legal_mask(), 0);
    }

    #[test]
    fn can_push_own_pieces() {
        let s = State::from_pieces(
            &[(4, Dir::E), (5, Dir::N), (0, Dir::S)],
            &[(6, Dir::S), (12, Dir::N), (15, Dir::N)],
            Color::White,
        );
        assert!(is_legal(&s, action(4, ActionKind::Push)));
    }

    #[test]
    fn rules_commute_with_symmetries() {
        let s = State::start();
        let succ = s.successors();
        for g in 0..NUM_SYMS {
            let mut mapped: Vec<State> = succ.iter().map(|n| n.transformed(g)).collect();
            mapped.sort_unstable();
            assert_eq!(s.transformed(g).successors(), mapped, "symmetry {g}");
        }
    }

    #[test]
    fn successors_are_distinct_turn_ends() {
        let succ = State::start().successors();
        assert!(succ.iter().all(|s| s.step() == 0));
        // White can only lose on its own turn, by pushing its own piece off.
        assert!(succ.iter().all(|s| match s.winner() {
            None => s.side_to_move() == Color::Black,
            Some(winner) => winner == Color::Black && s.side_to_move() == Color::White,
        }));
        assert!(succ.iter().any(|s| s.winner() == Some(Color::Black)));
        assert!(succ.windows(2).all(|w| w[0] != w[1]));
    }
}
