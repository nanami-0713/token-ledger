use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use anyhow::{Context, bail};
use serde_json::Value;

use super::record::{normalize_model, UsageRecord};

/// One declarative question to ask of every JSON line of a log.
pub struct JsonlQuery<'a> {
    pub source_id: &'a str,
    pub paths: &'a [String],
    pub zstd: bool,
    pub input_includes_cache: bool,
    pub model: Option<&'a str>,
    pub sticky_model: bool,
    pub time: &'a str,
    pub input: &'a str,
    pub output: &'a str,
    pub cache_read: Option<&'a str>,
    pub cache_write: Option<&'a str>,
    pub aliases: &'a BTreeMap<String, String>,
}

/// Walk a dotted path (`data.chunk.usage.inputTokens`) through a JSON object.
fn dig<'v>(value: &'v Value, path: &str) -> Option<&'v Value> {
    let mut at = value;
    for key in path.split('.') {
        at = at.get(key)?;
    }
    Some(at)
}

fn as_u64(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

/// Epoch milliseconds from whatever the log wrote: epoch millis, epoch
/// seconds, or an RFC 3339 string.
fn epoch_ms(value: &Value) -> anyhow::Result<i64> {
    if let Some(number) = value.as_f64() {
        if number >= 1e11 {
            return Ok(number as i64);
        }
        if number >= 1e9 {
            return Ok((number * 1000.0) as i64);
        }
        bail!("time {number} is not an epoch");
    }
    if let Some(text) = value.as_str() {
        return Ok(text
            .parse::<jiff::Timestamp>()
            .context("time is not RFC 3339")?
            .as_millisecond());
    }
    bail!("time is neither number nor string")
}

pub fn scan_generic(query: JsonlQuery) -> anyhow::Result<Vec<UsageRecord>> {
    let mut records = Vec::new();
    let mut model_files = 0;
    for pattern in query.paths {
        let pattern = super::sources::expand_tilde(pattern);
        let pattern = pattern
            .to_str()
            .with_context(|| format!("{pattern:?} is not UTF-8"))?;
        for entry in glob::glob(pattern).with_context(|| format!("bad glob {pattern}"))? {
            let path = entry.with_context(|| "one glob entry failed")?;
            model_files += 1;
            scan_file(&path, &query, &mut records);
        }
    }
    if model_files == 0 {
        bail!("no file matches");
    }
    Ok(records)
}

fn scan_file(path: &Path, query: &JsonlQuery, records: &mut Vec<UsageRecord>) {
    let open = || -> anyhow::Result<Box<dyn Read>> {
        let file = File::open(path).with_context(|| format!("{path:?}"))?;
        if query.zstd {
            Ok(Box::new(zstd::stream::read::Decoder::new(file)?))
        } else {
            Ok(Box::new(file))
        }
    };
    let reader = match open() {
        Ok(reader) => BufReader::new(reader),
        Err(error) => {
            log::warn!("source {}: {error:#}", query.source_id);
            return;
        }
    };
    // DSH names the session by directory, Claude and Codex by file.
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let session = if parent.starts_with("session-") {
        parent.clone()
    } else {
        file_stem.clone()
    };
    // The workdir slug above a DSH session (`--Users-nanami-Desktop-`)
    // decodes to the project path the session ran in.
    let title = if parent.starts_with("session-") {
        path.parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .map(|slug| slug.replace('-', "/"))
    } else {
        None
    };
    let mut sticky: Option<String> = None;
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                log::warn!("source {}: {path:?}: {error}", query.source_id);
                break;
            }
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if query.sticky_model {
            if let Some(found) = query.model.and_then(|m| dig(&value, m)) {
                if let Some(name) = found.as_str() {
                    sticky = Some(name.to_string());
                }
            }
        }
        let model_raw = match query.model.and_then(|m| dig(&value, m)).and_then(Value::as_str) {
            Some(name) => name.to_string(),
            None => match &sticky {
                Some(name) => name.clone(),
                None => continue,
            },
        };
        let (Some(in_value), Some(out_value)) = (dig(&value, query.input), dig(&value, query.output))
        else {
            continue;
        };
        let Some(time_value) = dig(&value, query.time) else {
            continue;
        };
        let Ok(ts_ms) = epoch_ms(time_value) else {
            continue;
        };
        let (input, cache_read) = if query.input_includes_cache {
            let gross = as_u64(Some(in_value));
            let cached = as_u64(query.cache_read.and_then(|p| dig(&value, p)));
            (gross.saturating_sub(cached), cached)
        } else {
            (
                as_u64(Some(in_value)),
                as_u64(query.cache_read.and_then(|p| dig(&value, p))),
            )
        };
        records.push(UsageRecord {
            source: query.source_id.into(),
            session: session.clone(),
            session_title: title.clone(),
            provider: None,
            model_key: normalize_model(&model_raw, query.aliases),
            model_raw,
            ts_ms,
            input_net: input,
            cache_read,
            cache_write: as_u64(query.cache_write.and_then(|p| dig(&value, p))),
            output: as_u64(Some(out_value)),
            reasoning: 0,
            ttft_ms: None,
            agent: None,
        });
    }
}

/// DeepSeek Harness: `~/.dsh/sessions/<workdir-slug>/session-<uuid>/session.jsonl.zstd`,
/// usage riding on `data.chunk.usage` chunks, the model stated per turn.
pub fn scan_dsh(root: &Path, ctx: &super::sources::ScanCtx) -> (Vec<UsageRecord>, usize) {
    // <workdir-slug>/session-<uuid>/session.jsonl.zstd under the root.
    let paths = [format!("{}/%/session-*/session.jsonl.zstd", root.display()).replace('%', "*")];
    let files = glob::glob(&paths[0])
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(0);
    let query = JsonlQuery {
        source_id: "dsh",
        paths: &paths,
        zstd: true,
        input_includes_cache: true,
        model: Some("data.model"),
        sticky_model: true,
        time: "time",
        input: "data.chunk.usage.inputTokens",
        output: "data.chunk.usage.outputTokens",
        cache_read: Some("data.chunk.usage.cacheReadTokens"),
        cache_write: None,
        aliases: &ctx.aliases,
    };
    match scan_generic(query) {
        Ok(found) => (found, files),
        Err(error) => {
            log::warn!("source dsh: {error:#}");
            (Vec::new(), files)
        }
    }
}
