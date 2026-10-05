use std::collections::BTreeMap;

use ely_gpui_component::charts::{AreaChart, Series};
use ely_gpui_component::tables::{Cell, Column, DataTable, Row};
use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, px, rems};

use super::LedgerApp;
use crate::core::aggregate::{Daily, Totals};

/// The merge, on show: every model one row, the sources that called it in one
/// cell, its burn by day stacked by source.
pub fn render(app: &mut LedgerApp, _window: &mut Window, cx: &mut Context<LedgerApp>) -> gpui::Div {
    let billing = &app.billing;
    let records = &app.records;
    let mut table: BTreeMap<String, BTreeMap<String, Totals>> = BTreeMap::new();
    for rec in records {
        table
            .entry(rec.model_key.clone())
            .or_default()
            .entry(rec.source.clone())
            .or_default()
            .add(rec, billing);
    }
    let mut rows: Vec<(String, BTreeMap<String, Totals>)> = table.into_iter().collect();
    rows.sort_by(|a, b| {
        let burn = |by: &BTreeMap<String, Totals>| by.values().map(|t| t.credits).sum::<f64>();
        burn(&b.1).total_cmp(&burn(&a.1))
    });
    let daily = Daily::build(records, |rec| rec.model_key.clone(), "credits", billing);
    let top: Vec<&(String, BTreeMap<String, Totals>)> = rows.iter().take(6).collect();
    let theme = cx.theme();
    let mut chart = AreaChart::new("models-days", daily.days.clone());
    for (model, _) in &top {
        chart = chart.series(Series::new(
            model.clone(),
            daily.series.get(model).cloned().unwrap_or_default(),
        ));
    }
    chart = chart.stacked().format(|value| format!("{value:.0}"));
    let table_rows: Vec<Row> = rows
        .iter()
        .map(|(model, by_source)| {
            let mut total = Totals::default();
            for totals in by_source.values() {
                total.requests += totals.requests;
                total.input_net += totals.input_net;
                total.cache_read += totals.cache_read;
                total.output += totals.output;
                total.credits += totals.credits;
            }
            let tokens = total.input_net + total.cache_read + total.output;
            Row::new(
                model.clone(),
                [
                    Cell::Text(model.clone().into()),
                    Cell::Text(by_source.keys().cloned().collect::<Vec<_>>().join(" + ").into()),
                    Cell::Number(total.requests as f64),
                    Cell::Number(tokens as f64),
                    Cell::Number(total.credits),
                    Cell::Number(
                        total.cache_hit_rate().unwrap_or(0.0) * 100.0,
                    ),
                ],
            )
        })
        .collect();
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
                .child("Top models by credits per day"),
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
                    Column::new("credits", "Credits").width(rems(9.)).end(),
                    Column::new("hit", "Cache %").width(rems(8.)).end(),
                ],
            )
            .rows(table_rows),
        ))
}
