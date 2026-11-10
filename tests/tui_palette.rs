use gigawatts::tui::palette::{Rgb, blend, parse_replies};

#[test]
fn a_terminal_reply_gives_its_text_and_background_colors() {
    let replies = b"\x1b]10;rgb:ebeb/dbdb/b2b2\x1b\\\x1b]11;rgb:2828/2828/2828\x07\x1b[?62;22c";

    let (text, background) = parse_replies(replies);

    assert_eq!(text, Some(Rgb(0xeb, 0xdb, 0xb2)));
    assert_eq!(background, Some(Rgb(0x28, 0x28, 0x28)));
}

#[test]
fn two_digit_channels_and_a_missing_reply_parse_too() {
    let (text, background) = parse_replies(b"\x1b]11;rgb:ff/80/00\x1b\\\x1b[?1;2c");

    assert_eq!(text, None);
    assert_eq!(background, Some(Rgb(0xff, 0x80, 0x00)));
}

#[test]
fn a_shade_sits_the_given_share_of_the_way_from_background_to_text() {
    assert_eq!(
        blend(Rgb(0, 0, 0), Rgb(200, 100, 50), 0.5),
        Rgb(100, 50, 25)
    );
    assert_eq!(
        blend(Rgb(40, 40, 40), Rgb(240, 240, 240), 0.25),
        Rgb(90, 90, 90)
    );
}
