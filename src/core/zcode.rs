use std::path::Path;

use anyhow::Context;
use rusqlite::{Connection, OpenFlags};

use super::record::UsageRecord;

/// ZCode's per-request ledger: `~/.zcode/cli/db/db.sqlite`, table
/// `model_usage` (the billing-grade source, per zusage.sh), joined to
/// `session` for a human title. The tool ledger's wall time rides the
/// same read-only connection, so one scan answers both questions.
pub fn scan(
    db: &Path,
    ctx: &super::sources::ScanCtx,
) -> anyhow::Result<(Vec<UsageRecord>, u64)> {
    if !db.exists() {
        anyhow::bail!("{} does not exist", db.display());
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let db = Connection::open_with_flags(db, flags).context("open db read-only")?;
    let mut statement = db
        .prepare(
            "SELECT m.started_at, m.model_id, m.provider_id, m.session_id, m.agent,
                    m.input_tokens, m.output_tokens, m.reasoning_tokens,
                    m.cache_creation_input_tokens, m.cache_read_input_tokens,
                    m.time_to_first_token_ms, m.status, m.attempt_index,
                    m.duration_ms, m.tool_call_count, s.title
             FROM model_usage m LEFT JOIN session s ON s.id = m.session_id",
        )
        .context("prepare model_usage query")?;
    let rows = statement
        .query_map([], |row| {
            Ok(Row {
                started_at: row.get(0)?,
                model: row.get(1)?,
                provider: row.get(2)?,
                session: row.get(3)?,
                agent: row.get(4)?,
                input: row.get(5)?,
                output: row.get(6)?,
                reasoning: row.get(7)?,
                cache_write: row.get(8)?,
                cache_read: row.get(9)?,
                ttft: row.get(10)?,
                status: row.get(11)?,
                attempt: row.get(12)?,
                duration: row.get(13)?,
                tools: row.get(14)?,
                title: row.get(15)?,
            })
        })
        .context("query model_usage")?;
    let mut records = Vec::new();
    for row in rows {
        // One bad row (a value outside its column's type) skips that row,
        // the way the JSONL paths skip one bad line — never the whole source.
        let row = match row {
            Ok(row) => row,
            Err(error) => {
                log::warn!("source zcode: one model_usage row skipped: {error}");
                continue;
            }
        };
        if !super::jsonl::ts_in_range(row.started_at) {
            log::warn!("source zcode: model_usage row with time {} out of range skipped", row.started_at);
            continue;
        }
        let ts_ms = row.started_at;
        // ZCode's input_tokens is gross: the cache reads are inside it.
        let input_net = row.input.saturating_sub(row.cache_read);
        records.push(UsageRecord {
            source: "zcode".into(),
            session: row.session,
            session_title: row.title,
            provider: row.provider,
            model_key: ctx.key(&row.model),
            model_raw: row.model,
            ts_ms,
            input_net,
            cache_read: row.cache_read,
            cache_write: row.cache_write,
            output: row.output,
            reasoning: row.reasoning,
            ttft_ms: row.ttft,
            agent: row.agent,
            failed: row.status == "error",
            retry: row.attempt > 0,
            duration_ms: row.duration,
            tool_calls: row.tools,
        });
    }
    // Wall time from the tool ledger, which has no per-request join; the
    // count it selects alongside has no consumer, the sum is what ships.
    let tool_ms = db
        .query_row(
            "SELECT count(*), coalesce(sum(duration_ms), 0) FROM tool_usage",
            [],
            |row| Ok(row.get::<_, i64>(1)?),
        )
        .context("query tool_usage")?
        .max(0) as u64;
    Ok((records, tool_ms))
}

struct Row {
    started_at: i64,
    model: String,
    provider: Option<String>,
    session: String,
    agent: Option<String>,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_write: u64,
    cache_read: u64,
    ttft: Option<u64>,
    status: String,
    attempt: i64,
    duration: Option<u64>,
    tools: u64,
    title: Option<String>,
}
