use std::collections::HashMap;
use std::process::Command;

use crate::config::Config;
use crate::pricing::{FastPrices, ModelPrices, PriceTable, Prices, TABLE_SOURCE};
use crate::store::{self, DownloadedPrices, PriceUpdate, Store};

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const PLAUSIBLE_FACTOR: f64 = 5.0;
const DOWNLOAD_SECONDS: &str = "20";
const COLUMNS: [&str; 6] = [
    "Model",
    "Base input tokens",
    "5m cache writes",
    "1h cache writes",
    "Cache hits and refreshes",
    "Output tokens",
];
const FAST_COLUMNS: [&str; 3] = ["Model", "Input", "Output"];
const UP_TO_100K: &str = "(for prompts up to 100,000 tokens)";
const OVER_100K: &str = "(for prompts over 100,000 tokens)";

pub trait Download {
    fn get(&self, url: &str) -> Result<String, String>;
}

pub struct Curl;

impl Download for Curl {
    fn get(&self, url: &str) -> Result<String, String> {
        let user_agent = format!("gigawatts/{}", env!("CARGO_PKG_VERSION"));
        let output = Command::new("curl")
            .arg("-q")
            .args(["--fail", "--silent", "--show-error", "--location"])
            .args(["--proto", "=https", "--proto-redir", "=https"])
            .args(["--max-time", DOWNLOAD_SECONDS, "--user-agent", &user_agent])
            .arg(url)
            .output()
            .map_err(|error| format!("can't run `curl`: {error}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                format!("`curl` exited ({})", output.status)
            } else {
                stderr
            });
        }
        String::from_utf8(output.stdout).map_err(|_| "the page isn't UTF-8 text".to_string())
    }
}

pub fn update_if_needed(
    store: &mut Store,
    config: &Config,
    download: &dyn Download,
    now_ms: i64,
) -> store::Result<()> {
    let last = store.last_price_update()?;
    if !config.price_updates() {
        return forget_failure(store, last);
    }
    if last
        .as_ref()
        .is_some_and(|last| now_ms - last.attempted_at_ms < DAY_MS)
    {
        return Ok(());
    }
    let table = store.price_table(config)?;
    if store.models_without_price(&table)?.is_empty() && store.price_gaps()?.is_empty() {
        return forget_failure(store, last);
    }
    let previous = store.downloaded_prices()?.unwrap_or_default();
    match fetch(download, &table) {
        Ok(page) => {
            let downloaded = DownloadedPrices {
                fetched_at_ms: now_ms,
                models: changed_from_built_in(previous.models, page, &table),
            };
            store.save_downloaded_prices(&downloaded)?;
            store.save_price_update(&PriceUpdate {
                attempted_at_ms: now_ms,
                failure: None,
                failing_since_ms: None,
            })?;
            store.reprice_if_changed(&store.price_table(config)?)?;
        }
        Err(failure) => {
            let failing_since_ms = last
                .and_then(|last| last.failing_since_ms)
                .unwrap_or(now_ms);
            store.save_price_update(&PriceUpdate {
                attempted_at_ms: now_ms,
                failure: Some(failure),
                failing_since_ms: Some(failing_since_ms),
            })?;
        }
    }
    Ok(())
}

fn forget_failure(store: &mut Store, last: Option<PriceUpdate>) -> store::Result<()> {
    match last.filter(|last| last.failure.is_some()) {
        Some(last) => store.save_price_update(&PriceUpdate {
            failure: None,
            failing_since_ms: None,
            ..last
        }),
        None => Ok(()),
    }
}

fn fetch(
    download: &dyn Download,
    current: &PriceTable,
) -> Result<HashMap<String, ModelPrices>, String> {
    let page = download
        .get(TABLE_SOURCE)
        .map_err(|error| format!("Gigawatts can't download {TABLE_SOURCE}: {error}"))?;
    let models =
        parse_page(&page).map_err(|error| format!("{TABLE_SOURCE} doesn't parse: {error}"))?;
    match implausible(&models, current) {
        Some(price) => Err(format!(
            "{TABLE_SOURCE} lists {price}, which looks wrong, so Gigawatts kept its current prices"
        )),
        None => Ok(models),
    }
}

fn changed_from_built_in(
    mut kept: HashMap<String, ModelPrices>,
    page: HashMap<String, ModelPrices>,
    current: &PriceTable,
) -> HashMap<String, ModelPrices> {
    for (model, prices) in page {
        let unchanged = current.built_in_model(&model).is_some_and(|known| {
            known.standard == prices.standard
                && prices
                    .long_prompt
                    .is_none_or(|long| known.long_prompt == Some(long))
                && prices.fast.is_none_or(|fast| known.fast == Some(fast))
        });
        if unchanged {
            kept.remove(&model);
        } else {
            kept.insert(model, prices);
        }
    }
    kept
}

pub fn parse_page(page: &str) -> Result<HashMap<String, ModelPrices>, String> {
    let Some(rows) = table_rows(page, &COLUMNS) else {
        return Err(format!(
            "it has no table with the columns {}",
            COLUMNS.join(", ")
        ));
    };
    if rows.is_empty() {
        return Err("its model pricing table has no rows".to_string());
    }
    let mut standard: HashMap<String, Prices> = HashMap::new();
    let mut long_prompt: HashMap<String, Prices> = HashMap::new();
    for row in rows {
        let [
            name,
            input,
            cache_write_5m,
            cache_write_1h,
            cache_read,
            output,
        ] = row.as_slice()
        else {
            return Err(format!("a row has {} cells: {row:?}", row.len()));
        };
        let prices = Prices {
            input: price(name, input)?,
            cache_write_5m: price(name, cache_write_5m)?,
            cache_write_1h: price(name, cache_write_1h)?,
            cache_read: price(name, cache_read)?,
            output: price(name, output)?,
        };
        let (model, over_100k) = model_id(name)?;
        let prices_for = if over_100k {
            &mut long_prompt
        } else {
            &mut standard
        };
        if prices_for.insert(model, prices).is_some() {
            return Err(format!("{name} is listed twice"));
        }
    }
    if let Some(model) = long_prompt
        .keys()
        .find(|model| !standard.contains_key(*model))
    {
        return Err(format!(
            "{model} has a long-prompt price but no standard one"
        ));
    }
    let mut fast = fast_prices(page)?;
    if let Some(model) = fast.keys().find(|model| !standard.contains_key(*model)) {
        return Err(format!("{model} has a fast-mode price but no standard one"));
    }
    Ok(standard
        .into_iter()
        .map(|(model, prices)| {
            let model_prices = ModelPrices {
                standard: prices,
                long_prompt: long_prompt.remove(&model),
                fast: fast.remove(&model),
            };
            (model, model_prices)
        })
        .collect())
}

fn fast_prices(page: &str) -> Result<HashMap<String, FastPrices>, String> {
    let mut fast = HashMap::new();
    for row in table_rows(page, &FAST_COLUMNS).unwrap_or_default() {
        let [names, input, output] = row.as_slice() else {
            return Err(format!("a fast-mode row has {} cells: {row:?}", row.len()));
        };
        let prices = FastPrices {
            input: price(names, input)?,
            output: price(names, output)?,
        };
        for name in names.split(" / ") {
            let (model, _) = model_id(name)?;
            if fast.insert(model, prices).is_some() {
                return Err(format!("{name} has two fast-mode prices"));
            }
        }
    }
    Ok(fast)
}

fn table_rows(page: &str, columns: &[&str]) -> Option<Vec<Vec<String>>> {
    let mut lines = page.lines().skip_while(|line| cells(line) != columns);
    lines.next()?;
    Some(
        lines
            .skip(1)
            .take_while(|line| line.trim_start().starts_with('|'))
            .map(cells)
            .collect(),
    )
}

fn cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let Some(inner) = line
        .strip_prefix('|')
        .and_then(|line| line.strip_suffix('|'))
    else {
        return Vec::new();
    };
    inner
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn model_id(name: &str) -> Result<(String, bool), String> {
    let over_100k = name.ends_with(OVER_100K);
    let bare = name.split(" (").next().unwrap_or(name).trim();
    let qualifier = name[bare.len()..].trim();
    if qualifier.starts_with("(for ") && qualifier != UP_TO_100K && qualifier != OVER_100K {
        return Err(format!("{name} has a pricing rule Gigawatts doesn't know"));
    }
    let id = bare.to_lowercase().replace([' ', '.'], "-");
    if !id.starts_with("claude-")
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!("{name:?} doesn't read as a model name"));
    }
    Ok((id, over_100k))
}

fn price(model: &str, cell: &str) -> Result<f64, String> {
    let without_note = match cell.find("<sup>") {
        Some(start) => &cell[..start],
        None => cell,
    };
    without_note
        .trim()
        .strip_prefix('$')
        .and_then(|amount| amount.strip_suffix("/ MTok"))
        .map(|amount| amount.trim().trim_end_matches("USD").trim())
        .and_then(|amount| amount.parse::<f64>().ok())
        .ok_or_else(|| format!("{cell:?} for {model} isn't a price per million tokens"))
}

struct Amount {
    name: String,
    listed: f64,
    now: Option<f64>,
    highest: f64,
}

fn implausible(page: &HashMap<String, ModelPrices>, current: &PriceTable) -> Option<String> {
    let mut models: Vec<_> = page.iter().collect();
    models.sort_by_key(|(model, _)| model.as_str());
    for (model, prices) in models {
        for amount in amounts(prices, current.model(model), current) {
            let reference = amount.now.unwrap_or(amount.highest);
            let too_high = amount.listed > reference * PLAUSIBLE_FACTOR;
            let too_low = amount.now.is_some() && amount.listed < reference / PLAUSIBLE_FACTOR;
            if !amount.listed.is_finite() || amount.listed <= 0.0 || too_high || too_low {
                let against = match amount.now {
                    Some(_) => format!("against ${reference} now"),
                    None => format!("against ${reference} for the most expensive model"),
                };
                return Some(format!(
                    "{model}'s {} price as ${} per million tokens, {against}",
                    amount.name, amount.listed
                ));
            }
        }
    }
    None
}

fn amounts(listed: &ModelPrices, known: Option<&ModelPrices>, current: &PriceTable) -> Vec<Amount> {
    let mut amounts = Vec::new();
    let tiers = [
        ("", Some(listed.standard), known.map(|known| known.standard)),
        (
            "long-prompt ",
            listed.long_prompt,
            known.and_then(|known| known.long_prompt),
        ),
    ];
    for (tier, prices, now) in tiers {
        let Some(prices) = prices else { continue };
        for (index, (name, amount)) in fields(&prices).into_iter().enumerate() {
            amounts.push(Amount {
                name: format!("{tier}{name}"),
                listed: amount,
                now: now.map(|now| fields(&now)[index].1),
                highest: current.highest(|prices| fields(prices)[index].1),
            });
        }
    }
    if let Some(fast) = listed.fast {
        let now = known.and_then(|known| known.fast);
        amounts.push(Amount {
            name: "fast-mode input".to_string(),
            listed: fast.input,
            now: now.map(|now| now.input),
            highest: current.highest(|prices| prices.input),
        });
        amounts.push(Amount {
            name: "fast-mode output".to_string(),
            listed: fast.output,
            now: now.map(|now| now.output),
            highest: current.highest(|prices| prices.output),
        });
    }
    amounts
}

fn fields(prices: &Prices) -> [(&'static str, f64); 5] {
    [
        ("input", prices.input),
        ("5-minute cache write", prices.cache_write_5m),
        ("1-hour cache write", prices.cache_write_1h),
        ("cache read", prices.cache_read),
        ("output", prices.output),
    ]
}
