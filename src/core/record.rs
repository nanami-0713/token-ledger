use std::collections::BTreeMap;

/// One billable model call, in canonical units shared by every source.
///
/// Token semantics: `input_net` is fresh prompt text; cache reads are counted
/// separately. Sources whose API reports gross prompt tokens (ZCode, DSH,
/// OpenAI-style) have the cache part split off at parse time.
#[derive(Debug, Clone)]
pub struct UsageRecord {
    pub source: String,
    pub session: String,
    pub session_title: Option<String>,
    pub provider: Option<String>,
    pub model_raw: String,
    pub model_key: String,
    pub ts_ms: i64,
    pub input_net: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    pub reasoning: u64,
    pub ttft_ms: Option<u64>,
    pub agent: Option<String>,
    /// The request ended in an error (sources without a status say false).
    pub failed: bool,
    /// The request ran again after a first attempt.
    pub retry: bool,
    /// Tool calls made while serving this request, when the source counts them.
    pub tool_calls: u64,
    /// Wall time the provider took, when the source records it.
    pub duration_ms: Option<u64>,
}

impl UsageRecord {
    /// Gross prompt size as the API saw it.
    pub fn input_gross(&self) -> u64 {
        self.input_net + self.cache_read
    }
}

/// Strip a provider prefix (`anthropic/`, `bigmodel/`), a date-like release
/// suffix and case, so one model's spellings land in one bucket. Aliases map
/// cleaned spellings onto a chosen key; nothing here ever merges two models.
pub fn normalize_model(raw: &str, aliases: &BTreeMap<String, String>) -> String {
    let lowered = raw.trim().to_lowercase();
    let unprefixed = lowered.rsplit('/').next().unwrap_or(&lowered);
    let cleaned = strip_date_suffix(unprefixed);
    if let Some(key) = aliases.get(&cleaned) {
        return key.clone();
    }
    cleaned
}

/// Remove a trailing `-YYYYMMDD` (optionally with more suffix segments after
/// it, as in `model-20250929-highspeed`).
fn strip_date_suffix(name: &str) -> String {
    let segments: Vec<&str> = name.split('-').collect();
    let mut cut = segments.len();
    for (ix, segment) in segments.iter().enumerate() {
        if segment.len() == 8 && segment.bytes().all(|b| b.is_ascii_digit()) {
            cut = ix;
            break;
        }
    }
    if cut == 0 || cut >= segments.len() {
        return name.to_string();
    }
    segments[..cut].join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aliases() -> BTreeMap<String, String> {
        [("glm-5.3", "glm-5.3")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn same_model_spells_one_key() {
        let aliases = aliases();
        let a = normalize_model("GLM-5.3", &aliases);
        let b = normalize_model("glm-5.3", &aliases);
        let c = normalize_model("bigmodel/glm-5.3", &aliases);
        assert_eq!(a, "glm-5.3");
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn different_models_stay_apart() {
        let aliases = aliases();
        assert_ne!(
            normalize_model("anthropic/claude-fable-5.1", &aliases),
            normalize_model("GLM-5.3", &aliases)
        );
        assert_eq!(
            normalize_model("anthropic/claude-fable-5.1", &aliases),
            "claude-fable-5.1"
        );
    }

    #[test]
    fn date_suffixes_fold() {
        let aliases = aliases();
        assert_eq!(
            normalize_model("claude-sonnet-4-5-20250929", &aliases),
            "claude-sonnet-4-5"
        );
    }
}
