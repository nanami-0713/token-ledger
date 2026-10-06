use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use jiff::Timestamp;

use super::billing::Billing;
use super::record::UsageRecord;

/// Totals in the three ledgers.
#[derive(Debug, Clone, Default)]
pub struct Totals {
    pub requests: u64,
    pub input_net: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    pub reasoning: u64,
    pub credits: f64,
    pub cost_cny: Option<f64>,
    /// Requests whose source reports a status at all, the denominator of the
    /// health ratios; streamed logs that never mention one stay out.
    pub status_seen: u64,
    pub errors: u64,
    pub retries: u64,
    pub duration_ms: u64,
    pub tool_calls: u64,
}

impl Totals {
    pub fn add(&mut self, rec: &UsageRecord, billing: &Billing) {
        self.requests += 1;
        self.input_net += rec.input_net;
        self.cache_read += rec.cache_read;
        self.cache_write += rec.cache_write;
        self.output += rec.output;
        self.reasoning += rec.reasoning;
        self.credits += billing.credits.credits(rec);
        // The money ledger keeps `None` while no priced request has been
        // seen, so "no price card" stays distinguishable from ¥0.
        self.cost_cny = match (self.cost_cny, billing.cost_cny(rec)) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        if rec.failed || rec.retry || rec.duration_ms.is_some() {
            self.status_seen += 1;
        }
        self.errors += rec.failed as u64;
        self.retries += rec.retry as u64;
        self.duration_ms += rec.duration_ms.unwrap_or(0);
        self.tool_calls += rec.tool_calls;
    }

    /// All five token kinds, the same count the README promises.
    pub fn tokens_total(&self) -> u64 {
        self.input_net + self.cache_read + self.cache_write + self.output + self.reasoning
    }

    pub fn cache_hit_rate(&self) -> Option<f64> {
        // Cache writes are prompt tokens too; leaving them out would flatter
        // every source that bills them separately.
        let gross = self.input_net + self.cache_read + self.cache_write;
        if gross == 0 {
            None
        } else {
            Some(self.cache_read as f64 / gross as f64)
        }
    }
}

/// A local calendar day, `YYYY-MM-DD` in the machine's zone. A timestamp no
/// calendar can hold lands in its own bucket rather than panicking.
pub fn day_of(ts_ms: i64) -> String {
    Timestamp::from_millisecond(ts_ms)
        .map(|ts| ts.to_zoned(TimeZone::system()).strftime("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// The day buckets every chart reads: day → group key → totals.
pub struct Daily {
    pub days: Vec<String>,
    /// key → one value per day, aligned with `days`.
    pub series: BTreeMap<String, Vec<f64>>,
}

impl Daily {
    /// `value` picks the ledger: "credits", "usd", "tokens" or "requests".
    pub fn build(
        records: &[UsageRecord],
        group: fn(&UsageRecord) -> String,
        value: &str,
        billing: &Billing,
    ) -> Self {
        // Format each record's day once; both loops below read this list.
        let day_keys: Vec<String> = records.iter().map(|rec| day_of(rec.ts_ms)).collect();
        let mut days: Vec<String> = Vec::new();
        let mut seen: BTreeMap<String, bool> = BTreeMap::new();
        for day in &day_keys {
            if seen.insert(day.clone(), true).is_none() {
                days.push(day.clone());
            }
        }
        days.sort();
        let mut totals: BTreeMap<(String, String), Totals> = BTreeMap::new();
        for (rec, day) in records.iter().zip(&day_keys) {
            totals
                .entry((group(rec), day.clone()))
                .or_default()
                .add(rec, billing);
        }
        let mut series = BTreeMap::new();
        for ((key, day), total) in totals {
            let ix = days.binary_search(&day).expect("a known day");
            let column = series.entry(key).or_insert_with(|| vec![0.0; days.len()]);
            let cell = match value {
                "credits" => total.credits,
                "cny" => total.cost_cny.unwrap_or(0.0),
                "tokens" => total.tokens_total() as f64,
                _ => total.requests as f64,
            };
            column[ix] += cell;
        }
        Self { days, series }
    }
}

/// One session's footprint, for the sessions table.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub source: String,
    pub session: String,
    pub title: Option<String>,
    pub first_ms: i64,
    pub last_ms: i64,
    pub totals: Totals,
    pub models: Vec<String>,
}

impl SessionRow {
    /// Output tokens per second of wall time, the honest speed number a
    /// request log can give.
    pub fn tps(&self) -> Option<f64> {
        (self.totals.duration_ms > 0).then(|| {
            self.totals.output as f64 / (self.totals.duration_ms as f64 / 1000.0)
        })
    }
}

/// One model's merged footprint, for the models table.
#[derive(Debug, Clone)]
pub struct ModelRow {
    pub model: String,
    pub sources: Vec<String>,
    pub totals: Totals,
    /// Output tokens a second over the model's recorded wall time.
    pub tps: Option<f64>,
}

/// Everything the pages show, computed once per scan. Pages re-render off
/// this without touching the records again — hovering a chart must not
/// re-walk thirty thousand requests.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    pub totals: Totals,
    /// Tokens are the ledger's primary unit; money converted from them by
    /// price card follows, and plan credits stay an auxiliary view.
    pub today_tokens: f64,
    pub week_tokens: f64,
    pub today_credits: f64,
    pub week_credits: f64,
    pub days: Vec<String>,
    /// Daily credits, one column per source, aligned with `days`.
    pub by_source: BTreeMap<String, Vec<f64>>,
    /// Daily credits per model, aligned with `days`.
    pub by_model: BTreeMap<String, Vec<f64>>,
    /// Running total of daily credits, aligned with `days`.
    pub cumulative: Vec<f64>,
    /// Daily credits, one cell per day from the first record to today.
    pub calendar: (jiff::civil::Date, Vec<f64>),
    /// Credits per hour, the last `HOURLY_ROWS` days, newest last.
    pub hourly: Vec<(String, [f64; 24])>,
    /// Tokens a session, on average.
    pub avg_session_tokens: f64,
    /// The newest `TPS_SESSIONS` sessions that report timing, oldest first:
    /// a label and its output tokens per second.
    pub recent_tps: Vec<(String, f64)>,
    pub models_rows: Vec<ModelRow>,
    pub sessions: Vec<SessionRow>,
    /// Tool wall time, from sources that keep a tool ledger.
    pub tool_ms: u64,
}

/// A lookback window for the per-model pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Day,
    Week,
    Month,
    All,
}

impl Span {
    pub const ALL: [Span; 4] = [Span::Day, Span::Week, Span::Month, Span::All];
    pub fn label(self) -> &'static str {
        match self {
            Span::Day => "1D",
            Span::Week => "7D",
            Span::Month => "30D",
            Span::All => "All",
        }
    }
    pub fn cutoff_ms(self, now_ms: i64) -> Option<i64> {
        let days = match self {
            Span::Day => 1,
            Span::Week => 7,
            Span::Month => 30,
            Span::All => return None,
        };
        Some(now_ms - days * 24 * 3600 * 1000)
    }
}

/// Rows in the hour-by-day heatmap.
const HOURLY_ROWS: usize = 14;
/// Conversations in the speed chart.
const TPS_SESSIONS: usize = 30;

impl Ledger {
    pub fn build(records: &[UsageRecord], billing: &Billing, tool_ms: u64) -> Self {
        // Each record's day, formatted once and read by every loop below
        // (today's totals and the hourly grid) instead of re-formatting
        // the same timestamp per pass.
        let day_keys: Vec<String> = records.iter().map(|rec| day_of(rec.ts_ms)).collect();
        let mut totals = Totals::default();
        let today = jiff::Zoned::now().strftime("%Y-%m-%d").to_string();
        let today_ms = jiff::Zoned::now().timestamp().as_millisecond();
        let (mut today_credits, mut week_credits) = (0.0, 0.0);
        let (mut today_tokens, mut week_tokens) = (0.0, 0.0);
        let tokens_of = |rec: &UsageRecord| rec.tokens_total() as f64;
        for (ix, rec) in records.iter().enumerate() {
            totals.add(rec, billing);
            let credits = billing.credits.credits(rec);
            let tokens = tokens_of(rec);
            if day_keys[ix] == today {
                today_credits += credits;
                today_tokens += tokens;
            }
            if today_ms - rec.ts_ms <= 7 * 24 * 3600 * 1000 {
                week_credits += credits;
                week_tokens += tokens;
            }
        }
        let source_daily = Daily::build(records, |rec| rec.source.clone(), "tokens", billing);
        let model_daily = Daily::build(records, |rec| rec.model_key.clone(), "tokens", billing);
        let mut cumulative = vec![0.0; source_daily.days.len()];
        for column in source_daily.series.values() {
            for (ix, value) in column.iter().enumerate() {
                cumulative[ix] += value;
            }
        }
        let mut running = 0.0;
        for value in cumulative.iter_mut() {
            running += *value;
            *value = running;
        }
        let mut merged: BTreeMap<String, BTreeMap<String, Totals>> = BTreeMap::new();
        for rec in records {
            merged
                .entry(rec.model_key.clone())
                .or_default()
                .entry(rec.source.clone())
                .or_default()
                .add(rec, billing);
        }
        let mut models_rows: Vec<ModelRow> = merged
            .into_iter()
            .map(|(model, by_source)| {
                let mut row = ModelRow {
                    model: model.clone(),
                    sources: by_source.keys().cloned().collect(),
                    totals: Totals::default(),
                    tps: None,
                };
                for totals in by_source.values() {
                    // A complete merge: every field a row's consumers may
                    // read, so a new column can never silently read 0.
                    row.totals.requests += totals.requests;
                    row.totals.input_net += totals.input_net;
                    row.totals.cache_read += totals.cache_read;
                    row.totals.cache_write += totals.cache_write;
                    row.totals.output += totals.output;
                    row.totals.reasoning += totals.reasoning;
                    row.totals.credits += totals.credits;
                    row.totals.status_seen += totals.status_seen;
                    row.totals.errors += totals.errors;
                    row.totals.retries += totals.retries;
                    row.totals.duration_ms += totals.duration_ms;
                    row.totals.tool_calls += totals.tool_calls;
                    row.totals.cost_cny = match (row.totals.cost_cny, totals.cost_cny) {
                        (Some(a), Some(b)) => Some(a + b),
                        (a, b) => a.or(b),
                    };
                }
                row.tps = (row.totals.duration_ms > 0).then(|| {
                    row.totals.output as f64 / (row.totals.duration_ms as f64 / 1000.0)
                });
                row
            })
            .collect();
        models_rows.sort_by(|a, b| b.totals.credits.total_cmp(&a.totals.credits));
        // The calendar walks a trailing year, a count for every day,
        // records or not — the GitHub shape.
        let zone = jiff::tz::TimeZone::system();
        let mut daily_total: BTreeMap<String, f64> = BTreeMap::new();
        for column in source_daily.series.values() {
            for (ix, value) in column.iter().enumerate() {
                *daily_total.entry(source_daily.days[ix].clone()).or_default() += value;
            }
        }
        let start = (jiff::Zoned::now() - jiff::SignedDuration::from_hours(24 * 364)).date();
        let mut counts = vec![0.0; 365];
        for (day, value) in &daily_total {
            if let Ok(date) = day.parse::<jiff::civil::Date>() {
                if let Ok(offset) = date.since(start) {
                    let ix = offset.get_days() as usize;
                    if ix < counts.len() {
                        counts[ix] = *value;
                    }
                }
            }
        }
        let calendar = (start, counts);
        // The hour-by-day grid: credits in each hour of the last two weeks.
        let mut hourly: BTreeMap<String, [f64; 24]> = BTreeMap::new();
        let mut hourly_days: Vec<String> = Vec::new();
        for back in (0..HOURLY_ROWS).rev() {
            let day = (jiff::Zoned::now() - jiff::SignedDuration::from_hours(24 * back as i64))
                .strftime("%Y-%m-%d")
                .to_string();
            hourly.entry(day.clone()).or_insert([0.0; 24]);
            hourly_days.push(day);
        }
        for (ix, rec) in records.iter().enumerate() {
            if let Some(row) = hourly.get_mut(&day_keys[ix]) {
                if let Ok(zoned) = jiff::Timestamp::from_millisecond(rec.ts_ms) {
                    let hour = zoned.to_zoned(zone.clone()).hour() as usize;
                    row[hour] += tokens_of(rec);
                }
            }
        }
        let hourly: Vec<(String, [f64; 24])> = hourly_days
            .into_iter()
            .filter_map(|day| hourly.remove(&day).map(|row| (day, row)))
            .collect();
        let mut sessions = sessions(records, billing);
        let avg_session_tokens = if sessions.is_empty() {
            0.0
        } else {
            totals.tokens_total() as f64 / sessions.len() as f64
        };
        let mut timed: Vec<&SessionRow> = sessions
            .iter()
            .filter(|session| session.tps().is_some())
            .collect();
        timed.sort_by_key(|session| session.last_ms);
        let recent_tps: Vec<(String, f64)> = timed
            .into_iter()
            .rev()
            .take(TPS_SESSIONS)
            .map(|session| {
                let label = jiff::Timestamp::from_millisecond(session.last_ms)
                    .map(|ts| {
                        ts.to_zoned(zone.clone()).strftime("%m-%d %H:%M").to_string()
                    })
                    .unwrap_or_else(|_| session.session.clone());
                (label, session.tps().unwrap_or_default())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        sessions.truncate(200);
        Self {
            totals,
            today_tokens,
            week_tokens,
            today_credits,
            week_credits,
            days: source_daily.days,
            by_source: source_daily.series,
            by_model: model_daily.series,
            cumulative,
            calendar,
            hourly,
            avg_session_tokens,
            recent_tps,
            models_rows,
            sessions,
            tool_ms,
        }
    }
}

pub fn sessions(records: &[UsageRecord], billing: &Billing) -> Vec<SessionRow> {
    let mut by_key: BTreeMap<(String, String), SessionRow> = BTreeMap::new();
    for rec in records {
        let entry = by_key
            .entry((rec.source.clone(), rec.session.clone()))
            .or_insert_with(|| SessionRow {
                source: rec.source.clone(),
                session: rec.session.clone(),
                title: rec.session_title.clone(),
                first_ms: rec.ts_ms,
                last_ms: rec.ts_ms,
                totals: Totals::default(),
                models: Vec::new(),
            });
        entry.first_ms = entry.first_ms.min(rec.ts_ms);
        entry.last_ms = entry.last_ms.max(rec.ts_ms);
        entry.totals.add(rec, billing);
        if !entry.models.contains(&rec.model_key) {
            entry.models.push(rec.model_key.clone());
        }
    }
    let mut rows: Vec<SessionRow> = by_key.into_values().collect();
    rows.sort_by(|a, b| b.totals.credits.total_cmp(&a.totals.credits));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::billing::{Billing, CreditConfig, default_prices};

    fn rec(model: &str) -> UsageRecord {
        UsageRecord {
            source: "test".into(),
            session: "s".into(),
            session_title: None,
            provider: None,
            model_raw: model.into(),
            model_key: model.into(),
            ts_ms: 0,
            input_net: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
            output: 1_000_000,
            reasoning: 1_000_000,
            ttft_ms: None,
            agent: None,
            failed: false,
            retry: false,
            duration_ms: None,
            tool_calls: 0,
        }
    }

    fn billing() -> Billing {
        Billing {
            credits: CreditConfig::default(),
            prices: default_prices(),
            usd_cny: 7.2,
        }
    }

    #[test]
    fn money_stays_none_until_a_priced_request() {
        let b = billing();
        let mut totals = Totals::default();
        totals.add(&rec("no-price-card-for-this"), &b);
        assert!(totals.cost_cny.is_none());
        totals.add(&rec("glm-5.3"), &b);
        assert!(totals.cost_cny.is_some());
        // A later unpriced request must not zero the money answer out.
        totals.add(&rec("no-price-card-for-this"), &b);
        assert!(totals.cost_cny.unwrap() > 0.0);
    }

    #[test]
    fn tokens_total_counts_all_five_kinds() {
        let b = billing();
        let mut totals = Totals::default();
        totals.add(&rec("glm-5.3"), &b);
        assert_eq!(totals.tokens_total(), 5_000_000);
        // Cache writes are prompt tokens: one of three equal parts here.
        assert!((totals.cache_hit_rate().unwrap() - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn merged_model_rows_keep_every_field() {
        let b = billing();
        let mut one = rec("glm-5.3");
        one.source = "one".into();
        let mut two = rec("glm-5.3");
        two.source = "two".into();
        let ledger = Ledger::build(&[one, two], &b, 0);
        let row = &ledger.models_rows[0];
        assert_eq!(row.totals.requests, 2);
        assert_eq!(row.totals.cache_write, 2_000_000);
        assert_eq!(row.totals.reasoning, 2_000_000);
        assert_eq!(row.totals.tokens_total(), 10_000_000);
    }

    #[test]
    fn a_time_no_calendar_holds_lands_in_its_own_day_bucket() {
        assert_eq!(day_of(i64::MAX), "unknown");
        assert_ne!(day_of(1_791_183_600_000), "unknown");
    }
}
