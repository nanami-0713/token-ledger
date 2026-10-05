use ely_gpui_component::charts::{AreaChart, BarChart, CalendarHeatmap, HeatmapChart, LineChart, Series};
use ely_gpui_component::data_display::{KpiCard, Statistic};
use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, px};

use super::LedgerApp;

/// The front page: today's burn, the week, the cache, and the two charts that
/// tell the whole story — cumulative credits, and daily credits by source.
/// Everything here reads the scan-time cache; no page re-walks records.
pub fn render(app: &mut LedgerApp, _window: &mut Window, cx: &mut Context<LedgerApp>) -> gpui::Div {
    let ledger = &app.ledger;
    let hit = ledger.totals.cache_hit_rate().unwrap_or(0.0) * 100.0;
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .gap_6()
        .max_w(px(1080.))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_4()
                .child(
                    KpiCard::new(
                        Statistic::new("kpi-today", "Today · tokens", ledger.today_tokens)
                            .decimals(0),
                    )
                    .icon(ely_gpui_component::primitives::IconName::Zap)
                    .caption("every model call today, every GUI together"),
                )
                .child(
                    KpiCard::new(
                        Statistic::new("kpi-week", "Last 7 days · tokens", ledger.week_tokens)
                            .decimals(0),
                    )
                    .icon(ely_gpui_component::primitives::IconName::Calendar)
                    .caption("one window, all sources"),
                )
                .child(
                    KpiCard::new(
                        Statistic::new(
                            "kpi-cost",
                            "API list price",
                            ledger.totals.cost_cny.unwrap_or(0.0),
                        )
                        .decimals(0)
                        .prefix("¥"),
                    )
                    .icon(ely_gpui_component::primitives::IconName::Wallet)
                    .caption("tokens × per-model price cards, priced models only"),
                )
                .child(
                    KpiCard::new(
                        Statistic::new("kpi-cache", "Cache hit", hit as f64)
                            .decimals(1)
                            .suffix("%"),
                    )
                    .icon(ely_gpui_component::primitives::IconName::Database)
                    .caption("cache reads over all prompt tokens"),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_4()
                .child(
                    KpiCard::new(
                        Statistic::new(
                            "kpi-credits",
                            "Plan credits today",
                            ledger.today_credits,
                        )
                        .decimals(0),
                    )
                    .icon(ely_gpui_component::primitives::IconName::Gauge)
                    .caption("GLM Coding Plan only — one provider's quota unit, an auxiliary view"),
                )
                .child(
                    KpiCard::new(
                        Statistic::new(
                            "kpi-avg-session",
                            "Tokens a session",
                            ledger.avg_session_tokens,
                        )
                        .decimals(0),
                    )
                    .icon(ely_gpui_component::primitives::IconName::MessageSquare)
                    .caption("prompt + output, averaged over every session"),
                )
                .child(
                    KpiCard::new(
                        Statistic::new(
                            "kpi-errors",
                            "Failed",
                            pct(ledger.totals.errors, ledger.totals.status_seen) * 100.0,
                        )
                        .decimals(1)
                        .suffix("%"),
                    )
                    .icon(ely_gpui_component::primitives::IconName::TriangleAlert)
                    .caption("requests that ended in an error, of the status-reporting ones"),
                )
                .child(
                    KpiCard::new(
                        Statistic::new(
                            "kpi-retries",
                            "Retried",
                            pct(ledger.totals.retries, ledger.totals.status_seen) * 100.0,
                        )
                        .decimals(1)
                        .suffix("%"),
                    )
                    .icon(ely_gpui_component::primitives::IconName::RefreshCw)
                    .caption("requests that ran a second attempt"),
                ),
        )
        .child(section(
            "The last thirty conversations, tokens a second",
            theme.colors.fg_muted,
            div().w(px(1080.)).child(
                BarChart::new("recent-tps", ledger.recent_tps.iter().map(|(label, _)| label.clone()).collect::<Vec<_>>())
                    .series(Series::new(
                        "tokens/s",
                        ledger.recent_tps.iter().map(|(_, tps)| *tps).collect::<Vec<_>>(),
                    ))
                    .format(|value| format!("{value:.0}")),
            ),
        ))
        .child(section(
            "Tokens per day, by source",
            theme.colors.fg_muted,
            div().w(px(980.)).child(by_source_chart(ledger)),
        ))
        .child(section(
            "Cumulative tokens",
            theme.colors.fg_muted,
            div().w(px(980.)).child(
                LineChart::new("cumulative", ledger.days.clone())
                    .series(Series::new("all sources", ledger.cumulative.clone()))
                    .format(|value| human_f64(value)),
            ),
        ))
        .child(section(
            "A year of tokens, a tile a day",
            theme.colors.fg_muted,
            div().w(px(980.)).child(
                CalendarHeatmap::new("calendar", ledger.calendar.0, ledger.calendar.1.clone())
                    .format(|value| format!("{} tokens", human_f64(value))),
            ),
        ))
        .child(section(
            "When the tokens burn: the last two weeks, hour by hour",
            theme.colors.fg_muted,
            div().w(px(980.)).child(hourly_chart(ledger)),
        ))
        .child(section(
            "Where the tokens go",
            theme.colors.fg_muted,
            div().w(px(680.)).child(
                BarChart::new(
                    "token-mix",
                    ["Cache reads", "Fresh input", "Output"],
                )
                .series(Series::new(
                    "Tokens",
                    vec![
                        ledger.totals.cache_read as f64,
                        ledger.totals.input_net as f64,
                        ledger.totals.output as f64,
                    ],
                ))
                .horizontal()
                .format(|value| human_tokens(value as u64)),
            ),
        ))
        .child(
            div()
                .text_size(gpui::rems(0.8))
                .text_color(theme.colors.fg_muted)
                .child(format!(
                    "{} days on record · cache {} of {} prompt tokens · {:.1}M output tokens · API list price \u{a5}{} (priced models)",
                    ledger.days.len(),
                    human_tokens(ledger.totals.cache_read),
                    human_tokens(ledger.totals.cache_read + ledger.totals.input_net),
                    ledger.totals.output as f64 / 1e6,
                    ledger
                        .totals
                        .cost_cny
                        .map(|cny| format!("{cny:.0}"))
                        .unwrap_or_else(|| "0".into()),
                )),
        )
}

/// One stacked area per source that actually has data — a chart series
/// without a value per label is a panic, so absent sources stay absent.
fn by_source_chart(ledger: &crate::core::aggregate::Ledger) -> impl IntoElement {
    const NAMES: [(&str, &str); 4] = [
        ("zcode", "ZCode"),
        ("codex", "ChatGPT"),
        ("dsh", "DeepSeek Harness"),
        ("claude-code", "Claude Code"),
    ];
    let mut chart = AreaChart::new("by-source", ledger.days.clone());
    for (key, label) in NAMES {
        if let Some(values) = ledger.by_source.get(key) {
            if values.len() == ledger.days.len() {
                chart = chart.series(Series::new(label, values.clone()));
            }
        }
    }
    chart.stacked().format(|value| human_f64(value))
}

/// Fourteen rows, one a day, twenty-four columns, one an hour: when the
/// machine actually burns its quota. Rows without data stay as empty grids
/// rather than panicking the chart.
fn hourly_chart(ledger: &crate::core::aggregate::Ledger) -> impl IntoElement {
    let hours: Vec<String> = (0..24).map(|hour| format!("{hour:02}")).collect();
    let mut chart = HeatmapChart::new("hourly", hours);
    for (day, row) in &ledger.hourly {
        let label = day.strip_prefix("20").unwrap_or(day).to_string();
        chart = chart.row(label, row.iter().copied());
    }
    chart.format(|value| human_f64(value))
}

fn section(title: &str, muted: gpui::Hsla, chart: gpui::Div) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(gpui::rems(0.9))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(muted)
                .child(title.to_string()),
        )
        .child(chart)
}

fn human_f64(value: f64) -> String {
    if value >= 1e9 {
        format!("{value:.2}B")
    } else if value >= 1e6 {
        format!("{value:.1}M")
    } else if value >= 1e3 {
        format!("{value:.0}k")
    } else {
        format!("{value:.0}")
    }
}

fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

fn human_tokens(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.2}B", value as f64 / 1e9)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1e6)
    } else {
        format!("{value}")
    }
}
