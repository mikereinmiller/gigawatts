use rusqlite::{Transaction, params};

use super::{Result, Store, delete_unused_labels};

impl Store {
    pub fn expire_requests(&mut self, keep_days: u32, today: &str) -> Result<u64> {
        let expired =
            self.without_foreign_keys(|store| store.delete_expired_requests(keep_days, today))?;
        let free_pages: i64 = self
            .connection
            .query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
        if free_pages == 0 {
            return Ok(expired);
        }
        self.free_unused_pages()?;
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(expired)
    }

    fn free_unused_pages(&self) -> Result<()> {
        // incremental_vacuum frees one page per step, and execute_batch steps only once.
        let mut vacuum = self.connection.prepare("PRAGMA incremental_vacuum")?;
        let mut freed = vacuum.query([])?;
        while freed.next()?.is_some() {}
        Ok(())
    }

    fn delete_expired_requests(&mut self, keep_days: u32, today: &str) -> Result<u64> {
        let transaction = self.connection.transaction()?;
        let cutoff: String = transaction.query_row(
            "SELECT date(?1, ?2)",
            params![today, format!("-{} days", keep_days.saturating_sub(1))],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO store_settings (key, value) VALUES ('requests_kept_from', ?1)
             ON CONFLICT (key) DO UPDATE SET value = max(value, excluded.value)",
            [&cutoff],
        )?;
        transaction.execute(
            "UPDATE sessions SET requests_expired = 1
             WHERE id IN (SELECT session_id FROM requests WHERE day < ?1)",
            [&cutoff],
        )?;
        save_expired_totals(&transaction, &cutoff)?;
        transaction.execute(
            "DELETE FROM request_tools WHERE request_row_id IN (
                SELECT id FROM requests WHERE day < ?1
             )",
            [&cutoff],
        )?;
        let expired = transaction.execute("DELETE FROM requests WHERE day < ?1", [&cutoff])?;
        if expired > 0 {
            drop_price_gaps_of_expired_pairs(&transaction)?;
            delete_unused_labels(&transaction)?;
        }
        transaction.commit()?;
        Ok(expired as u64)
    }
}

fn drop_price_gaps_of_expired_pairs(transaction: &Transaction) -> Result<()> {
    transaction.execute(
        "UPDATE session_models SET price_gap_usd = NULL
         WHERE price_gap_usd IS NOT NULL
           AND EXISTS (
               SELECT 1 FROM expired_requests
               WHERE expired_requests.session_id = session_models.session_id
                 AND expired_requests.model_label = session_models.model_label
           )
           AND NOT EXISTS (
               SELECT 1 FROM requests
               WHERE requests.session_id = session_models.session_id
                 AND requests.model_label = session_models.model_label
           )",
        [],
    )?;
    Ok(())
}

fn save_expired_totals(transaction: &Transaction, cutoff: &str) -> Result<()> {
    transaction.execute(
        "INSERT INTO expired_requests (
            session_id, model_label, input_tokens, output_tokens, cache_read_tokens,
            cache_write_5m_tokens, cache_write_1h_tokens, web_searches, cost_usd
         )
         SELECT session_id, model_label, sum(input_tokens), sum(output_tokens),
                sum(cache_read_tokens), sum(cache_write_5m_tokens), sum(cache_write_1h_tokens),
                sum(web_searches), total(cost_usd)
         FROM requests
         WHERE day < ?1
         GROUP BY session_id, model_label
         ON CONFLICT (session_id, model_label) DO UPDATE SET
            input_tokens = input_tokens + excluded.input_tokens,
            output_tokens = output_tokens + excluded.output_tokens,
            cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
            cache_write_5m_tokens = cache_write_5m_tokens + excluded.cache_write_5m_tokens,
            cache_write_1h_tokens = cache_write_1h_tokens + excluded.cache_write_1h_tokens,
            web_searches = web_searches + excluded.web_searches,
            cost_usd = cost_usd + excluded.cost_usd",
        [cutoff],
    )?;
    Ok(())
}
