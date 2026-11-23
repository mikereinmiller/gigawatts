use super::sessions::{self, SessionsToday};

#[derive(Clone, Copy)]
pub(crate) enum Step {
    Spending,
    Spacing,
    CardsGap,
    Chart,
    SecondRow,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Extras {
    pub(crate) spending: bool,
    pub(crate) spacing: bool,
    pub(crate) cards_gap: bool,
    pub(crate) chart: bool,
    pub(crate) second_row: bool,
}

impl Extras {
    fn with(mut self, step: Step) -> Extras {
        match step {
            Step::Spending => self.spending = true,
            Step::Spacing => self.spacing = true,
            Step::CardsGap => self.cards_gap = true,
            Step::Chart => self.chart = true,
            Step::SecondRow => self.second_row = true,
        }
        self
    }

    pub(crate) fn cards(self) -> sessions::Cards {
        sessions::Cards::Rows {
            rows: if self.second_row { 2 } else { 1 },
            gap: self.cards_gap,
        }
    }
}

pub(crate) fn grow(start: Extras, steps: &[Step], fits: impl Fn(Extras) -> bool) -> Extras {
    let mut extras = start;
    for step in steps {
        let tried = extras.with(*step);
        if !fits(tried) {
            break;
        }
        extras = tried;
    }
    extras
}

pub(crate) fn cards_height(
    sessions_today: &SessionsToday,
    width: u16,
    with_sessions: bool,
    extras: Extras,
) -> u16 {
    if !with_sessions {
        return 0;
    }
    sessions::height(sessions_today, width, u16::MAX, extras.cards())
        .max(sessions::one_row_height(extras.cards_gap))
}
