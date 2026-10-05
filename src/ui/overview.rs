use ely_gpui_component::charts::{AreaChart, BarChart, CalendarHeatmap, HeatmapChart, LineChart, Series};
use ely_gpui_component::data_display::{KpiCard, Statistic};
use ely_gpui_component::primitives::{IconName, Tooltip};
use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    Window, div, px,
};

use super::LedgerApp;
use crate::core::aggregate::Ledger;

/// The front page. Tiles carry a name and a number; the long explanation of
/// each metric waits on its hover. Everything reads the scan-time cache.
pub fn render(app: &mut LedgerApp, _window: &mut Window, cx: &mut Context<LedgerApp>) -> gpui::Div {
    let ledger = &app.ledger;
    let hit = ledger.totals.cache_hit_rate().unwrap_or(0.0) * 100.0;
    let tool_hours = ledger.tool_ms as f64 / 3_600_000.0;
    let tool_tip = format!(
        "{} tool calls while serving these requests; {:.1}h of tool wall time in total (ZCode's tool ledger)",
        ledger.totals.tool_calls, tool_hours
    );
    let theme = cx.theme();
    struct Tile {
        name: &'static str,
        tip: String,
        value: f64,
        prefix: Option<&'static str>,
        percent: bool,
        icon: IconName,
    }
    let tiles = vec![
        Tile {
            name: "Today · tokens",
            tip: "every model call today, every GUI together, prompt and output counted once".into(),
            value: ledger.today_tokens,
            prefix: None,
            percent: false,
            icon: IconName::Zap,
        },
        Tile {
            name: "7 days · tokens",
            tip: "the same count over the trailing week".into(),
            value: ledger.week_tokens,
            prefix: None,
            percent: false,
            icon: IconName::Calendar,
        },
        Tile {
            name: "API list price",
            tip: "the ledger's tokens priced by each model's official card, in CNY; models without a card are excluded, never guessed".into(),
            value: ledger.totals.cost_cny.unwrap_or(0.0),
            prefix: Some("¥"),
            percent: false,
            icon: IconName::Wallet,
        },
        Tile {
            name: "Cache hit",
            tip: "cache reads over all prompt tokens: how much of every prompt the provider already had".into(),
            value: hit,
            prefix: None,
            percent: true,
            icon: IconName::Database,
        },
        Tile {
            name: "Tool calls",
            tip: tool_tip,
            value: ledger.totals.tool_calls as f64,
            prefix: None,
            percent: false,
            icon: IconName::Wrench,
        },
        Tile {
            name: "Plan credits today",
            tip: "the GLM Coding Plan's own quota unit — one provider's subscription meter, an auxiliary view, never the ledger's unit".into(),
            value: ledger.today_credits,
            prefix: None,
            percent: false,
            icon: IconName::Gauge,
        },
        Tile {
            name: "Failed",
            tip: "requests that ended in an error, as a share of the requests whose source reports a status".into(),
            value: pct(ledger.totals.errors, ledger.totals.status_seen) * 100.0,
            prefix: None,
            percent: true,
            icon: IconName::TriangleAlert,
        },
        Tile {
            name: "Retried",
            tip: "requests that ran a second attempt after the first one did not land".into(),
            value: pct(ledger.totals.retries, ledger.totals.status_seen) * 100.0,
            prefix: None,
            percent: true,
            icon: IconName::RefreshCw,
        },
        Tile {
            name: "Tokens a session",
            tip: "prompt plus output tokens, averaged over every session on record".into(),
            value: ledger.avg_session_tokens,
            prefix: None,
            percent: false,
            icon: IconName::MessageSquare,
        },
    ];
    // Three rows of three, each tile flexing to a third of the row: the
    // count above must stay a multiple of three or the grid comes out ragged.
    let grid = div().flex().flex_col().gap_3().children((0..3).map(|row| {
        div().flex().gap_3().children((0..3).map(|col| {
            let ix = row * 3 + col;
            let tile = &tiles[ix];
            let mut stat = Statistic::new(("kpi", ix), tile.name, tile.value).decimals(0);
            if let Some(prefix) = tile.prefix {
                stat = stat.prefix(prefix);
            }
            if tile.percent {
                stat = stat.decimals(1).suffix("%");
            }
            let (name, tip) = (tile.name, tile.tip.clone());
            div()
                .id(("kpi-tile", ix))
                .flex_1()
                .min_w_0()
                // with_meta lays title and hint on one row, which ellipsizes a
                // sentence; rich + a column lets the tip wrap inside the cap.
                .tooltip(Tooltip::rich(move |_, cx| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(name),
                        )
                        .child(
                            div()
                                .text_color(cx.theme().colors.tooltip_fg.opacity(0.6))
                                .child(tip.clone()),
                        )
                        .into_any_element()
                }))
                .child(KpiCard::new(stat).icon(tile.icon))
        }))
    }));
    div()
        .flex()
        .flex_col()
        .gap_6()
        .max_w(px(1080.))
        .child(grid)
        .child(section(
            "Tokens per day, by source",
            theme.colors.fg_muted,
            div().w(px(980.)).child(by_source_chart(ledger)),
        ))
        .child(section(
            "The last thirty conversations, tokens a second",
            theme.colors.fg_muted,
            div().w(px(1080.)).child(
                BarChart::new(
                    "recent-tps",
                    ledger
                        .recent_tps
                        .iter()
                        .map(|(label, _)| label.clone())
                        .collect::<Vec<_>>(),
                )
                .series(Series::new(
                    "tokens/s",
                    ledger.recent_tps.iter().map(|(_, tps)| *tps).collect::<Vec<_>>(),
                ))
                .format(|value| format!("{value:.0}")),
            ),
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
                BarChart::new("token-mix", ["Cache reads", "Fresh input", "Output"])
                    .series(Series::new(
                        "Tokens",
                        vec![
                            ledger.totals.cache_read as f64,
                            ledger.totals.input_net as f64,
                            ledger.totals.output as f64,
                        ],
                    ))
                    .horizontal()
                    .format(|value| human_f64(value)),
            ),
        ))
        .child(
            div()
                .text_size(gpui::rems(0.8))
                .text_color(theme.colors.fg_muted)
                .child(format!(
                    "{} days on record · cache {} of {} prompt tokens · {:.1}M output tokens · API list price ¥{} (priced models)",
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
fn by_source_chart(ledger: &Ledger) -> impl IntoElement {
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
/// machine actually burns its quota.
fn hourly_chart(ledger: &Ledger) -> impl IntoElement {
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

fn human_tokens(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.2}B", value as f64 / 1e9)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1e6)
    } else {
        format!("{value}")
    }
}

fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}
