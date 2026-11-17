use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::sources::transcript::{Request, Tokens};

pub const TABLE_SOURCE: &str = "https://platform.claude.com/docs/en/about-claude/pricing.md";
pub const TABLE_DATE: &str = "2026-11-11";

const WEB_SEARCH_USD: f64 = 10.0 / 1000.0;
const US_GEO_MULTIPLIER: f64 = 1.1;
const HAIKU_5_5_LONG_PROMPT_TOKENS: u64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FastPrices {
    pub input: f64,
    pub output: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelPrices {
    pub standard: Prices,
    pub long_prompt: Option<Prices>,
    pub fast: Option<FastPrices>,
}

impl ModelPrices {
    pub const fn standard(prices: Prices) -> Self {
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

const FABLE_5_2: Prices = Prices::new(8.0, 10.0, 16.0, 0.20, 40.0);
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

const FAST_FABLE_5_2: FastPrices = FastPrices {
    input: 16.0,
    output: 80.0,
};
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
        ("claude-fable-5-2", with_fast(FABLE_5_2, FAST_FABLE_5_2)),
        ("claude-mythos-5-2", with_fast(FABLE_5_2, FAST_FABLE_5_2)),
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
    downloaded: HashMap<String, ModelPrices>,
    overrides: HashMap<String, ModelPrices>,
}

impl PriceTable {
    pub fn built_in() -> Self {
        PriceTable::layered(HashMap::new(), HashMap::new())
    }

    pub fn with_overrides(overrides: HashMap<String, ModelPrices>) -> Self {
        PriceTable::layered(HashMap::new(), overrides)
    }

    pub fn layered(
        downloaded: HashMap<String, ModelPrices>,
        overrides: HashMap<String, ModelPrices>,
    ) -> Self {
        let built_in = built_in_models();
        let downloaded = inheriting_rules(downloaded, &built_in);
        let overrides = inheriting_rules(overrides, &built_in);
        PriceTable {
            built_in,
            downloaded,
            overrides,
        }
    }

    pub fn model(&self, model: &str) -> Option<&ModelPrices> {
        let id = without_date_suffix(model);
        self.overrides
            .get(id)
            .or_else(|| self.downloaded.get(id))
            .or_else(|| self.built_in.get(id))
    }

    pub fn built_in_model(&self, model: &str) -> Option<&ModelPrices> {
        self.built_in.get(without_date_suffix(model))
    }

    pub fn highest(&self, field: impl Fn(&Prices) -> f64) -> f64 {
        self.built_in
            .values()
            .chain(self.downloaded.values())
            .chain(self.overrides.values())
            .flat_map(|prices| [Some(prices.standard), prices.long_prompt])
            .flatten()
            .map(|prices| field(&prices))
            .fold(0.0, f64::max)
    }

    pub fn price(&self, request: &Request) -> Option<f64> {
        self.price_tokens(
            &request.model,
            &request.tokens,
            request.inference_geo.as_deref(),
            request.speed.as_deref(),
        )
    }

    pub fn price_tokens(
        &self,
        model: &str,
        tokens: &Tokens,
        inference_geo: Option<&str>,
        speed: Option<&str>,
    ) -> Option<f64> {
        let prices = self.model(model)?.for_request(tokens, inference_geo, speed);
        Some(prices.cost(tokens))
    }

    pub fn fingerprint(&self) -> String {
        let mut built_in: Vec<_> = self.built_in.iter().collect();
        built_in.sort_by_key(|(model, _)| *model);
        let mut overrides: Vec<_> = self.overrides.iter().collect();
        overrides.sort_by_key(|(model, _)| model.as_str());
        if self.downloaded.is_empty() {
            return format!("{TABLE_DATE} {built_in:?} {overrides:?}");
        }
        let mut downloaded: Vec<_> = self.downloaded.iter().collect();
        downloaded.sort_by_key(|(model, _)| model.as_str());
        format!("{TABLE_DATE} {built_in:?} {downloaded:?} {overrides:?}")
    }
}

fn inheriting_rules(
    prices: HashMap<String, ModelPrices>,
    built_in: &HashMap<&'static str, ModelPrices>,
) -> HashMap<String, ModelPrices> {
    prices
        .into_iter()
        .map(|(model, mut prices)| {
            let model = without_date_suffix(&model).to_string();
            if let Some(known) = built_in.get(model.as_str()) {
                prices.long_prompt = prices.long_prompt.or(known.long_prompt);
                prices.fast = prices.fast.or(known.fast);
            }
            (model, prices)
        })
        .collect()
}

pub(crate) fn without_date_suffix(model: &str) -> &str {
    match model.rsplit_once('-') {
        Some((id, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => id,
        _ => model,
    }
}
