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

pub(crate) const SMALL_ROWS: usize = 3;

fn small_glyph(character: char) -> Option<[&'static str; 5]> {
    Some(match character {
        '0' => ["###", "#.#", "#.#", "#.#", "###"],
        '1' => ["##.", ".#.", ".#.", ".#.", "###"],
        '2' => ["###", "..#", "###", "#..", "###"],
        '3' => ["###", "..#", ".##", "..#", "###"],
        '4' => ["#.#", "#.#", "###", "..#", "..#"],
        '5' => ["###", "#..", "###", "..#", "###"],
        '6' => ["###", "#..", "###", "#.#", "###"],
        '7' => ["###", "..#", "..#", "..#", "..#"],
        '8' => ["###", "#.#", "###", "#.#", "###"],
        '9' => ["###", "#.#", "###", "..#", "###"],
        '.' => [".", ".", ".", ".", "#"],
        _ => return None,
    })
}

pub(crate) fn small(text: &str) -> Option<[String; SMALL_ROWS]> {
    let glyphs: Vec<[&str; 5]> = text.chars().map(small_glyph).collect::<Option<_>>()?;
    let mut pixels: [String; 6] = Default::default();
    for (index, glyph) in glyphs.iter().enumerate() {
        let width = glyph[0].len();
        for (row, line) in pixels.iter_mut().enumerate() {
            line.push_str(glyph.get(row).copied().unwrap_or(&".".repeat(width)));
            if index + 1 < glyphs.len() {
                line.push('.');
            }
        }
    }
    let mut rows: [String; SMALL_ROWS] = Default::default();
    for (row, out) in rows.iter_mut().enumerate() {
        *out = pixels[2 * row]
            .chars()
            .zip(pixels[2 * row + 1].chars())
            .map(|pair| match pair {
                ('#', '#') => '█',
                ('#', _) => '▀',
                (_, '#') => '▄',
                _ => ' ',
            })
            .collect();
    }
    Some(rows)
}
