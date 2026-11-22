//! Word-wrapping and cursor placement for the note body: pure, with the font's widths passed in.

/// One drawn line: characters `start..end` of the text, and the x of each character boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub edges: Vec<f32>,
}

/// Wraps `text` to `max_w`, breaking at spaces when it can, and at a newline always.
pub fn wrap(text: &str, max_w: f32, width: &dyn Fn(char) -> f32) -> Vec<Line> {
    let chars: Vec<char> = text.chars().collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start <= chars.len() {
        let mut x = 0.0f32;
        let mut edges = vec![0.0f32];
        let mut i = start;
        let mut last_space: Option<usize> = None;
        let mut end = chars.len();
        let mut next = chars.len() + 1;
        while i < chars.len() {
            let c = chars[i];
            if c == '\n' {
                end = i;
                next = i + 1;
                break;
            }
            let w = width(c);
            if x + w > max_w && i > start {
                match last_space {
                    Some(s) if s + 1 > start => {
                        end = s + 1;
                        edges.truncate(end - start + 1);
                    }
                    _ => end = i,
                }
                next = end;
                break;
            }
            x += w;
            edges.push(x);
            if c == ' ' {
                last_space = Some(i);
            }
            i += 1;
        }
        lines.push(Line { start, end, edges });
        if next > chars.len() {
            break;
        }
        start = next;
    }
    lines
}

/// The line the cursor is drawn on, and its column.
pub fn locate(lines: &[Line], pos: usize) -> (usize, usize) {
    for (i, l) in lines.iter().enumerate() {
        let last = i + 1 == lines.len();
        if last || pos < lines[i + 1].start {
            return (i, pos.saturating_sub(l.start).min(l.end - l.start));
        }
    }
    (0, 0)
}

/// The column of `line` whose edge is nearest `x`.
pub fn column_at(line: &Line, x: f32) -> usize {
    let mut best = 0;
    let mut dist = f32::MAX;
    for (c, e) in line.edges.iter().enumerate() {
        let d = (e - x).abs();
        if d < dist {
            dist = d;
            best = c;
        }
    }
    best.min(line.end - line.start)
}

/// The cursor after Up (`-1`) or Down (`1`).
pub fn vertical(lines: &[Line], pos: usize, dir: isize) -> usize {
    let (row, col) = locate(lines, pos);
    let target = row as isize + dir;
    if target < 0 {
        return 0;
    }
    let Some(line) = lines.get(target as usize) else {
        return lines.last().map_or(0, |l| l.end);
    };
    let x = lines[row].edges.get(col).copied().unwrap_or(0.0);
    line.start + column_at(line, x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(_: char) -> f32 {
        6.0
    }

    #[test]
    fn newlines_and_wraps() {
        let lines = wrap("ab\ncd", 100.0, &w);
        assert_eq!(lines.iter().map(|l| (l.start, l.end)).collect::<Vec<_>>(), [(0, 2), (3, 5)]);
        let lines = wrap("hello world again", 60.0, &w);
        let spans: Vec<_> = lines.iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(spans, [(0, 6), (6, 12), (12, 17)]);
        assert!(lines.iter().all(|l| *l.edges.last().unwrap() <= 60.0));
    }

    #[test]
    fn long_word_breaks_and_empty_text_has_one_line() {
        let lines = wrap("abcdefghijkl", 30.0, &w);
        assert_eq!(lines.len(), 3);
        assert_eq!(wrap("", 30.0, &w).len(), 1);
        assert_eq!(wrap("a\n", 30.0, &w).len(), 2);
    }

    #[test]
    fn cursor_lands_on_the_next_line_after_a_wrap_but_not_after_a_newline() {
        let lines = wrap("hello world", 60.0, &w); // (0,6) (6,11)
        assert_eq!(locate(&lines, 6), (1, 0));
        let lines = wrap("ab\ncd", 100.0, &w);
        assert_eq!(locate(&lines, 2), (0, 2));
        assert_eq!(locate(&lines, 3), (1, 0));
    }

    #[test]
    fn up_down_keep_the_column() {
        let lines = wrap("abcd\nef\nghij", 100.0, &w);
        assert_eq!(vertical(&lines, 3, 1), 7); // col 3 -> "ef" clamps to its end
        assert_eq!(vertical(&lines, 7, 1), 10);
        assert_eq!(vertical(&lines, 3, -1), 0);
        assert_eq!(vertical(&lines, 11, 1), 12);
        assert_eq!(column_at(&lines[0], 13.0), 2);
    }
}
