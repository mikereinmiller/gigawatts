pub(crate) const ROWS: usize = 5;

fn glyph(character: char) -> Option<[&'static str; ROWS]> {
    Some(match character {
        '0' => ["█▀█", "█ █", "█ █", "█ █", "█▄█"],
        '1' => ["▀█ ", " █ ", " █ ", " █ ", "▄█▄"],
        '2' => ["▀▀█", "  █", "█▀▀", "█  ", "█▄▄"],
        '3' => ["▀▀█", "  █", " ▀█", "  █", "▄▄█"],
        '4' => ["█ █", "█ █", "▀▀█", "  █", "  █"],
        '5' => ["█▀▀", "█  ", "▀▀█", "  █", "▄▄█"],
        '6' => ["█▀▀", "█  ", "█▀█", "█ █", "█▄█"],
        '7' => ["▀▀█", "  █", "  █", "  █", "  █"],
        '8' => ["█▀█", "█ █", "█▀█", "█ █", "█▄█"],
        '9' => ["█▀█", "█ █", "▀▀█", "  █", "▄▄█"],
        '$' => ["▄█▄", "█  ", "▀█▄", "  █", "▀█▀"],
        '.' => ["  ", "  ", "  ", "  ", "▄ "],
        _ => return None,
    })
}

pub(crate) fn large(text: &str) -> Option<[String; ROWS]> {
    let glyphs: Vec<(char, [&str; ROWS])> = text
        .chars()
        .map(|character| glyph(character).map(|rows| (character, rows)))
        .collect::<Option<_>>()?;
    let mut rows: [String; ROWS] = Default::default();
    for (index, (character, glyph)) in glyphs.iter().enumerate() {
        let last = index + 1 == glyphs.len();
        for (row, part) in rows.iter_mut().zip(glyph) {
            row.push_str(part);
            if !last && *character != '.' {
                row.push(' ');
            }
        }
    }
    Some(rows)
}
