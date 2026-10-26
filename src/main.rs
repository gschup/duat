use duat::game::{State, perft};
use duat::solve::{self, Solution};
use std::path::Path;
use std::time::Instant;

const USAGE: &str = "usage: duat perft [depth] | duat solve [table-file] | duat line [table-file]";
const DEFAULT_TABLE: &str = "tables/duat.bin";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("perft") => run_perft(args.get(1).map_or(3, |a| a.parse().expect(USAGE))),
        Some("solve") => run_solve(Path::new(args.get(1).map_or(DEFAULT_TABLE, String::as_str))),
        Some("line") => run_line(Path::new(args.get(1).map_or(DEFAULT_TABLE, String::as_str))),
        _ => eprintln!("{USAGE}"),
    }
}

fn run_perft(max_depth: u32) {
    let start = State::start();
    println!("{start}\n");
    for depth in 1..=max_depth {
        let t = Instant::now();
        let nodes = perft(&start, depth);
        println!("perft({depth}) = {nodes:>14}  ({:.2?})", t.elapsed());
    }
}

fn run_solve(out: &Path) {
    let t = Instant::now();
    let solution = solve::solve(|depth, resolved| {
        if depth % 10 == 1 {
            println!(
                "distance {depth:>4}: {resolved:>11} states  ({:.0?})",
                t.elapsed()
            );
        }
    });
    let stats = solution.stats();
    println!("solved in {:.0?}: {stats:?}", t.elapsed());

    let start = State::start();
    println!("\n{start}\nvalue: {:?}", solution.value(&start));

    let t = Instant::now();
    match solution.verify(10_000_000) {
        Ok(()) => println!("verified 10M sampled states ({:.0?})", t.elapsed()),
        Err(s) => println!("INCONSISTENT state:\n{s}"),
    }

    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).expect("cannot create table directory");
    }
    solution.save(out).expect("cannot write table");
    println!("saved to {}", out.display());
}

/// Plays out perfect play from the start, one turn per printed board.
fn run_line(table: &Path) {
    let solution = Solution::load(table).expect("cannot read table; run `duat solve` first");
    let mut s = State::start();
    println!("{s}\nvalue: {:?}\n", solution.value(&s));
    let mut turn = 1;
    while !s.is_terminal() {
        let mover = s.side_to_move();
        let mut actions = Vec::new();
        while s.side_to_move() == mover && !s.is_terminal() {
            let a = solution.best_action(&s);
            actions.push(a);
            s.apply(a);
        }
        println!("turn {turn} ({mover:?}): {actions:?}\n{s}\n");
        turn += 1;
    }
}
