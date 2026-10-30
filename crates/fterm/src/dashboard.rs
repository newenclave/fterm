//! The dashboard (Phase 12): tiles in a grid over the window, drawn as scene commands. A tile shows a pane, the
//! output of a command, buttons, or a tile from the API. Pure: the app gives the tiles and draws the commands.

use std::time::{Duration, Instant};

use fterm_config::load::{PaneFilter, PaneMatch};
use fterm_scene::{Op, Rgb};
use unicode_width::UnicodeWidthChar;

/// The activity chart: how much output a pane had in each of the last `BUCKETS` spans of `BUCKET`.
pub const BUCKET: Duration = Duration::from_secs(10);
pub const BUCKETS: usize = 30;

/// Output events of one pane, counted in spans of `BUCKET` (the last 5 minutes).
#[derive(Clone, Debug, Default)]
pub struct Activity {
    /// (the start of the span, the count), the oldest first.
    spans: std::collections::VecDeque<(Instant, u32)>,
}

impl Activity {
    /// The pane had output at `now`.
    pub fn note(&mut self, now: Instant) {
        match self.spans.back_mut() {
            Some((start, n)) if now.saturating_duration_since(*start) < BUCKET => *n += 1,
            _ => self.spans.push_back((now, 1)),
        }
        while self.spans.len() > BUCKETS {
            self.spans.pop_front();
        }
    }

    /// The counts of the last `BUCKETS` spans up to `now`, the oldest first (spans with no output are 0).
    pub fn values(&self, now: Instant) -> Vec<f64> {
        let mut out = vec![0.0; BUCKETS];
        for (start, n) in &self.spans {
            let age = now.saturating_duration_since(*start).as_secs() / BUCKET.as_secs();
            if let Some(i) = (BUCKETS as u64 - 1).checked_sub(age) {
                out[i as usize] += f64::from(*n);
            }
        }
        out
    }
}

/// A line of a list tile: the line and its fields (between tabs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub line: String,
    pub fields: Vec<String>,
}

impl Item {
    pub fn parse(line: &str) -> Self {
        Self {
            line: line.to_owned(),
            fields: line.split('\t').map(str::to_owned).collect(),
        }
    }

    /// The text to show: field `show` (from 1), else the whole line.
    pub fn shown(&self, show: Option<usize>) -> &str {
        show.and_then(|n| n.checked_sub(1))
            .and_then(|i| self.fields.get(i))
            .filter(|_| self.fields.len() > 1)
            .map_or(self.line.as_str(), String::as_str)
    }
}

/// `template` with `{1}`, `{2}`, ... (the fields of `item`; a missing one is empty) and `{line}`. Other braces stay.
pub fn fill(template: &str, item: &Item) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                if name == "line" {
                    out.push_str(&item.line);
                } else if let Ok(n) = name.parse::<usize>() {
                    if let Some(field) = n.checked_sub(1).and_then(|i| item.fields.get(i)) {
                        out.push_str(field);
                    }
                } else {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The arguments of a program with the fields filled in: each one stays one argument (no shell splits them).
pub fn fill_args(args: &[String], item: &Item) -> Vec<String> {
    args.iter().map(|a| fill(a, item)).collect()
}

/// The first `http://` or `https://` link in a text.
pub fn first_url(text: &str) -> Option<&str> {
    let start = text.find("https://").or_else(|| text.find("http://"))?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']'))
        .unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches(['.', ',', ';', ':']);
    (url.len() > "https://".len()).then_some(url)
}

/// What the filter of a `panes` tile needs to know about a pane.
#[derive(Clone, Debug, Default)]
pub struct PaneFacts<'a> {
    pub agent: bool,
    /// A command runs in it now.
    pub running: bool,
    /// Its last command failed.
    pub failed: bool,
    pub program: &'a str,
    pub title: &'a str,
    pub workspace: &'a str,
    pub profile: &'a str,
}

/// The pane goes into a `panes` tile with this filter.
pub fn pane_matches(filter: &PaneFilter, pane: &PaneFacts<'_>) -> bool {
    match filter {
        PaneFilter::All => true,
        PaneFilter::Agents => pane.agent,
        PaneFilter::Busy => pane.running || pane.failed,
        PaneFilter::Match(m) => matches(m, pane),
    }
}

fn matches(m: &PaneMatch, pane: &PaneFacts<'_>) -> bool {
    let has = |want: &Option<String>, text: &str| {
        want.as_ref()
            .is_none_or(|w| text.to_lowercase().contains(w.as_str()))
    };
    has(&m.program, pane.program)
        && has(&m.title, pane.title)
        && has(&m.workspace, pane.workspace)
        && has(&m.profile, pane.profile)
}

/// What a tile tells with the color of its frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TileState {
    #[default]
    Idle,
    Working,
    Waiting,
    Done,
    Error,
}

impl TileState {
    /// From the name of an agent state or of the API (`working`, `waiting`, `done`, `error`; else idle).
    pub fn from_name(name: &str) -> Self {
        match name {
            "working" => Self::Working,
            "waiting" => Self::Waiting,
            "done" | "ok" | "success" => Self::Done,
            "error" | "failed" => Self::Error,
            _ => Self::Idle,
        }
    }
}

/// One tile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tile {
    pub title: String,
    /// A short text at the right of the title (`working 4m`, `exit 1`).
    pub status: String,
    pub state: TileState,
    /// The lines to show (the last ones are kept when they do not fit).
    pub lines: Vec<String>,
    /// A chart at the bottom (activity, or values from the API); empty = none.
    pub values: Vec<f64>,
    /// The labels of its buttons.
    pub buttons: Vec<String>,
    /// A list: the lines from the top (not the last ones), and the chosen one.
    pub from_top: bool,
    pub cursor: Option<usize>,
    /// The keys of the chosen item: (key, label), shown at the bottom as `[a] approve`.
    pub keys: Vec<(String, String)>,
}

/// The colors of the dashboard.
#[derive(Clone, Copy, Debug)]
pub struct Colors {
    pub text: Rgb,
    pub dim: Rgb,
    /// Idle, working, waiting, done, error.
    pub states: [Rgb; 5],
}

impl Colors {
    fn state(&self, state: TileState) -> Rgb {
        self.states[state as usize]
    }
}

/// A rect in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRect {
    pub col: usize,
    pub row: usize,
    pub cols: usize,
    pub rows: usize,
}

impl CellRect {
    pub fn contains(&self, col: usize, row: usize) -> bool {
        (self.col..self.col + self.cols).contains(&col)
            && (self.row..self.row + self.rows).contains(&row)
    }
}

/// Where the tiles and the buttons are (for clicks).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    pub tiles: Vec<CellRect>,
    /// (tile, button, its place).
    pub buttons: Vec<(usize, usize, CellRect)>,
    /// (tile, item key, its place).
    pub keys: Vec<(usize, usize, CellRect)>,
}

/// The key of tile `i`: `1` to `9`; later tiles have none (arrows and Enter reach them).
pub fn tile_key(i: usize) -> Option<char> {
    (i < 9).then(|| char::from(b'1' + i as u8))
}

/// The key of the `n`th button of the dashboard (in tile order): `a` to `z`.
pub fn button_key(n: usize) -> Option<char> {
    (n < 26).then(|| char::from(b'a' + n as u8))
}

/// The tile and the button of the button key `key`.
pub fn button_of_key(tiles: &[Tile], key: char) -> Option<(usize, usize)> {
    tiles
        .iter()
        .enumerate()
        .flat_map(|(t, tile)| (0..tile.buttons.len()).map(move |b| (t, b)))
        .zip(0..)
        .find(|(_, n)| button_key(*n) == Some(key))
        .map(|(tb, _)| tb)
}

/// How many columns and rows of tiles: `columns` from the config (0 = choose). A good tile is about 2.5 times as
/// wide as high in cells (a cell is about twice as high as wide, so it looks a little wide).
pub fn grid(n: usize, cols: usize, rows: usize, columns: usize) -> (usize, usize) {
    if n == 0 {
        return (1, 1);
    }
    let pick = if columns > 0 {
        columns.min(n)
    } else {
        (1..=n)
            .min_by(|&a, &b| {
                let score = |c: usize| {
                    let r = n.div_ceil(c);
                    let w = cols as f64 / c as f64;
                    let h = (rows as f64 / r as f64).max(1.0);
                    ((w / h) / 2.5).ln().abs()
                };
                score(a).total_cmp(&score(b))
            })
            .unwrap_or(1)
    };
    (pick, n.div_ceil(pick))
}

/// `text` cut to `width` cells (an ellipsis when it was longer). A tab is a space and other control chars go: they
/// take a cell on the screen but no width here, so they pushed the text over the frame.
fn cut(text: &str, width: usize) -> String {
    let text: String = text
        .chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    let text = text.as_str();
    let mut out = String::new();
    let mut used = 0;
    let total: usize = text.chars().map(|c| c.width().unwrap_or(0)).sum();
    if total <= width {
        return text.to_owned();
    }
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

fn width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

fn color(rgb: Rgb) -> Op {
    Op::Color {
        color: Some(rgb.hex()),
    }
}

fn text(col: usize, row: usize, text: String) -> Op {
    Op::Text {
        col: col as i32,
        row: row as i32,
        text,
    }
}

/// The scene commands for the dashboard in a scene of `cols` × `rows` cells, and the places of tiles and buttons.
/// `header` is the first row (the name and the keys); `selected` has a double frame.
pub fn draw(
    tiles: &[Tile],
    cols: usize,
    rows: usize,
    columns: usize,
    selected: Option<usize>,
    header: &str,
    colors: &Colors,
) -> (Vec<Op>, Places) {
    let mut ops = vec![
        Op::Clear,
        color(colors.dim),
        text(1, 0, cut(header, cols.saturating_sub(2))),
    ];
    let mut places = Places::default();
    if tiles.is_empty() || cols < 8 || rows < 4 {
        return (ops, places);
    }
    let (gc, gr) = grid(tiles.len(), cols, rows - 1, columns);
    let tile_cols = cols / gc;
    let tile_rows = (rows - 1) / gr;
    let mut button_n = 0;
    for (i, tile) in tiles.iter().enumerate() {
        let place = CellRect {
            col: (i % gc) * tile_cols,
            row: 1 + (i / gc) * tile_rows,
            cols: tile_cols,
            rows: tile_rows,
        };
        places.tiles.push(place);
        if tile_cols < 6 || tile_rows < 3 {
            continue;
        }
        let frame = colors.state(tile.state);
        let (x, y) = ((place.col * 2) as i32, (place.row * 4) as i32);
        let (w, h) = ((place.cols * 2) as i32 - 2, (place.rows * 4) as i32 - 2);
        ops.push(color(frame));
        ops.push(Op::Rect {
            x,
            y,
            w,
            h,
            fill: false,
        });
        if selected == Some(i) {
            // One dot inside the frame: it stays in the cells of the frame, off the text and the keys.
            ops.push(Op::Rect {
                x: x + 1,
                y: y + 1,
                w: w - 2,
                h: h - 2,
                fill: false,
            });
        }
        // The title row: the key, the title, and the status at the right.
        let inner = place.cols.saturating_sub(4);
        let status = cut(&tile.status, inner / 2);
        let key = tile_key(i).map(|k| format!("[{k}] ")).unwrap_or_default();
        let title = cut(
            &format!("{key}{}", tile.title),
            inner.saturating_sub(width(&status) + 1),
        );
        ops.push(text(place.col + 2, place.row + 1, title));
        if !status.is_empty() {
            ops.push(color(colors.dim));
            ops.push(text(
                place.col + 2 + inner - width(&status),
                place.row + 1,
                status,
            ));
        }
        // The rows inside: the lines, then the chart, then the buttons.
        let first = place.row + 3;
        let last = place.row + place.rows - 2;
        let mut bottom = last;
        if !tile.buttons.is_empty() && bottom >= first {
            let mut col = place.col + 2;
            ops.push(color(colors.text));
            for (b, label) in tile.buttons.iter().enumerate() {
                let key = button_key(button_n)
                    .map(|k| format!("[{k}] "))
                    .unwrap_or_default();
                button_n += 1;
                let shown = format!("{key}{label}");
                let w = width(&shown);
                if col + w > place.col + 2 + inner {
                    break;
                }
                places.buttons.push((
                    i,
                    b,
                    CellRect {
                        col,
                        row: bottom,
                        cols: w,
                        rows: 1,
                    },
                ));
                ops.push(text(col, bottom, shown));
                col += w + 2;
            }
            bottom = bottom.saturating_sub(2);
        }
        if !tile.keys.is_empty() && bottom >= first {
            let mut col = place.col + 2;
            ops.push(color(colors.dim));
            for (k, (key, label)) in tile.keys.iter().enumerate() {
                let shown = format!("[{key}] {label}");
                let w = width(&shown);
                if col + w > place.col + 2 + inner {
                    break;
                }
                places.keys.push((
                    i,
                    k,
                    CellRect {
                        col,
                        row: bottom,
                        cols: w,
                        rows: 1,
                    },
                ));
                ops.push(text(col, bottom, shown));
                col += w + 2;
            }
            bottom = bottom.saturating_sub(2);
        }
        let room = (bottom + 1).saturating_sub(first);
        // The chart takes about a third of the room, when there is room for lines too.
        if !tile.values.is_empty() && room >= 6 {
            let chart = room / 3;
            let top = bottom + 1 - chart;
            ops.push(color(frame));
            ops.push(Op::Plot {
                values: tile.values.clone(),
                min: Some(0.0),
                max: None,
                bars: false,
                x: Some(x + 4),
                y: Some((top * 4) as i32),
                w: Some(w - 8),
                h: Some((chart * 4) as i32 - 2),
            });
            bottom = top.saturating_sub(2);
        }
        let room = (bottom + 1).saturating_sub(first);
        if room > 0 {
            ops.push(color(colors.text));
            // A list from the top, scrolled so the chosen item is seen; else the last lines.
            let start = if tile.from_top {
                match tile.cursor {
                    Some(c) if c >= room => c + 1 - room,
                    _ => 0,
                }
            } else {
                tile.lines.len().saturating_sub(room)
            };
            let end = (start + room).min(tile.lines.len());
            for (n, line) in tile.lines[start.min(end)..end].iter().enumerate() {
                let row = first + n;
                match tile.cursor {
                    Some(c) if c == start + n => {
                        ops.push(color(frame));
                        ops.push(text(place.col + 2, row, cut(&format!("› {line}"), inner)));
                        ops.push(color(colors.text));
                    }
                    Some(_) => ops.push(text(place.col + 2, row, cut(&format!("  {line}"), inner))),
                    None => ops.push(text(place.col + 2, row, cut(line, inner))),
                }
            }
        }
    }
    (ops, places)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fterm_scene::Scene;

    const COLORS: Colors = Colors {
        text: Rgb::new(200, 200, 200),
        dim: Rgb::new(120, 120, 120),
        states: [
            Rgb::new(90, 90, 90),
            Rgb::new(0, 0, 255),
            Rgb::new(255, 255, 0),
            Rgb::new(0, 255, 0),
            Rgb::new(255, 0, 0),
        ],
    };

    /// The text rows of the drawn dashboard.
    fn rows_text(tiles: &[Tile], cols: usize, rows: usize) -> Vec<String> {
        let (ops, _) = draw(tiles, cols, rows, 0, None, "Dashboard", &COLORS);
        let mut scene = Scene::new(cols, rows);
        scene.draw(&ops).unwrap();
        scene.canvas().rows_text()
    }

    fn tile(title: &str) -> Tile {
        Tile {
            title: title.into(),
            ..Default::default()
        }
    }

    #[test]
    fn activity_in_spans_of_ten_seconds() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut a = Activity::default();
        a.note(s(0));
        a.note(s(5));
        a.note(s(12));
        let v = a.values(s(12));
        assert_eq!(v.len(), BUCKETS);
        assert_eq!(&v[BUCKETS - 2..], [2.0, 1.0], "the newest span is the last");
        // Five minutes later the old spans are gone from the chart.
        assert!(a.values(s(400)).iter().all(|n| *n == 0.0));
        for i in 0..100 {
            a.note(s(20 + i * 10));
        }
        assert!(a.spans.len() <= BUCKETS, "it does not grow");
    }

    #[test]
    fn which_panes_a_tile_takes() {
        let agent = PaneFacts {
            agent: true,
            program: "claude",
            title: "Fix the tests",
            workspace: "main",
            ..Default::default()
        };
        let cargo = PaneFacts {
            running: true,
            program: "cargo",
            workspace: "work",
            ..Default::default()
        };
        let failed = PaneFacts {
            failed: true,
            program: "pwsh",
            ..Default::default()
        };
        let idle = PaneFacts {
            program: "pwsh",
            ..Default::default()
        };
        assert!(pane_matches(&PaneFilter::Agents, &agent));
        assert!(!pane_matches(&PaneFilter::Agents, &cargo));
        assert!(
            pane_matches(&PaneFilter::Busy, &cargo) && pane_matches(&PaneFilter::Busy, &failed)
        );
        assert!(!pane_matches(&PaneFilter::Busy, &idle));
        assert!(pane_matches(&PaneFilter::All, &idle));
        let m = |program: Option<&str>, title: Option<&str>| {
            PaneFilter::Match(PaneMatch {
                program: program.map(str::to_owned),
                title: title.map(str::to_owned),
                ..Default::default()
            })
        };
        assert!(pane_matches(&m(Some("carg"), None), &cargo));
        assert!(!pane_matches(&m(Some("carg"), None), &agent));
        assert!(pane_matches(&m(None, Some("tests")), &agent), "no case");
        assert!(
            !pane_matches(&m(Some("claude"), Some("deploy")), &agent),
            "all must match"
        );
    }

    #[test]
    fn the_grid_fits_the_window() {
        assert_eq!(grid(1, 109, 49, 0), (1, 1));
        assert_eq!(grid(4, 109, 49, 0), (2, 2));
        assert_eq!(grid(6, 200, 50, 0), (3, 2));
        assert_eq!(grid(3, 200, 20, 0), (3, 1), "a wide, low window: one row");
        assert_eq!(grid(5, 100, 40, 1), (1, 5), "the config says one column");
        assert_eq!(grid(2, 100, 40, 4), (2, 1), "never more columns than tiles");
        assert_eq!(grid(0, 100, 40, 0), (1, 1));
    }

    #[test]
    fn keys_of_tiles_and_buttons() {
        assert_eq!(tile_key(0), Some('1'));
        assert_eq!(tile_key(8), Some('9'));
        assert_eq!(tile_key(9), None);
        assert_eq!(button_key(0), Some('a'));
        assert_eq!(button_key(26), None);
        let tiles = [
            Tile {
                buttons: vec!["x".into(), "y".into()],
                ..tile("one")
            },
            tile("two"),
            Tile {
                buttons: vec!["z".into()],
                ..tile("three")
            },
        ];
        assert_eq!(button_of_key(&tiles, 'a'), Some((0, 0)));
        assert_eq!(button_of_key(&tiles, 'b'), Some((0, 1)));
        assert_eq!(
            button_of_key(&tiles, 'c'),
            Some((2, 0)),
            "the buttons count across tiles"
        );
        assert_eq!(button_of_key(&tiles, 'd'), None);
    }

    #[test]
    fn a_tile_shows_its_title_status_and_last_lines() {
        let tiles = [Tile {
            title: "claude".into(),
            status: "working 4m".into(),
            state: TileState::Working,
            lines: (1..=30).map(|n| format!("line {n}")).collect(),
            ..Default::default()
        }];
        let rows = rows_text(&tiles, 60, 16);
        let text = rows.join("\n");
        assert!(text.contains("Dashboard"), "{text}");
        assert!(
            text.contains("[1] claude"),
            "the key and the title:\n{text}"
        );
        assert!(text.contains("working 4m"), "{text}");
        // A row has the frame around the text, so look at the text between the frame chars.
        let shown = |n: usize| {
            rows.iter().any(|r| {
                r.split(|c: char| !c.is_ascii_alphanumeric() && c != ' ')
                    .any(|part| part.trim() == format!("line {n}"))
            })
        };
        assert!(shown(30) && shown(21), "the last lines are seen:\n{text}");
        assert!(
            !shown(1) && !shown(10),
            "the first lines do not fit:\n{text}"
        );
    }

    #[test]
    fn long_lines_and_titles_are_cut() {
        assert_eq!(cut("abcdef", 10), "abcdef");
        assert_eq!(cut("abcdef", 4), "abc…");
        assert_eq!(cut("日本語です", 5), "日本…", "wide chars take two cells");
        let tiles = [Tile {
            lines: vec!["x".repeat(500)],
            ..tile("t")
        }];
        let text = rows_text(&tiles, 40, 10);
        assert!(text.iter().all(|r| r.chars().count() <= 40));
        assert!(text.iter().any(|r| r.contains('…')));
    }

    #[test]
    fn tabs_and_control_chars_stay_inside_the_frame() {
        // `gh` with no terminal puts tabs between the fields.
        let line =
            "owner/repo\t2756\topen\tCSBP-1: a long title of a pull request\t2026-10-30T13:11:00Z";
        let tiles = [Tile {
            lines: vec![line.into(), "bell\u{7} and\u{1b} esc".into()],
            ..tile("PRs")
        }];
        let (cols, rows) = (40, 10);
        let text = rows_text(&tiles, cols, rows);
        let frame = |c: char| ('\u{2800}'..='\u{28ff}').contains(&c);
        for row in &text[2..rows - 1] {
            assert!(!row.contains(['\t', '\u{7}', '\u{1b}']), "{row:?}");
            let last = row.chars().nth(cols - 2).unwrap_or(' ');
            assert!(frame(last), "the right frame is still there: {row:?}");
        }
    }

    #[test]
    fn an_item_has_fields_between_tabs() {
        let item = Item::parse("https://github.com/o/r/pull/7\to/r: Fix it\t2026-10-30");
        assert_eq!(item.fields.len(), 3);
        assert_eq!(item.shown(Some(2)), "o/r: Fix it");
        assert_eq!(item.shown(None), item.line);
        assert_eq!(
            item.shown(Some(9)),
            item.line,
            "no such field: the whole line"
        );
        assert_eq!(fill("{1}", &item), "https://github.com/o/r/pull/7");
        assert_eq!(
            fill("see {2} at {1}", &item),
            "see o/r: Fix it at https://github.com/o/r/pull/7"
        );
        assert_eq!(fill("{line}", &item), item.line);
        assert_eq!(
            fill("{4} {x} {", &item),
            " {x} {",
            "no field 4: empty; other braces stay"
        );
    }

    #[test]
    fn a_field_stays_one_argument() {
        // A title of a pull request is written by someone else: it must never become a command.
        let item = Item::parse("https://x/pull/1\tnice\"; rm -rf ~; echo \"");
        let args = fill_args(
            &["gh".into(), "pr".into(), "review".into(), "{2}".into()],
            &item,
        );
        assert_eq!(args.len(), 4, "{args:?}");
        assert_eq!(args[3], "nice\"; rm -rf ~; echo \"");
    }

    #[test]
    fn the_first_link_of_a_line() {
        assert_eq!(
            first_url("o/r#7 https://github.com/o/r/pull/7 open"),
            Some("https://github.com/o/r/pull/7")
        );
        assert_eq!(first_url("see (http://a.b/c)."), Some("http://a.b/c"));
        assert_eq!(first_url("no link here"), None);
    }

    #[test]
    fn a_list_tile_shows_its_cursor_and_keys() {
        let tiles = [Tile {
            lines: (1..=30).map(|n| format!("item {n}")).collect(),
            from_top: true,
            cursor: Some(0),
            keys: vec![
                ("enter".into(), "open".into()),
                ("a".into(), "approve".into()),
            ],
            ..tile("PRs")
        }];
        let (cols, rows) = (60, 14);
        let (ops, places) = draw(&tiles, cols, rows, 0, None, "", &COLORS);
        let mut scene = Scene::new(cols, rows);
        scene.draw(&ops).unwrap();
        let text = scene.canvas().rows_text().join("\n");
        assert!(
            text.contains("› item 1"),
            "the cursor, from the top:\n{text}"
        );
        assert!(!text.contains("item 30"), "{text}");
        assert!(
            text.contains("[enter] open") && text.contains("[a] approve"),
            "{text}"
        );
        assert_eq!(places.keys.len(), 2);
        assert_eq!((places.keys[1].0, places.keys[1].1), (0, 1));
        // The cursor far down: the list scrolls to it.
        let tiles = [Tile {
            cursor: Some(25),
            ..tiles[0].clone()
        }];
        let text = rows_text(&tiles, cols, rows).join("\n");
        assert!(text.contains("› item 26"), "{text}");
        assert!(!text.contains("item 1 "), "{text}");
    }

    #[test]
    fn the_frame_of_the_chosen_tile_stays_off_the_keys() {
        let tiles = [Tile {
            buttons: vec!["Go".into()],
            ..tile("t")
        }];
        let (ops, places) = draw(&tiles, 40, 12, 0, Some(0), "", &COLORS);
        let mut scene = Scene::new(40, 12);
        scene.draw(&ops).unwrap();
        let rect = places.buttons[0].2;
        let row: Vec<char> = scene.canvas().rows_text()[rect.row].chars().collect();
        // Right after the button text there is no frame line.
        assert_eq!(
            row[rect.col + rect.cols],
            ' ',
            "{:?}",
            row.iter().collect::<String>()
        );
    }

    #[test]
    fn buttons_have_keys_and_places() {
        let tiles = [
            tile("panes"),
            Tile {
                title: "Publish".into(),
                buttons: vec!["Staging".into(), "Prod".into()],
                ..Default::default()
            },
        ];
        let (ops, places) = draw(&tiles, 100, 30, 0, None, "", &COLORS);
        assert_eq!(places.tiles.len(), 2);
        assert_eq!(places.buttons.len(), 2);
        let (t, b, rect) = places.buttons[1];
        assert_eq!((t, b), (1, 1));
        assert!(
            places.tiles[1].contains(rect.col, rect.row),
            "a button is in its tile"
        );
        let mut scene = Scene::new(100, 30);
        scene.draw(&ops).unwrap();
        let row = &scene.canvas().rows_text()[rect.row];
        assert!(
            row.contains("[a] Staging") && row.contains("[b] Prod"),
            "{row}"
        );
    }

    #[test]
    fn a_chart_and_the_selected_tile() {
        let tiles = [Tile {
            values: vec![1.0, 5.0, 2.0],
            lines: vec!["out".into()],
            ..tile("busy")
        }];
        let (ops, _) = draw(&tiles, 60, 30, 0, Some(0), "", &COLORS);
        assert!(
            ops.iter().any(|op| matches!(op, Op::Plot { .. })),
            "the chart"
        );
        let rects = ops
            .iter()
            .filter(|op| matches!(op, Op::Rect { .. }))
            .count();
        assert_eq!(rects, 2, "a double frame on the selected tile");
        let (ops, _) = draw(&tiles, 60, 8, 0, None, "", &COLORS);
        assert!(
            !ops.iter().any(|op| matches!(op, Op::Plot { .. })),
            "no room: lines first, no chart"
        );
    }

    #[test]
    fn the_frame_has_the_color_of_the_state() {
        let tiles = [Tile {
            state: TileState::Error,
            ..tile("npm")
        }];
        let (ops, _) = draw(&tiles, 60, 20, 0, None, "", &COLORS);
        let before_rect = ops
            .iter()
            .take_while(|op| !matches!(op, Op::Rect { .. }))
            .filter_map(|op| match op {
                Op::Color { color } => color.clone(),
                _ => None,
            })
            .last();
        assert_eq!(before_rect.as_deref(), Some("#ff0000"));
        assert_eq!(TileState::from_name("waiting"), TileState::Waiting);
        assert_eq!(TileState::from_name("failed"), TileState::Error);
        assert_eq!(TileState::from_name("what"), TileState::Idle);
    }
}
