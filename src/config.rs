use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml::value::{Datetime, Offset};

use crate::pricing::{ModelPrices, PriceTable, Prices};
use crate::sources::claude_json::Account;

#[derive(Debug)]
pub enum Error {
    Io(PathBuf, io::Error),
    Toml(toml::de::Error),
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(path, error) => write!(f, "{}: {error}", path.display()),
            Error::Toml(error) => write!(f, "{error}"),
            Error::Invalid(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSettings {
    pub label: Option<String>,
    pub tracking: bool,
    pub store_titles: bool,
    pub credit_reset: Option<CreditReset>,
}

impl Default for AccountSettings {
    fn default() -> Self {
        AccountSettings {
            label: None,
            tracking: true,
            store_titles: true,
            credit_reset: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditReset {
    pub day: u8,
    pub minute_of_day: u16,
    pub offset_minutes: i16,
}

impl CreditReset {
    pub const PROVISIONAL: CreditReset = CreditReset {
        day: 1,
        minute_of_day: 0,
        offset_minutes: 0,
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switch {
    pub at_ms: i64,
    pub earlier: Account,
}

pub const DEFAULT_REPORT_PORT: u16 = 7878;
pub const DEFAULT_FAST_USD_PER_HOUR: f64 = 20.0;

#[derive(Debug, Default)]
pub struct Config {
    pub extra_paths: Vec<PathBuf>,
    pub request_detail_days: Option<u32>,
    report_port: Option<u16>,
    accounts: HashMap<Account, AccountSettings>,
    switches: HashMap<PathBuf, Switch>,
    prices: HashMap<String, ModelPrices>,
    price_updates: Option<bool>,
    fast_usd_per_hour: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    extra_paths: Vec<String>,
    request_detail_days: Option<u32>,
    report_port: Option<u16>,
    accounts: BTreeMap<String, RawAccount>,
    directories: BTreeMap<String, RawDirectory>,
    prices: BTreeMap<String, RawPrices>,
    price_updates: Option<bool>,
    fast_usd_per_hour: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAccount {
    label: Option<String>,
    tracking: Option<bool>,
    store_titles: Option<bool>,
    credit_reset: Option<Datetime>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrices {
    input: f64,
    cache_write_5m: f64,
    cache_write_1h: f64,
    cache_read: f64,
    output: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDirectory {
    switched_at: Datetime,
    earlier_account: String,
}

impl Config {
    pub fn load(path: &Path, home: &Path) -> Result<Config, Error> {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text, home),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
            Err(error) => Err(Error::Io(path.to_path_buf(), error)),
        }
    }

    pub fn parse(text: &str, home: &Path) -> Result<Config, Error> {
        let raw: RawConfig = toml::from_str(text).map_err(Error::Toml)?;
        if raw.request_detail_days == Some(0) {
            return Err(Error::Invalid(
                "request_detail_days must be 1 or more. Leave it out to keep request detail forever"
                    .to_string(),
            ));
        }
        if raw
            .fast_usd_per_hour
            .is_some_and(|rate| !rate.is_finite() || rate <= 0.0)
        {
            return Err(Error::Invalid(
                "fast_usd_per_hour must be more than 0, in US dollars an hour".to_string(),
            ));
        }
        let mut accounts = HashMap::new();
        for (id, settings) in raw.accounts {
            let account = id.parse().map_err(Error::Invalid)?;
            let defaults = AccountSettings::default();
            let credit_reset = match settings.credit_reset {
                None => None,
                Some(datetime) => Some(credit_reset(&datetime).ok_or_else(|| {
                    Error::Invalid(format!(
                        "credit_reset for {id} needs a date, a time, and a time zone, like 2026-11-01T00:00:00Z"
                    ))
                })?),
            };
            accounts.insert(
                account,
                AccountSettings {
                    label: settings.label,
                    tracking: settings.tracking.unwrap_or(defaults.tracking),
                    store_titles: settings.store_titles.unwrap_or(defaults.store_titles),
                    credit_reset,
                },
            );
        }
        let mut switches = HashMap::new();
        for (path, directory) in raw.directories {
            let at_ms = epoch_ms(&directory.switched_at).ok_or_else(|| {
                Error::Invalid(format!(
                    "switched_at for {path} needs a date, a time, and a time zone, like 2026-11-10T17:00:00Z"
                ))
            })?;
            let earlier = directory.earlier_account.parse().map_err(Error::Invalid)?;
            switches.insert(expand_home(&path, home), Switch { at_ms, earlier });
        }
        Ok(Config {
            extra_paths: raw
                .extra_paths
                .iter()
                .map(|path| expand_home(path, home))
                .collect(),
            request_detail_days: raw.request_detail_days,
            report_port: raw.report_port,
            accounts,
            switches,
            prices: raw
                .prices
                .into_iter()
                .map(|(model, prices)| {
                    let prices = Prices {
                        input: prices.input,
                        cache_write_5m: prices.cache_write_5m,
                        cache_write_1h: prices.cache_write_1h,
                        cache_read: prices.cache_read,
                        output: prices.output,
                    };
                    (model, ModelPrices::standard(prices))
                })
                .collect(),
            price_updates: raw.price_updates,
            fast_usd_per_hour: raw.fast_usd_per_hour,
        })
    }

    pub fn report_port(&self) -> Option<u16> {
        match self.report_port {
            Some(0) => None,
            Some(port) => Some(port),
            None => Some(DEFAULT_REPORT_PORT),
        }
    }

    pub fn account(&self, account: &Account) -> AccountSettings {
        self.accounts.get(account).cloned().unwrap_or_default()
    }

    pub fn configured_accounts(&self) -> impl Iterator<Item = &Account> {
        self.accounts.keys()
    }

    pub fn switch(&self, directory: &Path) -> Option<&Switch> {
        self.switches.get(directory)
    }

    pub fn price_table(&self) -> PriceTable {
        PriceTable::with_overrides(self.prices.clone())
    }

    pub fn price_table_over(&self, downloaded: HashMap<String, ModelPrices>) -> PriceTable {
        PriceTable::layered(downloaded, self.prices.clone())
    }

    pub fn price_updates(&self) -> bool {
        self.price_updates.unwrap_or(true)
    }

    pub fn fast_usd_per_hour(&self) -> f64 {
        self.fast_usd_per_hour.unwrap_or(DEFAULT_FAST_USD_PER_HOUR)
    }
}

pub fn add_extra_path(text: &str, path: &Path, home: &Path) -> Result<String, Error> {
    let mut document = editable(text)?;
    let entry = home_relative(path, home);
    let paths = document
        .entry("extra_paths")
        .or_insert(toml_edit::value(toml_edit::Array::new()))
        .as_array_mut()
        .ok_or_else(|| Error::Invalid("extra_paths must be a list of paths".to_string()))?;
    if !paths
        .iter()
        .any(|existing| existing.as_str() == Some(&entry))
    {
        paths.push(entry);
    }
    Ok(document.to_string())
}

pub fn set_account_flag(
    text: &str,
    account: &Account,
    key: &str,
    value: bool,
) -> Result<String, Error> {
    let mut document = editable(text)?;
    let trailing = document.trailing().as_str().unwrap_or_default().to_string();
    let accounts = document
        .entry("accounts")
        .or_insert(toml_edit::table())
        .as_table_mut()
        .ok_or_else(|| Error::Invalid("accounts must be a table".to_string()))?;
    accounts.set_implicit(true);
    let id = account.to_string();
    let is_new = !accounts.contains_key(&id);
    let settings = accounts
        .entry(&id)
        .or_insert(toml_edit::table())
        .as_table_mut()
        .ok_or_else(|| Error::Invalid(format!("accounts.\"{account}\" must be a table")))?;
    settings[key] = toml_edit::value(value);
    if is_new && !trailing.is_empty() {
        settings.decor_mut().set_prefix(format!("{trailing}\n"));
        document.set_trailing("");
    }
    Ok(document.to_string())
}

fn editable(text: &str) -> Result<toml_edit::DocumentMut, Error> {
    text.parse()
        .map_err(|error: toml_edit::TomlError| Error::Invalid(error.to_string()))
}

pub fn home_relative(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn expand_home(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if path == "~" => home.to_path_buf(),
        None => PathBuf::from(path),
    }
}

pub fn epoch_ms_from_text(timestamp: &str) -> Option<i64> {
    epoch_ms(&timestamp.parse().ok()?)
}

fn credit_reset(datetime: &Datetime) -> Option<CreditReset> {
    let date = datetime.date?;
    let time = datetime.time?;
    let offset_minutes = match datetime.offset? {
        Offset::Z => 0,
        Offset::Custom { minutes } => minutes,
    };
    Some(CreditReset {
        day: date.day,
        minute_of_day: u16::from(time.hour) * 60 + u16::from(time.minute),
        offset_minutes,
    })
}

fn epoch_ms(datetime: &Datetime) -> Option<i64> {
    let date = datetime.date?;
    let time = datetime.time?;
    let offset_minutes = match datetime.offset? {
        Offset::Z => 0,
        Offset::Custom { minutes } => i64::from(minutes),
    };
    let days = days_from_civil(i64::from(date.year), date.month, date.day);
    let seconds = days * 86_400
        + i64::from(time.hour) * 3_600
        + i64::from(time.minute) * 60
        + i64::from(time.second.unwrap_or(0))
        - offset_minutes * 60;
    Some(seconds * 1_000 + i64::from(time.nanosecond.unwrap_or(0) / 1_000_000))
}

// Howard Hinnant's days_from_civil: days since 1970-01-01 in the proleptic Gregorian calendar.
pub(crate) fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
