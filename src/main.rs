use duat::game::{State, perft};
use std::time::Instant;

fn main() {
    let max_depth: u32 = std::env::args()
        .nth(1)
        .map(|arg| arg.parse().expect("depth must be a number"))
        .unwrap_or(3);
    let start = State::start();
    println!("{start}\n");
    for depth in 1..=max_depth {
        let t = Instant::now();
        let nodes = perft(&start, depth);
        println!("perft({depth}) = {nodes:>14}  ({:.2?})", t.elapsed());
    }
}
