use std::collections::HashMap;

use crate::sources::transcript::{Request, Tokens};

pub const TABLE_SOURCE: &str = "https://platform.claude.com/docs/en/about-claude/pricing.md";
pub const TABLE_DATE: &str = "2026-11-02";

const WEB_SEARCH_USD: f64 = 10.0 / 1000.0;
const US_GEO_MULTIPLIER: f64 = 1.1;
const HAIKU_5_5_LONG_PROMPT_TOKENS: u64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prices {
    pub input: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
    pub output: f64,
}

impl Prices {
    const fn new(
        input: f64,
        cache_write_5m: f64,
        cache_write_1h: f64,
        cache_read: f64,
        output: f64,
    ) -> Self {
        Prices {
            input,
            cache_write_5m,
            cache_write_1h,
            cache_read,
            output,
        }
    }

    fn scaled(&self, factor: f64) -> Prices {
        Prices::new(
            self.input * factor,
            self.cache_write_5m * factor,
            self.cache_write_1h * factor,
            self.cache_read * factor,
            self.output * factor,
        )
    }

    fn fast(&self, fast: FastPrices) -> Prices {
        let input_factor = fast.input / self.input;
        Prices::new(
            fast.input,
            self.cache_write_5m * input_factor,
            self.cache_write_1h * input_factor,
            self.cache_read * input_factor,
            fast.output,
        )
    }

    pub fn cost(&self, tokens: &Tokens) -> f64 {
        let per_mtok = tokens.input as f64 * self.input
            + tokens.cache_write_5m as f64 * self.cache_write_5m
            + tokens.cache_write_1h as f64 * self.cache_write_1h
            + tokens.cache_read as f64 * self.cache_read
            + tokens.output as f64 * self.output;
        per_mtok / 1_000_000.0 + tokens.web_searches as f64 * WEB_SEARCH_USD
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FastPrices {
    pub input: f64,
    pub output: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrices {
    pub standard: Prices,
    pub long_prompt: Option<Prices>,
    pub fast: Option<FastPrices>,
}

impl ModelPrices {
    const fn standard(prices: Prices) -> Self {
        ModelPrices {
            standard: prices,
            long_prompt: None,
            fast: None,
        }
    }

    pub fn for_request(
        &self,
        tokens: &Tokens,
        inference_geo: Option<&str>,
        speed: Option<&str>,
    ) -> Prices {
        let mut prices = match self.long_prompt {
            Some(long) if tokens.prompt() > HAIKU_5_5_LONG_PROMPT_TOKENS => long,
            _ => self.standard,
        };
        if let (Some(fast), Some("fast")) = (self.fast, speed) {
            prices = prices.fast(fast);
        }
        if inference_geo == Some("us") {
            prices = prices.scaled(US_GEO_MULTIPLIER);
        }
        prices
    }
}

const FABLE_5_1: Prices = Prices::new(10.0, 12.5, 20.0, 0.25, 50.0);
const FABLE_5: Prices = Prices::new(10.0, 12.5, 20.0, 1.0, 50.0);
const OPUS_5_5: Prices = Prices::new(4.0, 5.0, 8.0, 0.20, 20.0);
const OPUS_5: Prices = Prices::new(5.0, 6.25, 10.0, 0.50, 25.0);
const OPUS_4_1: Prices = Prices::new(15.0, 18.75, 30.0, 1.50, 75.0);
const SONNET_5_5: Prices = Prices::new(2.0, 2.5, 4.0, 0.10, 10.0);
const SONNET_5: Prices = Prices::new(2.0, 2.5, 4.0, 0.20, 10.0);
const SONNET_4_6: Prices = Prices::new(3.0, 3.75, 6.0, 0.30, 15.0);
const HAIKU_5_5: Prices = Prices::new(0.10, 0.125, 0.20, 0.01, 0.50);
const HAIKU_5_5_LONG: Prices = Prices::new(0.50, 0.625, 1.0, 0.05, 2.50);
const HAIKU_4_5: Prices = Prices::new(1.0, 1.25, 2.0, 0.10, 5.0);
const HAIKU_3_5: Prices = Prices::new(0.80, 1.0, 1.60, 0.08, 4.0);

const FAST_OPUS_5_5: FastPrices = FastPrices {
    input: 8.0,
    output: 40.0,
};
const FAST_OPUS_5: FastPrices = FastPrices {
    input: 10.0,
    output: 50.0,
};

fn built_in_models() -> HashMap<&'static str, ModelPrices> {
    let standard = ModelPrices::standard;
    let with_fast = |prices, fast| ModelPrices {
        standard: prices,
        long_prompt: None,
        fast: Some(fast),
    };
    HashMap::from([
        ("claude-fable-5-1", standard(FABLE_5_1)),
        ("claude-mythos-5-1", standard(FABLE_5_1)),
        ("claude-fable-5", standard(FABLE_5)),
        ("claude-mythos-5", standard(FABLE_5)),
        ("claude-opus-5-5", with_fast(OPUS_5_5, FAST_OPUS_5_5)),
        ("claude-opus-5", with_fast(OPUS_5, FAST_OPUS_5)),
        ("claude-opus-4-8", with_fast(OPUS_5, FAST_OPUS_5)),
        ("claude-opus-4-7", standard(OPUS_5)),
        ("claude-opus-4-6", standard(OPUS_5)),
        ("claude-opus-4-5", standard(OPUS_5)),
        ("claude-opus-4-1", standard(OPUS_4_1)),
        ("claude-opus-4", standard(OPUS_4_1)),
        ("claude-sonnet-5-5", standard(SONNET_5_5)),
        ("claude-sonnet-5", standard(SONNET_5)),
        ("claude-sonnet-4-6", standard(SONNET_4_6)),
        ("claude-sonnet-4-5", standard(SONNET_4_6)),
        ("claude-sonnet-4", standard(SONNET_4_6)),
        (
            "claude-haiku-5-5",
            ModelPrices {
                standard: HAIKU_5_5,
                long_prompt: Some(HAIKU_5_5_LONG),
                fast: None,
            },
        ),
        ("claude-haiku-4-5", standard(HAIKU_4_5)),
        ("claude-3-5-haiku", standard(HAIKU_3_5)),
    ])
}

pub struct PriceTable {
    built_in: HashMap<&'static str, ModelPrices>,
    overrides: HashMap<String, ModelPrices>,
}

impl PriceTable {
    pub fn built_in() -> Self {
        PriceTable {
            built_in: built_in_models(),
            overrides: HashMap::new(),
        }
    }

    pub fn with_overrides(overrides: HashMap<String, ModelPrices>) -> Self {
        PriceTable {
            built_in: built_in_models(),
            overrides,
        }
    }

    pub fn model(&self, model: &str) -> Option<&ModelPrices> {
        let id = without_date_suffix(model);
        self.overrides.get(id).or_else(|| self.built_in.get(id))
    }

    pub fn price(&self, request: &Request) -> Option<f64> {
        let prices = self.model(&request.model)?.for_request(
            &request.tokens,
            request.inference_geo.as_deref(),
            request.speed.as_deref(),
        );
        Some(prices.cost(&request.tokens))
    }
}

fn without_date_suffix(model: &str) -> &str {
    match model.rsplit_once('-') {
        Some((id, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => id,
        _ => model,
    }
}
