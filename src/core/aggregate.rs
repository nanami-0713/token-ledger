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
        let cny = self.cost_cny.unwrap_or(0.0) + billing.cost_cny(rec).unwrap_or(0.0);
        self.cost_cny = Some(cny);
        if rec.failed || rec.retry || rec.duration_ms.is_some() {
            self.status_seen += 1;
        }
        self.errors += rec.failed as u64;
        self.retries += rec.retry as u64;
        self.duration_ms += rec.duration_ms.unwrap_or(0);
    }

    pub fn cache_hit_rate(&self) -> Option<f64> {
        let gross = self.input_net + self.cache_read;
        if gross == 0 {
            None
        } else {
            Some(self.cache_read as f64 / gross as f64)
        }
    }
}

/// A local calendar day, `YYYY-MM-DD` in the machine's zone.
pub fn day_of(ts_ms: i64) -> String {
    Timestamp::from_millisecond(ts_ms)
        .expect("a time in range")
        .to_zoned(TimeZone::system())
        .strftime("%Y-%m-%d")
        .to_string()
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
        let mut days: Vec<String> = Vec::new();
        let mut seen: BTreeMap<String, bool> = BTreeMap::new();
        for rec in records {
            let day = day_of(rec.ts_ms);
            if seen.insert(day.clone(), true).is_none() {
                days.push(day);
            }
        }
        days.sort();
        let mut totals: BTreeMap<(String, String), Totals> = BTreeMap::new();
        for rec in records {
            let day = day_of(rec.ts_ms);
            totals
                .entry((group(rec), day))
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
                "tokens" => (total.input_net + total.cache_read + total.output) as f64,
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
}

/// Rows in the hour-by-day heatmap.
const HOURLY_ROWS: usize = 14;
/// Conversations in the speed chart.
const TPS_SESSIONS: usize = 30;

impl Ledger {
    pub fn build(records: &[UsageRecord], billing: &Billing) -> Self {
        let mut totals = Totals::default();
        let today = jiff::Zoned::now().strftime("%Y-%m-%d").to_string();
        let today_ms = jiff::Zoned::now().timestamp().as_millisecond();
        let (mut today_credits, mut week_credits) = (0.0, 0.0);
        let (mut today_tokens, mut week_tokens) = (0.0, 0.0);
        let tokens_of = |rec: &UsageRecord| {
            (rec.input_net + rec.cache_read + rec.output) as f64
        };
        for rec in records {
            totals.add(rec, billing);
            let credits = billing.credits.credits(rec);
            let tokens = tokens_of(rec);
            if day_of(rec.ts_ms) == today {
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
                };
                for totals in by_source.values() {
                    row.totals.requests += totals.requests;
                    row.totals.input_net += totals.input_net;
                    row.totals.cache_read += totals.cache_read;
                    row.totals.output += totals.output;
                    row.totals.credits += totals.credits;
                    row.totals.cost_cny = match (row.totals.cost_cny, totals.cost_cny) {
                        (Some(a), Some(b)) => Some(a + b),
                        (a, b) => a.or(b),
                    };
                }
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
        let today = jiff::Zoned::now().date();
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
        for rec in records {
            let day = day_of(rec.ts_ms);
            if let Some(row) = hourly.get_mut(&day) {
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
            (totals.input_net + totals.cache_read + totals.output) as f64
                / sessions.len() as f64
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
