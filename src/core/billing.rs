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
    pub flash_in: f64,
    pub flash_cache: f64,
    pub flash_out: f64,
    /// Coefficients per 10k tokens, every other model.
    pub std_in: f64,
    pub std_cache: f64,
    pub std_out: f64,
    pub divisor: f64,
    /// Peak window is full price; outside it this factor applies.
    pub offpeak_factor: f64,
}

impl Default for CreditConfig {
    fn default() -> Self {
        // The GLM Coding Plan formula, verified against zusage.sh.
        Self {
            flash_in: 2.3,
            flash_cache: 0.56,
            flash_out: 8.0,
            std_in: 6.9,
            std_cache: 1.7,
            std_out: 24.0,
            divisor: 10_000.0,
            offpeak_factor: 0.5,
        }
    }
}

impl CreditConfig {
    fn peak(ts_ms: i64) -> bool {
        let shanghai = TimeZone::get("Asia/Shanghai").expect("a known zone");
        let zoned = Timestamp::from_millisecond(ts_ms)
            .expect("a time in range")
            .to_zoned(shanghai);
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

/// USD per one million tokens.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub input: f64,
    pub cache_read: f64,
    pub output: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Billing {
    pub credits: CreditConfig,
    pub prices: BTreeMap<String, Price>,
}

impl Billing {
    pub fn usd(&self, rec: &UsageRecord) -> Option<f64> {
        let price = self.prices.get(&rec.model_key)?;
        Some(
            (rec.input_net as f64 / 1e6) * price.input
                + (rec.cache_read as f64 / 1e6) * price.cache_read
                + (rec.output as f64 / 1e6) * price.output,
        )
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
    fn offpeak_is_half_of_peak() {
        let cfg = CreditConfig::default();
        // 2026-10-05 is a Monday: 15:00 Shanghai is peak, 20:00 is off-peak.
        let peak = Timestamp::from_millisecond(1_791_183_600_000).expect("in range");
        let off = Timestamp::from_millisecond(1_791_201_600_000).expect("in range");
        let at_peak = rec(peak.as_millisecond(), "glm-5.3", 10_000, 0, 10_000);
        let at_off = rec(off.as_millisecond(), "glm-5.3", 10_000, 0, 10_000);
        assert!((cfg.credits(&at_off) - cfg.credits(&at_peak) * 0.5).abs() < 1e-9);
    }
}
