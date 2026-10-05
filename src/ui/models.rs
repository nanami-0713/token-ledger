use ely_gpui_component::charts::{AreaChart, Series};
use ely_gpui_component::tables::{Cell, Column, DataTable, Row};
use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, px, rems};

use super::LedgerApp;

/// The merge, on show: every model one row, the sources that called it in one
/// cell, its burn by day stacked by model — all from the scan-time cache.
pub fn render(app: &mut LedgerApp, _window: &mut Window, cx: &mut Context<LedgerApp>) -> gpui::Div {
    let ledger = &app.ledger;
    let mut chart = AreaChart::new("models-days", ledger.days.clone());
    for row in ledger.models_rows.iter().take(6) {
        chart = chart.series(Series::new(
            row.model.clone(),
            ledger.by_model.get(&row.model).cloned().unwrap_or_default(),
        ));
    }
    chart = chart.stacked().format(|value| {
        if value >= 1e9 {
            format!("{value:.2}B")
        } else if value >= 1e6 {
            format!("{value:.1}M")
        } else if value >= 1e3 {
            format!("{value:.0}k")
        } else {
            format!("{value:.0}")
        }
    });
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
                    Cell::Number(row.totals.cache_hit_rate().unwrap_or(0.0) * 100.0),
                ],
            )
        })
        .collect();
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .gap_6()
        .max_w(px(1080.))
        .child(
            div()
                .text_size(gpui::rems(0.9))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.colors.fg_muted)
                .child("Top models by tokens per day"),
        )
        .child(div().w(px(980.)).child(chart))
        .child(div().w(px(980.)).child(
            DataTable::new(
                "models-table",
                [
                    Column::new("model", "Model").width(rems(18.)),
                    Column::new("sources", "Sources").width(rems(14.)),
                    Column::new("requests", "Requests").width(rems(8.)).end(),
                    Column::new("tokens", "Tokens").width(rems(10.)).end(),
                    Column::new("cost", "¥ list").width(rems(9.)).end(),
                    Column::new("credits", "Credits").width(rems(9.)).end(),
                    Column::new("hit", "Cache %").width(rems(8.)).end(),
                ],
            )
            .rows(table_rows),
        ))
}
