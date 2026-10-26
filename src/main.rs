use duat::game::{State, perft};
use duat::solve;
use duat::table::Table;
use std::path::Path;
use std::time::Instant;

const USAGE: &str = "usage: duat perft [depth] | duat solve [table-file] | duat line [table-file]";
const DEFAULT_TABLE: &str = "tables/duat.table.gz";

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
    let table = solution.table();
    let checks = solution
        .verify(10_000_000)
        .and_then(|()| solution.verify_table(&table, 1_000_000));
    if let Err(s) = checks {
        eprintln!("INCONSISTENT state:\n{s}");
        std::process::exit(1);
    }
    println!(
        "verified 10M sampled states and 1M table lookups ({:.0?})",
        t.elapsed()
    );

    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).expect("cannot create table directory");
    }
    let file = std::fs::File::create(out).expect("cannot create table file");
    table.write_gz(file).expect("cannot write table");
    println!("saved to {}", out.display());
}

/// Plays out perfect play from the start, one turn per printed board.
fn run_line(table: &Path) {
    let bytes = std::fs::read(table).expect("cannot read table; run `duat solve` first");
    let solution = Table::read_gz(&bytes).expect("corrupt table");
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
