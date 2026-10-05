use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use super::billing::{Billing, CreditConfig, Price};
use super::jsonl::JsonlQuery;
use super::record::{normalize_model, UsageRecord};

/// How one tool's log becomes records. Built-in sources are defined here in
/// code; the user's `config.toml` overrides them by id and adds new ones, so
/// any CLI or GUI that writes JSONL logs can join the ledger without a
/// release.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum SourceKind {
    Zcode {
        #[serde(default = "default_zcode_db")]
        db: String,
    },
    Dsh {
        #[serde(default = "default_dsh_root")]
        root: String,
    },
    Jsonl {
        paths: Vec<String>,
        #[serde(default)]
        zstd: bool,
        /// True when the source's `input` number already contains the cache
        /// reads (OpenAI- and DeepSeek-style reports); false when they are
        /// separate line items (Anthropic-style).
        #[serde(default = "default_true")]
        input_includes_cache: bool,
        /// Field paths into one JSON line, dotted (`message.usage.input_tokens`).
        model: Option<String>,
        /// Reuse the last seen `model` value when a line has none (streamed
        /// logs state the model once per turn).
        #[serde(default)]
        sticky_model: bool,
        time: String,
        input: String,
        output: String,
        cache_read: Option<String>,
        cache_write: Option<String>,
    },
}

fn default_zcode_db() -> String {
    "~/.zcode/cli/db/db.sqlite".into()
}
fn default_dsh_root() -> String {
    "~/.dsh/sessions".into()
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDef {
    pub id: String,
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: SourceKind,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub source: Vec<SourceDef>,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub credits: Option<CreditConfig>,
    #[serde(default)]
    pub prices: BTreeMap<String, Price>,
}

impl Config {
    /// Built-in sources, tried even with no config file.
    pub fn builtin() -> Vec<SourceDef> {
        vec![
            SourceDef {
                id: "zcode".into(),
                label: "ZCode".into(),
                enabled: true,
                kind: SourceKind::Zcode {
                    db: default_zcode_db(),
                },
            },
            SourceDef {
                id: "dsh".into(),
                label: "DeepSeek Harness".into(),
                enabled: true,
                kind: SourceKind::Dsh {
                    root: default_dsh_root(),
                },
            },
            SourceDef {
                id: "codex".into(),
                label: "Codex".into(),
                enabled: true,
                kind: SourceKind::Jsonl {
                    paths: vec!["~/.codex/sessions/**/*.jsonl".into()],
                    zstd: false,
                    input_includes_cache: true,
                    model: Some("payload.model".into()),
                    sticky_model: true,
                    time: "timestamp".into(),
                    input: "payload.info.last_token_usage.input_tokens".into(),
                    output: "payload.info.last_token_usage.output_tokens".into(),
                    cache_read: Some("payload.info.last_token_usage.cached_input_tokens".into()),
                    cache_write: None,
                },
            },
            SourceDef {
                id: "claude-code".into(),
                label: "Claude Code".into(),
                enabled: true,
                kind: SourceKind::Jsonl {
                    paths: vec!["~/.claude/projects/**/*.jsonl".into()],
                    zstd: false,
                    input_includes_cache: false,
                    model: Some("message.model".into()),
                    sticky_model: false,
                    time: "timestamp".into(),
                    input: "message.usage.input_tokens".into(),
                    output: "message.usage.output_tokens".into(),
                    cache_read: Some("message.usage.cache_read_input_tokens".into()),
                    cache_write: Some("message.usage.cache_creation_input_tokens".into()),
                },
            },
        ]
    }

    /// Load `~/.config/token-ledger/config.toml` over the built-ins: same-id
    /// definitions replace, new ids append.
    pub fn load() -> Self {
        let mut config = Self::default();
        let path = config_path();
        if let Ok(text) = std::fs::read_to_string(&path) {
            match toml::from_str::<Config>(&text) {
                Ok(user) => config = user,
                Err(error) => log::warn!("config: {path:?} failed to parse: {error}"),
            }
        }
        let mut sources = Self::builtin();
        for def in config.source {
            if let Some(slot) = sources.iter_mut().find(|b| b.id == def.id) {
                *slot = def;
            } else {
                sources.push(def);
            }
        }
        config.source = sources;
        config
    }

    pub fn billing(&self) -> Billing {
        Billing {
            credits: self.credits.clone().unwrap_or_default(),
            prices: self.prices.clone(),
        }
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(dir) = std::env::var("TOKEN_LEDGER_CONFIG_DIR") {
        return PathBuf::from(dir).join("config.toml");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/token-ledger/config.toml")
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// What one source found on the last scan, for the Sources page.
#[derive(Debug, Clone)]
pub struct SourceStatus {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub present: bool,
    pub records: usize,
    pub detail: String,
}

/// Everything a scan needs to normalize records on the way in.
pub struct ScanCtx {
    pub aliases: BTreeMap<String, String>,
}

impl ScanCtx {
    pub fn key(&self, raw: &str) -> String {
        normalize_model(raw, &self.aliases)
    }
}

/// Scan every enabled source. A broken source reports itself and gets out of
/// the way; the rest of the ledger still loads.
pub fn scan_all(config: &Config) -> (Vec<UsageRecord>, Vec<SourceStatus>) {
    let ctx = ScanCtx {
        aliases: config.aliases.clone(),
    };
    let mut records = Vec::new();
    let mut statuses = Vec::new();
    for def in &config.source {
        if !def.enabled {
            statuses.push(SourceStatus {
                id: def.id.clone(),
                label: def.label.clone(),
                enabled: false,
                present: false,
                records: 0,
                detail: "off".into(),
            });
            continue;
        }
        let outcome = scan_source(def, &ctx);
        let (found, detail) = match &outcome {
            Ok(found) => (found.len(), detail_for(def, found)),
            Err(error) => (0, format!("{error:#}")),
        };
        statuses.push(SourceStatus {
            id: def.id.clone(),
            label: def.label.clone(),
            enabled: true,
            present: found > 0,
            records: found,
            detail,
        });
        if let Ok(found) = outcome {
            records.extend(found);
        }
    }
    records.sort_by_key(|rec| rec.ts_ms);
    (records, statuses)
}

fn detail_for(def: &SourceDef, found: &[UsageRecord]) -> String {
    let files = found
        .iter()
        .map(|rec| rec.session.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    match &def.kind {
        SourceKind::Zcode { .. } => format!("sqlite · {} sessions", files),
        SourceKind::Dsh { .. } => format!("zstd jsonl · {} sessions", files),
        SourceKind::Jsonl { .. } => format!("jsonl · {} sessions", files),
    }
}

fn scan_source(def: &SourceDef, ctx: &ScanCtx) -> anyhow::Result<Vec<UsageRecord>> {
    match &def.kind {
        SourceKind::Zcode { db } => super::zcode::scan(&expand_tilde(db), ctx),
        SourceKind::Dsh { root } => Ok(super::jsonl::scan_dsh(&expand_tilde(root), ctx).0),
        SourceKind::Jsonl {
            paths,
            zstd,
            input_includes_cache,
            model,
            sticky_model,
            time,
            input,
            output,
            cache_read,
            cache_write,
        } => super::jsonl::scan_generic(JsonlQuery {
            source_id: &def.id,
            paths,
            zstd: *zstd,
            input_includes_cache: *input_includes_cache,
            model: model.as_deref(),
            sticky_model: *sticky_model,
            time,
            input,
            output,
            cache_read: cache_read.as_deref(),
            cache_write: cache_write.as_deref(),
            aliases: &ctx.aliases,
        }),
    }
}
