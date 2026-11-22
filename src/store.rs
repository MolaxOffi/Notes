//! Notes and their file format: pure data, no game calls.

/// Which list a note belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    World,
    Server,
    Global,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::World, Scope::Server, Scope::Global];

    pub fn index(self) -> usize {
        match self {
            Scope::World => 0,
            Scope::Server => 1,
            Scope::Global => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Note {
    pub id: u64,
    pub pinned: bool,
    pub title: String,
    pub body: String,
}

pub const MAGIC: &str = "NOTES1";
pub const TITLE_MAX: usize = 48;
pub const BODY_MAX: usize = 4000;

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The text of a notes file.
pub fn encode(notes: &[Note]) -> String {
    let mut out = String::from(MAGIC);
    out.push('\n');
    for n in notes {
        out.push_str(&format!(
            "N\t{}\t{}\t{}\t{}\n",
            n.id,
            u8::from(n.pinned),
            escape(&n.title),
            escape(&n.body)
        ));
    }
    out
}

/// The notes of a file's text, and how many lines could not be read.
pub fn decode(text: &str) -> (Vec<Note>, usize) {
    let mut lines = text.lines();
    let mut bad = 0;
    if lines.next().map(str::trim) != Some(MAGIC) {
        return (Vec::new(), text.lines().filter(|l| !l.trim().is_empty()).count().max(1));
    }
    let mut notes = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        let parsed = (parts.len() == 5 && parts[0] == "N")
            .then(|| {
                Some(Note {
                    id: parts[1].parse().ok()?,
                    pinned: parts[2] == "1",
                    title: unescape(parts[3]),
                    body: unescape(parts[4]),
                })
            })
            .flatten();
        match parsed {
            Some(n) => notes.push(n),
            None => bad += 1,
        }
    }
    (notes, bad)
}

pub fn next_id(notes: &[Note]) -> u64 {
    notes.iter().map(|n| n.id).max().map_or(1, |m| m + 1)
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// The file a scope's notes live in; `key` is the world folder or the server address.
pub fn file_name(scope: Scope, key: &str) -> String {
    match scope {
        Scope::Global => "global.txt".to_owned(),
        Scope::World => format!("world-{}.txt", hex(key)),
        Scope::Server => format!("server-{}.txt", hex(&key.to_lowercase())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Note> {
        vec![
            Note { id: 1, pinned: true, title: "Base".into(), body: "x: 10\ty: 64\nline two \\ end".into() },
            Note { id: 7, pinned: false, title: String::new(), body: String::new() },
            Note { id: 9, pinned: false, title: "é à 😀".into(), body: "日本語\n\n".into() },
        ]
    }

    #[test]
    fn round_trips_every_character() {
        let (back, bad) = decode(&encode(&sample()));
        assert_eq!(bad, 0);
        assert_eq!(back, sample());
    }

    #[test]
    fn unreadable_lines_are_counted_and_the_rest_kept() {
        let text = format!("{MAGIC}\nN\t1\t0\ta\tb\ngarbage\nN\tx\t0\ta\tb\nN\t2\t1\tc\td\n");
        let (notes, bad) = decode(&text);
        assert_eq!(bad, 2);
        assert_eq!(notes.iter().map(|n| n.id).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn a_foreign_file_is_all_bad() {
        let (notes, bad) = decode("hello\nworld\n");
        assert!(notes.is_empty());
        assert_eq!(bad, 2);
    }

    #[test]
    fn ids_are_unique_and_file_names_are_safe() {
        assert_eq!(next_id(&[]), 1);
        assert_eq!(next_id(&sample()), 10);
        for (s, k) in [(Scope::World, "My World/../x"), (Scope::Server, "Play.Example.com:25565")] {
            let name = file_name(s, k);
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || "-.".contains(c)));
        }
        assert_eq!(file_name(Scope::Server, "A.B"), file_name(Scope::Server, "a.b"));
        assert_eq!(file_name(Scope::Global, "ignored"), "global.txt");
    }
}
