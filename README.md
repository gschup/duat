# duat

A solver for *Duat*, a two-player abstract game.

- [Duat on BoardGameGeek](https://boardgamegeek.com/boardgame/440609/duat)
- [Unphazed Games](https://unphazedgames.com/) (publisher)

## Rules (as implemented)

- 4x4 board, three scarabs per player, each facing one of four directions.
  White starts on (0,0) (0,3) (1,1) facing down, Black on (3,0) (3,3) (2,2) facing up.
- A turn is exactly **one or three** actions, any mix, on any of your pieces:
  - **MOVE**: step forward one square into a free square.
  - **ROTATE**: rotate 90° either way.
  - **PUSH**: if the square in front is occupied, slide forward one square,
    shifting the whole contiguous row ahead. Illegal if any pushed piece faces
    the pusher. You may push your own pieces.
  - A move cannot leave the board. Each move or quarter rotation is one action,
    even when repeated on the same piece.
- Pushing an opponent's piece off the board wins immediately.
- Pushing your **own** piece off the board loses immediately. The rulebook does
  not cover this case; this is our house rule.
- The starting player picks a colour; the engine fixes White to move first,
  which loses nothing since the rules are colour-symmetric.
- There is no draw rule (the rulebook suggests restarting if both players agree
  neither can win); the solver treats endless games as draws.
## Layout

- `src/game.rs`: state (two `u16` bitboards + 2-bit facings per square),
  action encoding (65 dense indices), rules, turn-level successors, perft.
- `src/index.rs`: perfect index of turn-start positions (656,015,360 slots)
  from the mover's perspective, for solver tables.

`cargo run --release -- <depth>` prints perft counts from the start position.
