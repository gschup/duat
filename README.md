# duat

A solver for *Duat*, a two-player abstract game.

- [Duat on BoardGameGeek](https://boardgamegeek.com/boardgame/440609/duat)
- [Unphazed Games](https://unphazedgames.com/) (publisher)

This is an unofficial fan project, not affiliated with or endorsed by Unphazed
Games. Duat and its name belong to its publisher; if you enjoy the game, get a
copy from them.

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
  not cover this case; this is our house rule. Making such pushes illegal
  instead gives exactly the same solution: rotating is always possible, so the
  suicide push is never forced, and it is never better than any alternative.
- The starting player picks a colour; the engine fixes White to move first,
  which loses nothing since the rules are colour-symmetric.
- There is no draw rule (the rulebook suggests restarting if both players agree
  neither can win); the solver treats endless games as draws.

## Result

Duat has about 246 million distinct game states (counting positions that are
mirror images, rotations or colour swaps of each other only once, and including
the states partway through a turn); with perfect play the starting player
always wins, and the best defence only delays the loss until the 15th turn
(the winner's 8th), while the longest forced win from any position takes 61
turns.

Of those states, 153M are wins for the side to move, 83M losses and 10M draws.
Distances count turns of both sides, the current turn included. Among equally
good choices, the solver finishes the turn in the fewest actions, so it never
stalls with three actions where one does the same.

## Usage

```sh
cargo run --release -- solve   # solve all states (~30 s, ~4 GB RAM), write tables/duat.table.gz (~8 MB)
cargo run --release -- line    # replay perfect play from the start using the table
cargo run --release -- perft 3 # move-generator check: turn-level perft counts

cargo run --release --bin play --features gui   # interactive board in a native window
web/build.sh && python3 -m http.server -d dist  # the same board in the browser, at localhost:8000
```

The interactive board starts as human vs human. Hints (best action and
perfect-play outcome) and auto play can be toggled per side at any time;
with both sides on auto it plays out perfect games. Pushes to `main` deploy it
to GitHub Pages via `.github/workflows/pages.yml`, which solves the game in CI.

## Layout

- `src/game.rs`: state (two `u16` bitboards + 2-bit facings per square),
  action encoding (65 dense indices), rules, board symmetries, un-actions,
  turn-level successors, perft.
- `src/index.rs`: index of boards from the mover's perspective, reduced by
  colour and board symmetry (90,202,112 slots).
- `src/solve.rs`: retrograde solver over single-action states (board + actions
  taken this turn), with distance-to-end and consistency checks.
- `src/table.rs`: compact solution (turn-start boards only, 2 bytes each,
  gzipped); mid-turn values come from searching the rest of the turn.
- `src/bin/play.rs`: macroquad board for desktop and wasm; `web/` holds the
  page and the site build script.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. This covers the code in this
repository only, not the game *Duat* itself.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
