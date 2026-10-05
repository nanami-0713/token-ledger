use ely_gpui_component::charts::{AreaChart, LineChart, Series};
use ely_gpui_component::data_display::{KpiCard, Statistic};
use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, px};

use super::LedgerApp;
use crate::core::aggregate::{day_of, Daily, Totals};

/// The front page: today's burn, the week, the cache, and the two charts that
/// tell the whole story — cumulative credits, and daily credits by source.
pub fn render(app: &mut LedgerApp, _window: &mut Window, cx: &mut Context<LedgerApp>) -> gpui::Div {
    let billing = &app.billing;
    let records = &app.records;
    let today = jiff::Zoned::now().strftime("%Y-%m-%d").to_string();
    let mut all = Totals::default();
    let mut today_credits = 0.0;
    let mut week_credits = 0.0;
    let today_ms = jiff::Zoned::now().timestamp().as_millisecond();
    for rec in records {
        all.add(rec, billing);
        let credits = billing.credits.credits(rec);
        if day_of(rec.ts_ms) == today {
            today_credits += credits;
        }
        if today_ms - rec.ts_ms <= 7 * 24 * 3600 * 1000 {
            week_credits += credits;
        }
    }
    let by_source = Daily::build(records, |rec| rec.source.clone(), "credits", billing);
    let mut cumulative = vec![0.0; by_source.days.len()];
    let mut running = 0.0;
    for column in by_source.series.values() {
        for (ix, value) in column.iter().enumerate() {
            cumulative[ix] += value;
        }
    }
    for value in cumulative.iter_mut() {
        running += *value;
        *value = running;
    }
    let hit = all.cache_hit_rate().unwrap_or(0.0) * 100.0;
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
                        Statistic::new("kpi-today", "Today (credits)", today_credits)
                            .decimals(0),
                    )
                    .icon(ely_gpui_component::primitives::IconName::Zap)
                    .caption("plan credits burned today, every GUI together"),
                )
                .child(
                    KpiCard::new(
                        Statistic::new("kpi-week", "Last 7 days", week_credits).decimals(0),
                    )
                    .icon(ely_gpui_component::primitives::IconName::Calendar)
                    .caption("one window, all sources"),
                )
                .child(
                    KpiCard::new(
                        Statistic::new("kpi-requests", "Requests", all.requests as f64)
                            .decimals(0),
                    )
                    .icon(ely_gpui_component::primitives::IconName::Activity)
                    .caption("model calls on this machine"),
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
        .child(section(
            "Credits per day, by source",
            theme.colors.fg_muted,
            div().w(px(980.)).child(
                AreaChart::new("by-source", by_source.days.clone())
                    .series(Series::new(
                        "ZCode",
                        by_source.series.get("zcode").cloned().unwrap_or_default(),
                    ))
                    .series(Series::new(
                        "DeepSeek Harness",
                        by_source.series.get("dsh").cloned().unwrap_or_default(),
                    ))
                    .stacked()
                    .format(|value| format!("{value:.0}")),
            ),
        ))
        .child(section(
            "Cumulative credits",
            theme.colors.fg_muted,
            div().w(px(980.)).child(
                LineChart::new("cumulative", by_source.days.clone())
                    .series(Series::new("all sources", cumulative))
                    .format(|value| format!("{value:.0}")),
            ),
        ))
        .child(
            div()
                .text_size(gpui::rems(0.8))
                .text_color(theme.colors.fg_muted)
                .child(format!(
                    "{} days on record · cache {} of {} prompt tokens · {:.1}M output tokens",
                    by_source.days.len(),
                    human_tokens(all.cache_read),
                    human_tokens(all.cache_read + all.input_net),
                    all.output as f64 / 1e6,
                )),
        )
}

fn section(title: &str, muted: gpui::Hsla, chart: gpui::Div) -> gpui::Div {
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

fn human_tokens(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.2}B", value as f64 / 1e9)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1e6)
    } else {
        format!("{value}")
    }
}
