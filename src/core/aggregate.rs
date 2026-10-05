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
    pub totals: Totals,
    pub models: Vec<String>,
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
    pub today_credits: f64,
    pub week_credits: f64,
    pub days: Vec<String>,
    /// Daily credits, one column per source, aligned with `days`.
    pub by_source: BTreeMap<String, Vec<f64>>,
    /// Daily credits per model, aligned with `days`.
    pub by_model: BTreeMap<String, Vec<f64>>,
    /// Running total of daily credits, aligned with `days`.
    pub cumulative: Vec<f64>,
    pub models_rows: Vec<ModelRow>,
    pub sessions: Vec<SessionRow>,
}

impl Ledger {
    pub fn build(records: &[UsageRecord], billing: &Billing) -> Self {
        let mut totals = Totals::default();
        let today = jiff::Zoned::now().strftime("%Y-%m-%d").to_string();
        let today_ms = jiff::Zoned::now().timestamp().as_millisecond();
        let (mut today_credits, mut week_credits) = (0.0, 0.0);
        for rec in records {
            totals.add(rec, billing);
            let credits = billing.credits.credits(rec);
            if day_of(rec.ts_ms) == today {
                today_credits += credits;
            }
            if today_ms - rec.ts_ms <= 7 * 24 * 3600 * 1000 {
                week_credits += credits;
            }
        }
        let source_daily = Daily::build(records, |rec| rec.source.clone(), "credits", billing);
        let model_daily = Daily::build(records, |rec| rec.model_key.clone(), "credits", billing);
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
        let mut sessions = sessions(records, billing);
        sessions.truncate(200);
        Self {
            totals,
            today_credits,
            week_credits,
            days: source_daily.days,
            by_source: source_daily.series,
            by_model: model_daily.series,
            cumulative,
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
                totals: Totals::default(),
                models: Vec::new(),
            });
        entry.first_ms = entry.first_ms.min(rec.ts_ms);
        entry.totals.add(rec, billing);
        if !entry.models.contains(&rec.model_key) {
            entry.models.push(rec.model_key.clone());
        }
    }
    let mut rows: Vec<SessionRow> = by_key.into_values().collect();
    rows.sort_by(|a, b| b.totals.credits.total_cmp(&a.totals.credits));
    rows
}
