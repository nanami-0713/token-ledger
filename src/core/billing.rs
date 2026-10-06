use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use jiff::Timestamp;
use serde::Deserialize;

use super::record::UsageRecord;

/// Three ledgers run side by side: raw tokens, plan credits and API dollars.
/// Tokens always exist; the other two appear where a rate is known.

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreditConfig {
    /// Coefficients per 10k tokens, flash tier.
    #[serde(default = "default_flash_in")]
    pub flash_in: f64,
    #[serde(default = "default_flash_cache")]
    pub flash_cache: f64,
    #[serde(default = "default_flash_out")]
    pub flash_out: f64,
    /// Coefficients per 10k tokens, every other model.
    #[serde(default = "default_std_in")]
    pub std_in: f64,
    #[serde(default = "default_std_cache")]
    pub std_cache: f64,
    #[serde(default = "default_std_out")]
    pub std_out: f64,
    #[serde(default = "default_divisor")]
    pub divisor: f64,
    /// Peak window is full price; outside it this factor applies.
    #[serde(default = "default_offpeak_factor")]
    pub offpeak_factor: f64,
}

// One function per field so `config.toml` can override a single coefficient
// without restating the rest; `Default` reads the same functions, so the
// built-in formula lives in exactly one place.
fn default_flash_in() -> f64 {
    2.3
}
fn default_flash_cache() -> f64 {
    0.56
}
fn default_flash_out() -> f64 {
    8.0
}
fn default_std_in() -> f64 {
    6.9
}
fn default_std_cache() -> f64 {
    1.7
}
fn default_std_out() -> f64 {
    24.0
}
fn default_divisor() -> f64 {
    10_000.0
}
fn default_offpeak_factor() -> f64 {
    0.5
}

impl Default for CreditConfig {
    fn default() -> Self {
        // The GLM Coding Plan formula, verified against zusage.sh.
        Self {
            flash_in: default_flash_in(),
            flash_cache: default_flash_cache(),
            flash_out: default_flash_out(),
            std_in: default_std_in(),
            std_cache: default_std_cache(),
            std_out: default_std_out(),
            divisor: default_divisor(),
            offpeak_factor: default_offpeak_factor(),
        }
    }
}

impl CreditConfig {
    fn peak(ts_ms: i64) -> bool {
        let shanghai = TimeZone::get("Asia/Shanghai").expect("a known zone");
        let Ok(zoned) = Timestamp::from_millisecond(ts_ms).map(|ts| ts.to_zoned(shanghai)) else {
            // A timestamp no calendar can hold claims no peak window; it
            // bills at the discount instead of panicking the ledger.
            return false;
        };
        let weekday = zoned.weekday().to_monday_zero_offset();
        let (hour, minute) = (zoned.hour(), zoned.minute());
        weekday < 5 && (hour as f64 + minute as f64 / 60.0) >= 14.0 && hour < 18
    }

    /// Plan credits for one request, tier and off-peak discount included.
    pub fn credits(&self, rec: &UsageRecord) -> f64 {
        let flash = rec.model_key.contains("flash");
        let (ci, cc, co) = if flash {
            (self.flash_in, self.flash_cache, self.flash_out)
        } else {
            (self.std_in, self.std_cache, self.std_out)
        };
        let raw = (rec.input_net as f64 * ci
            + rec.cache_read as f64 * cc
            + rec.output as f64 * co)
            / self.divisor;
        if Self::peak(rec.ts_ms) {
            raw
        } else {
            raw * self.offpeak_factor
        }
    }
}

/// List price per one million tokens, in its `currency` (default CNY, the
/// unit BigModel and DeepSeek publish). A `usd` card converts through
/// `Billing::usd_cny` so the money ledger stays one number.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub input: f64,
    pub cache_read: f64,
    pub output: f64,
    #[serde(default)]
    pub currency: Option<String>,
}

/// Each vendor's published per-use prices, shipped so the money column works
/// before any config. CNY cards are native; USD cards convert through
/// `Billing::usd_cny`. Tiered or time-of-day pricing takes its dear tier.
/// DeepSeek's retired names bill as their successor. Override or extend in
/// `config.toml`.
pub fn default_prices() -> BTreeMap<String, Price> {
    let card = |input: f64, cache_read: f64, output: f64| Price {
        input,
        cache_read,
        output,
        currency: None,
    };
    let usd = |input: f64, cache_read: f64, output: f64| Price {
        input,
        cache_read,
        output,
        currency: Some("usd".into()),
    };
    [
        // BigModel, CNY per million.
        ("glm-5.3", card(8.0, 2.0, 28.0)),
        ("glm-5.3-flash", card(0.8, 0.23, 2.8)),
        ("glm-5.3-flashx", card(2.0, 0.57, 7.0)),
        ("glm-5.2", card(8.0, 2.0, 28.0)),
        ("glm-5.1", card(8.0, 2.0, 28.0)),
        ("glm-5", card(6.0, 1.5, 22.0)),
        ("glm-5-turbo", card(7.0, 1.8, 26.0)),
        ("glm-4.7", card(4.0, 0.8, 16.0)),
        ("glm-4.7-flashx", card(0.5, 0.1, 3.0)),
        ("glm-4.7-flash", card(0.0, 0.0, 0.0)),
        // Kimi, CNY per million.
        ("kimi-k3", card(20.0, 2.0, 100.0)),
        ("k3", card(20.0, 2.0, 100.0)),
        ("kimi-k2.7-code", card(6.5, 1.3, 27.0)),
        ("kimi-k2.7-code-highspeed", card(13.0, 2.6, 54.0)),
        ("kimi-k2.6", card(6.5, 1.1, 27.0)),
        // DeepSeek, USD per million, peak tier.
        ("deepseek-v4-pro", usd(1.32, 0.044, 3.96)),
        ("deepseek-v4-flash", usd(0.30, 0.006, 1.20)),
        ("deepseek-flash", usd(0.30, 0.006, 1.20)),
        ("deepseek-v4-flash-vision-exp", usd(0.30, 0.006, 1.20)),
        // Anthropic, USD per million.
        ("claude-opus-4.1", usd(15.0, 1.5, 75.0)),
        ("claude-opus-4", usd(15.0, 1.5, 75.0)),
        ("claude-sonnet-4", usd(3.0, 0.3, 15.0)),
        ("claude-sonnet-3.7", usd(3.0, 0.3, 15.0)),
        ("claude-sonnet-3.5", usd(3.0, 0.3, 15.0)),
        ("claude-haiku-3.5", usd(0.8, 0.08, 4.0)),
        ("claude-haiku-3", usd(0.25, 0.03, 1.25)),
        // OpenAI, USD per million, as cited by resellers of the blocked page.
        ("gpt-5.2", usd(1.75, 0.175, 14.0)),
        ("gpt-5.2-codex", usd(1.75, 0.175, 14.0)),
        ("gpt-5.5", usd(5.0, 0.5, 30.0)),
    ]
    .into_iter()
    .map(|(key, price)| (key.to_string(), price))
    .collect()
}

#[derive(Debug, Clone)]
pub struct Billing {
    pub credits: CreditConfig,
    pub prices: BTreeMap<String, Price>,
    pub usd_cny: f64,
}

impl Billing {
    /// What the request would have cost on the API, in CNY.
    pub fn cost_cny(&self, rec: &UsageRecord) -> Option<f64> {
        let price = self.prices.get(&rec.model_key)?;
        let cny = (rec.input_net as f64 / 1e6) * price.input
            + (rec.cache_read as f64 / 1e6) * price.cache_read
            + (rec.output as f64 / 1e6) * price.output;
        let cny = if price.currency.as_deref() == Some("usd") {
            cny * self.usd_cny
        } else {
            cny
        };
        Some(cny)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ts_ms: i64, model: &str, input_net: u64, cache_read: u64, output: u64) -> UsageRecord {
        UsageRecord {
            source: "test".into(),
            session: "s".into(),
            session_title: None,
            provider: None,
            model_raw: model.into(),
            model_key: model.into(),
            ts_ms,
            input_net,
            cache_read,
            cache_write: 0,
            output,
            reasoning: 0,
            ttft_ms: None,
            agent: None,
            failed: false,
            retry: false,
            duration_ms: None,
            tool_calls: 0,
        }
    }

    #[test]
    fn flash_costs_less_than_std() {
        let cfg = CreditConfig::default();
        let flash = rec(0, "glm-5.3-flash", 10_000, 0, 10_000);
        let std = rec(0, "glm-5.3", 10_000, 0, 10_000);
        assert!(cfg.credits(&flash) < cfg.credits(&std));
    }

    #[test]
    fn glm_list_price_math() {
        let mut billing = Billing {
            credits: CreditConfig::default(),
            prices: super::default_prices(),
            usd_cny: 7.2,
        };
        // One million tokens each way of glm-5.3: 8 + 28 = CNY 36.
        let rec_ = rec(0, "glm-5.3", 1_000_000, 0, 1_000_000);
        assert!((billing.cost_cny(&rec_).unwrap() - 36.0).abs() < 1e-9);
        // Kimi's card, native CNY, checks out too.
        let rec_ = rec(0, "kimi-k3", 1_000_000, 0, 1_000_000);
        assert!((billing.cost_cny(&rec_).unwrap() - 120.0).abs() < 1e-9);
        // A usd-denominated card converts through the rate.
        billing.prices.insert(
            "fable".into(),
            Price {
                input: 3.0,
                cache_read: 0.3,
                output: 15.0,
                currency: Some("usd".into()),
            },
        );
        let rec_ = rec(0, "fable", 1_000_000, 0, 1_000_000);
        assert!((billing.cost_cny(&rec_).unwrap() - 18.0 * 7.2).abs() < 1e-9);
    }

    #[test]
    fn offpeak_is_half_of_peak() {
        let cfg = CreditConfig::default();
        // 2026-10-05 is a Monday: 15:00 Shanghai is peak, 20:00 is off-peak.
        let peak = Timestamp::from_millisecond(1_791_183_600_000).expect("in range");
        let off = Timestamp::from_millisecond(1_791_201_600_000).expect("in range");
        let at_peak = rec(peak.as_millisecond(), "glm-5.3", 10_000, 0, 10_000);
        let at_off = rec(off.as_millisecond(), "glm-5.3", 10_000, 0, 10_000);
        assert!((cfg.credits(&at_off) - cfg.credits(&at_peak) * 0.5).abs() < 1e-9);
    }

    // Anchor: 1_791_183_600_000 is Monday 2026-10-05 15:00 Asia/Shanghai;
    // every boundary below is that Monday plus or minus whole hours.
    #[test]
    fn peak_window_boundaries() {
        let monday_14_00 = 1_791_180_000_000;
        let monday_13_59_59_999 = monday_14_00 - 1;
        let monday_17_59 = 1_791_194_340_000;
        let monday_18_00 = 1_791_194_400_000;
        let friday_17_30 = 1_791_538_200_000;
        let friday_18_30 = 1_791_541_800_000;
        let saturday_15_00 = 1_791_615_600_000;
        // The window is [14:00, 18:00) on weekdays only.
        assert!(CreditConfig::peak(monday_14_00));
        assert!(!CreditConfig::peak(monday_13_59_59_999));
        assert!(CreditConfig::peak(monday_17_59));
        assert!(!CreditConfig::peak(monday_18_00));
        assert!(CreditConfig::peak(friday_17_30));
        assert!(!CreditConfig::peak(friday_18_30));
        assert!(!CreditConfig::peak(saturday_15_00));
        // A timestamp no calendar holds claims no peak window.
        assert!(!CreditConfig::peak(i64::MAX));
    }

    #[test]
    fn partial_credits_config_keeps_other_defaults() {
        // One coefficient in config.toml must not require the other seven.
        let parsed: CreditConfig = toml::from_str("std_in = 3.45").expect("one coefficient is enough");
        assert!((parsed.std_in - 3.45).abs() < 1e-9);
        assert!((parsed.std_out - 24.0).abs() < 1e-9);
        assert!((parsed.offpeak_factor - 0.5).abs() < 1e-9);
    }
}
