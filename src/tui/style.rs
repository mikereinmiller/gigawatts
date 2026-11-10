use ratatui::style::{Color, Modifier, Style};

pub(crate) const SECONDARY: Color = Color::Indexed(244);

pub(crate) const NORMAL: Style = Style::new();
pub(crate) const TITLE: Style = Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD);
pub(crate) const KEY: Style = Style::new().fg(Color::Cyan);
pub(crate) const MUTED: Style = Style::new().fg(SECONDARY);
pub(crate) const STALE: Color = Color::DarkGray;
pub(crate) const UNDER: Color = Color::LightGreen;
pub(crate) const EVEN: Color = Color::LightYellow;
pub(crate) const OVER: Color = Color::LightRed;
pub(crate) const UNPACED: Color = Color::Reset;
pub(crate) const WARNING: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
pub(crate) const DANGER: Style = Style::new()
    .fg(Color::LightRed)
    .add_modifier(Modifier::BOLD);
pub(crate) const PROBLEM: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::DIM);
