use ely_gpui_component::buttons::SegmentedControl;
use ely_gpui_component::charts::{AreaChart, BarChart, Series};
use ely_gpui_component::tables::{Cell, Column, DataTable, Row};
use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, IntoElement, ParentElement, SharedString, Styled, Window, div, px, rems};

use super::LedgerApp;
use crate::core::aggregate::{Ledger, ModelRow, Span};

/// The merge, on show, in any lookback window: every model one row and one
/// bar in each of the per-model charts — tokens a day, speed, cache, and
/// what it would cost at list price.
pub fn render(app: &mut LedgerApp, _window: &mut Window, cx: &mut Context<LedgerApp>) -> gpui::Div {
    let ledger = &app.span_ledger;
    let entity = cx.entity();
    let span = app.span;
    let mut windows = SegmentedControl::new("span", SharedString::from(span.label()));
    for choice in Span::ALL {
        let value = SharedString::from(choice.label());
        windows = windows.segment(value, SharedString::from(choice.label()), None);
    }
    windows = windows.on_change(move |value, _, cx| {
        let next = Span::ALL
            .iter()
            .copied()
            .find(|choice| choice.label() == value.as_ref())
            .unwrap_or(Span::All);
        entity.update(cx, |app, cx| app.set_span(next, cx));
    });

    // One bar per model that has the answer for that bar; absent answers
    // stay absent.
    let timed: Vec<&ModelRow> = ledger.models_rows.iter().filter(|r| r.tps.is_some()).collect();
    let cached: Vec<&ModelRow> = ledger
        .models_rows
        .iter()
        .filter(|r| r.totals.cache_hit_rate().is_some())
        .collect();
    let priced: Vec<&ModelRow> = ledger
        .models_rows
        .iter()
        .filter(|r| r.totals.cost_cny.is_some())
        .collect();
    let names =
        |rows: &[&ModelRow]| rows.iter().map(|r| r.model.clone()).collect::<Vec<_>>();

    let table_rows: Vec<Row> = ledger
        .models_rows
        .iter()
        .map(|row| {
            let tokens = row.totals.input_net + row.totals.cache_read + row.totals.output;
            Row::new(
                row.model.clone(),
                [
                    Cell::Text(row.model.clone().into()),
                    Cell::Text(row.sources.join(" + ").into()),
                    Cell::Number(row.totals.requests as f64),
                    Cell::Number(tokens as f64),
                    match row.totals.cost_cny {
                        Some(cny) => Cell::Number(cny),
                        None => Cell::from(""),
                    },
                    Cell::Number(row.totals.credits),
                    match row.tps {
                        Some(tps) => Cell::Number(tps),
                        None => Cell::from(""),
                    },
                    Cell::Number(row.totals.cache_hit_rate().unwrap_or(0.0) * 100.0),
                ],
            )
        })
        .collect();
    let theme = cx.theme();
    let mut stacked = AreaChart::new("models-days", ledger.days.clone());
    for row in ledger.models_rows.iter().take(6) {
        if let Some(values) = ledger.by_model.get(&row.model) {
            if values.len() == ledger.days.len() {
                stacked = stacked.series(Series::new(row.model.clone(), values.clone()));
            }
        }
    }
    stacked = stacked.stacked().format(human);
    div()
        .flex()
        .flex_col()
        .gap_6()
        .max_w(px(1080.))
        .child(
            div()
                .flex()
                .items_center()
                .gap_4()
                .child(
                    div()
                        .text_size(rems(0.9))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.colors.fg_muted)
                        .child(format!("Per model, over {}", span.label())),
                )
                .child(windows),
        )
        .child(section_title("Tokens per day, stacked", theme.colors.fg_muted)
            .child(div().w(px(980.)).child(stacked)))
        .child(section_title("Speed: output tokens a second, by model", theme.colors.fg_muted)
            .child(div().w(px(980.)).child(
                BarChart::new("models-tps", names(&timed))
                    .series(Series::new(
                        "tokens/s",
                        timed.iter().map(|r| r.tps.unwrap()).collect::<Vec<_>>(),
                    ))
                    .horizontal()
                    .format(|value| format!("{value:.0}")),
            )))
        .child(section_title("Cache hit, by model", theme.colors.fg_muted)
            .child(div().w(px(980.)).child(
                BarChart::new("models-cache", names(&cached))
                    .series(Series::new(
                        "cache hit %",
                        cached
                            .iter()
                            .map(|r| r.totals.cache_hit_rate().unwrap() * 100.0)
                            .collect::<Vec<_>>(),
                    ))
                    .horizontal()
                    .format(|value| format!("{value:.0}%")),
            )))
        .child(section_title("API list price, by model", theme.colors.fg_muted)
            .child(div().w(px(980.)).child(
                BarChart::new("models-cost", names(&priced))
                    .series(Series::new(
                        "¥ list",
                        priced
                            .iter()
                            .map(|r| r.totals.cost_cny.unwrap())
                            .collect::<Vec<_>>(),
                    ))
                    .horizontal()
                    .format(|value| format!("¥{value:.0}")),
            )))
        .child(div().w(px(1080.)).child(
            DataTable::new(
                "models-table",
                [
                    Column::new("model", "Model").width(rems(18.)),
                    Column::new("sources", "Sources").width(rems(14.)),
                    Column::new("requests", "Requests").width(rems(8.)).end(),
                    Column::new("tokens", "Tokens").width(rems(10.)).end(),
                    Column::new("cost", "¥ list").width(rems(9.)).end(),
                    Column::new("credits", "Credits").width(rems(9.)).end(),
                    Column::new("tps", "Tok/s").width(rems(7.)).end(),
                    Column::new("hit", "Cache %").width(rems(8.)).end(),
                ],
            )
            .rows(table_rows),
        ))
}

fn section_title(title: &str, muted: gpui::Hsla) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(rems(0.9))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(muted)
                .child(title.to_string()),
        )
}

fn human(value: f64) -> String {
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
