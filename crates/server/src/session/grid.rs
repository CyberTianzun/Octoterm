use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

#[derive(Clone)]
struct VoidListener;
impl EventListener for VoidListener {
    fn send_event(&self, _event: Event) {}
}

pub struct SessionGrid {
    term: Term<VoidListener>,
    parser: Processor,
    cols: u16,
    rows: u16,
}

impl SessionGrid {
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = TermSize::new(cols as usize, rows as usize);
        let term = Term::new(Config::default(), &size, VoidListener);
        Self { term, parser: Processor::new(), cols, rows }
    }

    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.term.resize(TermSize::new(cols as usize, rows as usize));
    }

    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// SGR 相关的 flags 子集：用于测试比对，避免因 WRAPLINE/WIDE_CHAR 等
    /// 排版记账位在 roundtrip 前后合理不同而导致误判。
    #[cfg(test)]
    const SGR_FLAGS: Flags = Flags::BOLD
        .union(Flags::DIM)
        .union(Flags::ITALIC)
        .union(Flags::UNDERLINE)
        .union(Flags::INVERSE)
        .union(Flags::STRIKEOUT)
        .union(Flags::HIDDEN);

    /// 返回 (字符, "fg|bg|flags" 调试串) 供测试比对
    #[cfg(test)]
    fn cell_at(&self, line: usize, col: usize) -> (char, String) {
        let cell = &self.term.grid()[Line(line as i32)][Column(col)];
        (
            cell.c,
            format!("{:?}|{:?}|{:?}", cell.fg, cell.bg, cell.flags & Self::SGR_FLAGS),
        )
    }

    pub fn repaint(&self) -> Vec<u8> {
        let mut out = String::from("\x1b[0m\x1b[?25l\x1b[2J\x1b[H");
        let grid = self.term.grid();
        let mut last_sgr = String::new();
        for line in 0..self.rows as usize {
            out.push_str(&format!("\x1b[{};1H", line + 1));
            for col in 0..self.cols as usize {
                let cell = &grid[Line(line as i32)][Column(col)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let sgr = sgr_string(cell.fg, cell.bg, cell.flags);
                if sgr != last_sgr {
                    out.push_str(&sgr);
                    last_sgr = sgr;
                }
                out.push(if cell.c == '\0' { ' ' } else { cell.c });
            }
        }
        out.push_str("\x1b[0m");
        let cursor = grid.cursor.point;
        out.push_str(&format!("\x1b[{};{}H", cursor.line.0 + 1, cursor.column.0 + 1));
        if self.term.mode().contains(TermMode::SHOW_CURSOR) {
            out.push_str("\x1b[?25h");
        }
        out.into_bytes()
    }
}

fn base_index(n: NamedColor) -> Option<(u8, bool)> {
    use NamedColor::*;
    Some(match n {
        Black => (0, false), Red => (1, false), Green => (2, false), Yellow => (3, false),
        Blue => (4, false), Magenta => (5, false), Cyan => (6, false), White => (7, false),
        BrightBlack => (0, true), BrightRed => (1, true), BrightGreen => (2, true),
        BrightYellow => (3, true), BrightBlue => (4, true), BrightMagenta => (5, true),
        BrightCyan => (6, true), BrightWhite => (7, true),
        _ => return None,
    })
}

fn color_params(c: Color, fg: bool) -> String {
    let (default, base, bright_base, ext) = if fg { ("39", 30, 90, "38") } else { ("49", 40, 100, "48") };
    match c {
        Color::Named(n) => match base_index(n) {
            Some((i, false)) => format!("{}", base + i as u16),
            Some((i, true)) => format!("{}", bright_base + i as u16),
            None => default.to_string(),
        },
        Color::Indexed(i) => format!("{ext};5;{i}"),
        Color::Spec(rgb) => format!("{ext};2;{};{};{}", rgb.r, rgb.g, rgb.b),
    }
}

fn sgr_string(fg: Color, bg: Color, flags: Flags) -> String {
    let mut parts = vec!["0".to_string()];
    if flags.contains(Flags::BOLD) { parts.push("1".into()); }
    if flags.contains(Flags::DIM) { parts.push("2".into()); }
    if flags.contains(Flags::ITALIC) { parts.push("3".into()); }
    // v1 起见,把所有花式下划线变体(双线/曲线/点线/虚线)都近似成普通下划线 SGR 4。
    if flags.intersects(
        Flags::UNDERLINE
            | Flags::DOUBLE_UNDERLINE
            | Flags::UNDERCURL
            | Flags::DOTTED_UNDERLINE
            | Flags::DASHED_UNDERLINE,
    ) {
        parts.push("4".into());
    }
    if flags.contains(Flags::INVERSE) { parts.push("7".into()); }
    if flags.contains(Flags::STRIKEOUT) { parts.push("9".into()); }
    if flags.contains(Flags::HIDDEN) { parts.push("8".into()); }
    parts.push(color_params(fg, true));
    parts.push(color_params(bg, false));
    format!("\x1b[{}m", parts.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_lands_in_grid() {
        let mut g = SessionGrid::new(20, 5);
        g.advance(b"hi");
        assert_eq!(g.cell_at(0, 0).0, 'h');
        assert_eq!(g.cell_at(0, 1).0, 'i');
    }

    #[test]
    fn repaint_roundtrip_preserves_content_and_color() {
        let mut a = SessionGrid::new(20, 5);
        a.advance(b"plain \x1b[31mred\x1b[0m\r\nline2 \x1b[1;44mbold-on-blue\x1b[0m\x1b[8mhidden");
        let repaint = a.repaint();

        let mut b = SessionGrid::new(20, 5);
        b.advance(&repaint);

        for line in 0..5 {
            for col in 0..20 {
                assert_eq!(
                    a.cell_at(line, col),
                    b.cell_at(line, col),
                    "cell mismatch at {line},{col}"
                );
            }
        }
    }

    #[test]
    fn repaint_roundtrip_preserves_wide_chars() {
        let mut a = SessionGrid::new(20, 5);
        // "你好 ok" 中文宽字符 + 红色前景，行尾要覆盖 WIDE_CHAR_SPACER 跳过路径与列对齐。
        a.advance(b"\x1b[31m\xe4\xbd\xa0\xe5\xa5\xbd ok");
        let repaint = a.repaint();

        let mut b = SessionGrid::new(20, 5);
        b.advance(&repaint);

        for line in 0..5 {
            for col in 0..20 {
                assert_eq!(
                    a.cell_at(line, col),
                    b.cell_at(line, col),
                    "cell mismatch at {line},{col}"
                );
            }
        }
    }

    #[test]
    fn repaint_restores_cursor_position() {
        let mut a = SessionGrid::new(20, 5);
        a.advance(b"abc\x1b[3;4H"); // 光标移到 3 行 4 列
        let repaint = String::from_utf8(a.repaint()).unwrap();
        assert!(repaint.ends_with("\x1b[3;4H\x1b[?25h"), "got tail: {:?}", &repaint[repaint.len().saturating_sub(24)..]);
    }
}
