//! Interactive Duat board: human vs human by default, with perfect-play hints
//! and auto players that can be switched on per side at any time. Runs as a
//! native window or, built for `wasm32-unknown-unknown`, in a browser.

use duat::game::{Action, ActionKind, Color as Side, Dir, State, neighbor};
use duat::table::{Table, Value};
use macroquad::experimental::coroutines::{Coroutine, start_coroutine};
use macroquad::prelude::*;

const TABLE_PATH: &str = "tables/duat.table.gz";
/// Seconds an auto player waits before each action, so games stay watchable.
const AUTO_DELAY: f64 = 0.6;
const SIDES: [Side; 2] = [Side::White, Side::Black];

const BACKGROUND: Color = Color::new(0.07, 0.08, 0.15, 1.0);
const SAND_LIGHT: Color = Color::new(0.87, 0.77, 0.56, 1.0);
const SAND_DARK: Color = Color::new(0.80, 0.68, 0.46, 1.0);
const IVORY: Color = Color::new(0.97, 0.94, 0.86, 1.0);
const EBONY: Color = Color::new(0.13, 0.12, 0.14, 1.0);
const SUN: Color = Color::new(0.98, 0.70, 0.18, 1.0);
const SELECT: Color = Color::new(0.25, 0.75, 0.95, 1.0);
const HINT: Color = Color::new(0.30, 0.85, 0.45, 1.0);
const TEXT: Color = Color::new(0.92, 0.90, 0.85, 1.0);
const MUTED: Color = Color::new(0.55, 0.56, 0.65, 1.0);
const BUTTON: Color = Color::new(0.18, 0.20, 0.32, 1.0);
const BUTTON_ON: Color = Color::new(0.62, 0.48, 0.16, 1.0);

const FONT: f32 = 20.0;
const ROW: f32 = 34.0;

fn window_conf() -> Conf {
    Conf {
        window_title: "Duat".to_owned(),
        window_width: 960,
        window_height: 640,
        ..Default::default()
    }
}

/// The perfect-play table: downloaded in the background, then unpacked.
enum Ai {
    Loading(Coroutine<Result<Vec<u8>, String>>),
    Unpacking(Vec<u8>),
    Ready(Table),
    Failed(String),
}

struct App {
    state: State,
    history: Vec<State>,
    selected: Option<u8>,
    hint: [bool; 2],
    auto: [bool; 2],
    ai: Ai,
    /// When the state last changed, to pace auto players.
    changed_at: f64,
    /// Best action and value for the cached state.
    advice: Option<(State, Action, Value)>,
}

struct Layout {
    board: Rect,
    panel: Rect,
}

impl Layout {
    /// Board left and panel right on wide screens, panel below on tall ones.
    fn compute() -> Layout {
        let (w, h, m) = (screen_width(), screen_height(), 16.0);
        if w >= h * 1.25 {
            let panel_w = (w * 0.34).clamp(280.0, 380.0);
            let size = (h - 2.0 * m).min(w - panel_w - 3.0 * m);
            Layout {
                board: Rect::new(m, (h - size) / 2.0, size, size),
                panel: Rect::new(2.0 * m + size, m, w - size - 3.0 * m, h - 2.0 * m),
            }
        } else {
            let panel_h = 14.0 * ROW;
            let size = (w - 2.0 * m).min(h - panel_h - 3.0 * m).max(120.0);
            Layout {
                board: Rect::new((w - size) / 2.0, m, size, size),
                panel: Rect::new(m, 2.0 * m + size, w - 2.0 * m, panel_h),
            }
        }
    }

    fn cell_size(&self) -> f32 {
        self.board.w / 4.0
    }

    fn cell_rect(&self, sq: u8) -> Rect {
        let c = self.cell_size();
        let (row, col) = ((sq / 4) as f32, (sq % 4) as f32);
        Rect::new(self.board.x + col * c, self.board.y + row * c, c, c)
    }

    fn cell_center(&self, sq: u8) -> Vec2 {
        self.cell_rect(sq).center()
    }

    fn cell_at(&self, p: Vec2) -> Option<u8> {
        self.board.contains(p).then(|| {
            let c = self.cell_size();
            let col = (((p.x - self.board.x) / c) as u8).min(3);
            let row = (((p.y - self.board.y) / c) as u8).min(3);
            row * 4 + col
        })
    }
}

fn side_index(side: Side) -> usize {
    match side {
        Side::White => 0,
        Side::Black => 1,
    }
}

fn is_legal(s: &State, a: Action) -> bool {
    s.legal_mask() & (1 << a.index()) != 0
}

/// Move or push, whichever the piece on `sq` can do.
fn forward_action(s: &State, sq: u8) -> Option<Action> {
    [ActionKind::Move, ActionKind::Push]
        .into_iter()
        .map(|kind| Action::new(sq, kind))
        .find(|&a| is_legal(s, a))
}

fn dir_vec(d: Dir) -> Vec2 {
    match d {
        Dir::N => vec2(0.0, -1.0),
        Dir::E => vec2(1.0, 0.0),
        Dir::S => vec2(0.0, 1.0),
        Dir::W => vec2(-1.0, 0.0),
    }
}

impl App {
    fn new() -> App {
        App {
            state: State::start(),
            history: Vec::new(),
            selected: None,
            hint: [false; 2],
            auto: [false; 2],
            ai: Ai::Loading(start_coroutine(async {
                load_file(TABLE_PATH).await.map_err(|e| format!("{e:?}"))
            })),
            changed_at: get_time(),
            advice: None,
        }
    }

    fn new_game(&mut self) {
        self.state = State::start();
        self.history.clear();
        self.selected = None;
        self.changed_at = get_time();
    }

    fn undo(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.state = prev;
            self.selected = None;
            self.changed_at = get_time();
        }
    }

    fn apply(&mut self, a: Action) {
        let mover = self.state.side_to_move();
        // Keep the acting piece selected, so several actions in a row are easy.
        let follow = a.piece_action().map(|(sq, kind)| match kind {
            ActionKind::Move | ActionKind::Push => {
                let (_, d) = self.state.piece_at(sq).unwrap();
                neighbor(sq, d).unwrap()
            }
            ActionKind::RotateCw | ActionKind::RotateCcw => sq,
        });
        self.history.push(self.state);
        self.state.apply(a);
        let still_moving = self.state.side_to_move() == mover && !self.state.is_terminal();
        self.selected = follow.filter(|_| still_moving);
        self.changed_at = get_time();
    }

    fn mover_is_auto(&self) -> bool {
        self.auto[side_index(self.state.side_to_move())]
    }

    fn advice(&mut self) -> Option<(Action, Value)> {
        let Ai::Ready(table) = &self.ai else {
            return None;
        };
        if self.state.is_terminal() {
            return None;
        }
        if self.advice.map(|(s, ..)| s) != Some(self.state) {
            let s = self.state;
            self.advice = Some((s, table.best_action(&s), table.value(&s)));
        }
        self.advice.map(|(_, a, v)| (a, v))
    }

    /// Advice to display: for the side to move if it has hints or auto on.
    fn shown_advice(&mut self) -> Option<(Action, Value)> {
        let i = side_index(self.state.side_to_move());
        if self.hint[i] || self.auto[i] {
            self.advice()
        } else {
            None
        }
    }

    fn update_ai(&mut self) {
        match &self.ai {
            Ai::Loading(download) => {
                if let Some(result) = download.retrieve() {
                    self.ai = match result {
                        Ok(bytes) => Ai::Unpacking(bytes),
                        Err(e) => Ai::Failed(e),
                    };
                }
            }
            // Unpack one frame after the download finished, so the
            // "unpacking" status gets drawn before the frame blocks.
            Ai::Unpacking(bytes) => {
                self.ai = match Table::read_gz(bytes) {
                    Ok(table) => Ai::Ready(table),
                    Err(e) => Ai::Failed(e.to_string()),
                };
            }
            Ai::Ready(_) | Ai::Failed(_) => {}
        }
    }

    fn run_auto(&mut self) {
        if self.mover_is_auto()
            && get_time() - self.changed_at >= AUTO_DELAY
            && let Some((a, _)) = self.advice()
        {
            self.apply(a);
        }
    }

    fn handle_input(&mut self, layout: &Layout) {
        if is_key_pressed(KeyCode::Backspace) || is_key_pressed(KeyCode::Z) {
            self.undo();
        }
        if is_key_pressed(KeyCode::N) {
            self.new_game();
        }
        if self.state.is_terminal() || self.mover_is_auto() {
            return;
        }
        if is_key_pressed(KeyCode::Enter) && is_legal(&self.state, Action::END_TURN) {
            self.apply(Action::END_TURN);
        }
        if let Some(sq) = self.selected {
            let pressed = [
                (KeyCode::Left, Some(Action::new(sq, ActionKind::RotateCcw))),
                (KeyCode::Right, Some(Action::new(sq, ActionKind::RotateCw))),
                (KeyCode::Up, forward_action(&self.state, sq)),
                (KeyCode::Space, forward_action(&self.state, sq)),
            ];
            if let Some(a) = pressed
                .into_iter()
                .find_map(|(key, a)| a.filter(|_| is_key_pressed(key)))
            {
                return self.apply(a);
            }
        }
        if !is_mouse_button_pressed(MouseButton::Left) {
            return;
        }
        let Some(cell) = layout.cell_at(mouse_position().into()) else {
            return;
        };
        if let Some(sq) = self.selected {
            let (_, d) = self.state.piece_at(sq).unwrap();
            if neighbor(sq, d) == Some(cell)
                && let Some(a) = forward_action(&self.state, sq)
            {
                return self.apply(a);
            }
        }
        let own = matches!(self.state.piece_at(cell), Some((side, _)) if side == self.state.side_to_move());
        self.selected = (own && self.selected != Some(cell)).then_some(cell);
    }

    fn draw_board(&mut self, layout: &Layout) {
        let c = layout.cell_size();
        for sq in 0..16u8 {
            let r = layout.cell_rect(sq);
            let shade = if (sq / 4 + sq % 4) % 2 == 0 {
                SAND_LIGHT
            } else {
                SAND_DARK
            };
            draw_rectangle(r.x, r.y, r.w, r.h, shade);
        }
        let b = layout.board;
        draw_rectangle_lines(b.x - 3.0, b.y - 3.0, b.w + 6.0, b.h + 6.0, 6.0, SUN);

        if let Some(sq) = self.selected {
            outline_cell(layout, sq, SELECT);
            let (_, d) = self.state.piece_at(sq).unwrap();
            if let (Some(target), Some(_)) = (neighbor(sq, d), forward_action(&self.state, sq)) {
                let r = layout.cell_rect(target);
                draw_rectangle(r.x, r.y, r.w, r.h, Color { a: 0.25, ..SELECT });
            }
        }

        let advice = self.shown_advice();
        if let Some((a, _)) = advice
            && let Some((sq, _)) = a.piece_action()
        {
            outline_cell(layout, sq, HINT);
        }

        for sq in 0..16u8 {
            if let Some((side, d)) = self.state.piece_at(sq) {
                draw_beetle(layout.cell_center(sq), c, side, d);
            }
        }

        if let Some((a, _)) = advice
            && let Some((sq, kind)) = a.piece_action()
        {
            let center = layout.cell_center(sq);
            let (_, d) = self.state.piece_at(sq).unwrap();
            match kind {
                ActionKind::Move | ActionKind::Push => draw_arrow(center, dir_vec(d), c),
                ActionKind::RotateCw | ActionKind::RotateCcw => {
                    let new = if kind == ActionKind::RotateCw {
                        d.cw()
                    } else {
                        d.ccw()
                    };
                    let p = center + dir_vec(new) * 0.36 * c;
                    draw_circle(p.x, p.y, 0.11 * c, Color { a: 0.55, ..HINT });
                    draw_circle_lines(p.x, p.y, 0.11 * c, 3.0, HINT);
                }
            }
        }
    }

    fn draw_panel(&mut self, layout: &Layout) {
        let p = layout.panel;
        let mut y = p.y;
        let line = |text: &str, size: f32, color: Color, y: &mut f32| {
            draw_text(text, p.x, *y + size, size, color);
            *y += size + 8.0;
        };

        line("DUAT", 34.0, SUN, &mut y);
        let s = self.state;
        let mover = s.side_to_move();
        match s.winner() {
            Some(w) => line(&format!("{w:?} wins!"), 28.0, SUN, &mut y),
            None => {
                line(&format!("{mover:?} to move"), 24.0, TEXT, &mut y);
                let step = match s.step() {
                    0 => "action 1 (a turn is 1 or 3 actions)",
                    1 => "action 2, or end the turn",
                    _ => "action 3 of 3",
                };
                line(step, FONT, MUTED, &mut y);
            }
        }
        let value = match self.shown_advice().map(|(_, v)| v) {
            Some(Value::Win(d)) => format!("perfect play: {mover:?} wins in {d} turns"),
            Some(Value::Loss(d)) => format!("perfect play: {mover:?} loses in {d} turns"),
            Some(Value::Draw) => "perfect play: draw".to_owned(),
            None => String::new(),
        };
        line(&value, FONT, HINT, &mut y);
        y += 4.0;

        let human_turn = !s.is_terminal() && !self.mover_is_auto();
        let advice = self.shown_advice().map(|(a, _)| a);
        let thirds: [Rect; 3] = row_cells(p, y);
        if let Some(sq) = self.selected.filter(|_| human_turn) {
            let forward = forward_action(&s, sq);
            let label = match forward.and_then(Action::piece_action) {
                Some((_, ActionKind::Push)) => "Push",
                _ => "Move",
            };
            let buttons = [
                (Some(Action::new(sq, ActionKind::RotateCcw)), "Rotate L"),
                (forward, label),
                (Some(Action::new(sq, ActionKind::RotateCw)), "Rotate R"),
            ];
            for (rect, (a, label)) in thirds.into_iter().zip(buttons) {
                if button(rect, label, a.is_some(), false, a.is_some() && a == advice) {
                    return self.apply(a.unwrap());
                }
            }
        } else {
            for (rect, label) in thirds.into_iter().zip(["Rotate L", "Move", "Rotate R"]) {
                button(rect, label, false, false, false);
            }
        }
        y += ROW;
        let end_turn = Action::END_TURN;
        let can_end = human_turn && is_legal(&s, end_turn);
        let [rect] = row_cells(p, y);
        if button(rect, "End turn", can_end, false, advice == Some(end_turn)) {
            return self.apply(end_turn);
        }
        y += ROW + 10.0;

        for (label, flags) in [("Hints", 0), ("Auto", 1)] {
            draw_text(label, p.x, y + 23.0, FONT, TEXT);
            let cells: [Rect; 2] = row_cells(Rect::new(p.x + 70.0, p.y, p.w - 70.0, p.h), y);
            for (rect, side) in cells.into_iter().zip(SIDES) {
                let i = side_index(side);
                let on = if flags == 0 {
                    self.hint[i]
                } else {
                    self.auto[i]
                };
                if button(rect, &format!("{side:?}"), true, on, false) {
                    let flag = if flags == 0 {
                        &mut self.hint[i]
                    } else {
                        &mut self.auto[i]
                    };
                    *flag = !*flag;
                    self.changed_at = get_time();
                }
            }
            y += ROW;
        }
        y += 10.0;

        let [undo, new] = row_cells(p, y);
        if button(undo, "Undo", !self.history.is_empty(), false, false) {
            self.undo();
        }
        if button(new, "New game", true, false, false) {
            self.new_game();
        }
        y += ROW + 8.0;

        let status = match &self.ai {
            Ai::Loading(_) => "AI: downloading perfect-play table...".to_owned(),
            Ai::Unpacking(_) => "AI: unpacking table...".to_owned(),
            Ai::Ready(_) => "AI: ready (perfect play)".to_owned(),
            Ai::Failed(e) => format!("AI unavailable: {e}"),
        };
        line(&status, 16.0, MUTED, &mut y);
        line(
            "Click a beetle, then the square ahead to move or push.",
            16.0,
            MUTED,
            &mut y,
        );
        line(
            "Keys: arrows rotate/move, Enter ends turn, Z undoes.",
            16.0,
            MUTED,
            &mut y,
        );
        y += 8.0;
        line(
            "Unofficial fan project, not affiliated",
            16.0,
            MUTED,
            &mut y,
        );
        line(
            "with Unphazed Games (unphazedgames.com).",
            16.0,
            MUTED,
            &mut y,
        );
    }
}

/// Splits a panel row into `N` equally wide button rectangles.
fn row_cells<const N: usize>(panel: Rect, y: f32) -> [Rect; N] {
    let gap = 8.0;
    let w = (panel.w - gap * (N as f32 - 1.0)) / N as f32;
    std::array::from_fn(|i| Rect::new(panel.x + i as f32 * (w + gap), y, w, ROW - 6.0))
}

/// Draws a button and returns whether it was clicked.
fn button(r: Rect, label: &str, enabled: bool, on: bool, hinted: bool) -> bool {
    let hovered = enabled && r.contains(mouse_position().into());
    let fill = match (on, hovered) {
        (true, _) => BUTTON_ON,
        (false, true) => Color {
            a: 1.0,
            ..lerp(BUTTON, TEXT, 0.15)
        },
        (false, false) => BUTTON,
    };
    draw_rectangle(r.x, r.y, r.w, r.h, fill);
    if hinted {
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 3.0, HINT);
    }
    let color = if enabled { TEXT } else { MUTED };
    let dims = measure_text(label, None, FONT as u16, 1.0);
    draw_text(
        label,
        r.x + (r.w - dims.width) / 2.0,
        r.y + (r.h + dims.offset_y) / 2.0,
        FONT,
        color,
    );
    hovered && is_mouse_button_pressed(MouseButton::Left)
}

fn lerp(a: Color, b: Color, t: f32) -> Color {
    Color::new(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

fn outline_cell(layout: &Layout, sq: u8, color: Color) {
    let r = layout.cell_rect(sq);
    draw_rectangle_lines(r.x + 2.0, r.y + 2.0, r.w - 4.0, r.h - 4.0, 5.0, color);
}

/// A scarab seen from above, pushing its sun in the facing direction.
fn draw_beetle(center: Vec2, cell: f32, side: Side, d: Dir) {
    let f = dir_vec(d);
    let r = f.perp();
    let (body, edge) = match side {
        Side::White => (IVORY, EBONY),
        Side::Black => (EBONY, IVORY),
    };
    let leg = 0.02 * cell;
    for k in [-1.0, 0.0, 1.0] {
        let base = center + f * (k * 0.13 * cell - 0.04 * cell);
        for s in [-1.0, 1.0] {
            let tip = base + r * (s * 0.34 * cell) + f * (k * 0.08 * cell);
            draw_line(base.x, base.y, tip.x, tip.y, leg * 1.6, edge);
        }
    }
    let back = center - f * 0.05 * cell;
    draw_circle(back.x, back.y, 0.25 * cell, edge);
    draw_circle(back.x, back.y, 0.23 * cell, body);
    let head = center + f * 0.2 * cell;
    draw_circle(head.x, head.y, 0.1 * cell, edge);
    draw_circle(head.x, head.y, 0.085 * cell, body);
    // The split between the wing cases.
    let (a, b) = (center + f * 0.12 * cell, center - f * 0.27 * cell);
    draw_line(a.x, a.y, b.x, b.y, leg, edge);
    let sun = center + f * 0.36 * cell;
    draw_circle(sun.x, sun.y, 0.11 * cell, SUN);
    draw_circle_lines(sun.x, sun.y, 0.11 * cell, 2.0, edge);
}

fn draw_arrow(center: Vec2, f: Vec2, cell: f32) {
    let (start, tip) = (center + f * 0.5 * cell, center + f * 0.9 * cell);
    let thick = 0.06 * cell;
    draw_line(start.x, start.y, tip.x, tip.y, thick, HINT);
    let side = f.perp() * 0.1 * cell;
    let head = tip + f * 0.1 * cell;
    draw_triangle(head, tip + side, tip - side, HINT);
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut app = App::new();
    loop {
        app.update_ai();
        let layout = Layout::compute();
        app.handle_input(&layout);
        app.run_auto();
        clear_background(BACKGROUND);
        app.draw_board(&layout);
        app.draw_panel(&layout);
        next_frame().await;
    }
}
