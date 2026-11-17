use std::collections::HashMap;

use rusqlite::OptionalExtension;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{Result, Store};
use crate::config::Config;
use crate::pricing::{ModelPrices, PriceTable};

const DOWNLOADED_PRICES: &str = "downloaded_prices";
const PRICE_UPDATE: &str = "price_update";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DownloadedPrices {
    pub fetched_at_ms: i64,
    pub models: HashMap<String, ModelPrices>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceUpdate {
    pub attempted_at_ms: i64,
    pub failure: Option<String>,
    pub failing_since_ms: Option<i64>,
}

impl Store {
    pub fn price_table(&self, config: &Config) -> Result<PriceTable> {
        let downloaded = self.downloaded_prices()?.unwrap_or_default();
        Ok(config.price_table_over(downloaded.models))
    }

    pub fn downloaded_prices(&self) -> Result<Option<DownloadedPrices>> {
        self.json_setting(DOWNLOADED_PRICES)
    }

    pub fn save_downloaded_prices(&mut self, prices: &DownloadedPrices) -> Result<()> {
        self.save_json_setting(DOWNLOADED_PRICES, prices)
    }

    pub fn last_price_update(&self) -> Result<Option<PriceUpdate>> {
        self.json_setting(PRICE_UPDATE)
    }

    pub fn save_price_update(&mut self, update: &PriceUpdate) -> Result<()> {
        self.save_json_setting(PRICE_UPDATE, update)
    }

    fn json_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let text: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM store_settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(text.and_then(|text| serde_json::from_str(&text).ok()))
    }

    fn save_json_setting<T: Serialize>(&mut self, key: &str, value: &T) -> Result<()> {
        let text = serde_json::to_string(value).expect("prices serialize to JSON");
        self.connection.execute(
            "INSERT INTO store_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [key, text.as_str()],
        )?;
        Ok(())
    }
}
