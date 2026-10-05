use ely_gpui_component::tables::{Cell, Column, DataTable, Row};
use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, px, rems};

use super::LedgerApp;
use crate::core::aggregate;

/// The heaviest sessions first: what ran, where, and what it cost.
pub fn render(app: &mut LedgerApp, _window: &mut Window, cx: &mut Context<LedgerApp>) -> gpui::Div {
    let sessions = aggregate::sessions(&app.records, &app.billing);
    let rows: Vec<Row> = sessions
        .iter()
        .take(60)
        .map(|session| {
            let name = session
                .title
                .clone()
                .unwrap_or_else(|| session.session.clone());
            let tokens = session.totals.input_net + session.totals.cache_read + session.totals.output;
            let when = jiff::Timestamp::from_millisecond(session.first_ms)
                .map(|ts| {
                    ts.to_zoned(jiff::tz::TimeZone::system())
                        .strftime("%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            Row::new(
                format!("{}:{}", session.source, session.session),
                [
                    Cell::Text(name.into()),
                    Cell::Text(session.source.clone().into()),
                    Cell::Text(session.models.join(", ").into()),
                    Cell::Number(session.totals.requests as f64),
                    Cell::Number(tokens as f64),
                    Cell::Number(session.totals.credits),
                    match session.totals.cost_cny {
                        Some(cny) => Cell::Number(cny),
                        None => Cell::from(""),
                    },
                    Cell::Text(when.into()),
                ],
            )
        })
        .collect();
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .gap_4()
        .max_w(px(1080.))
        .child(
            div()
                .text_size(gpui::rems(0.9))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.colors.fg_muted)
                .child(format!(
                    "{} sessions, heaviest first (top 60 shown)",
                    sessions.len()
                )),
        )
        .child(div().child(
            DataTable::new(
                "sessions-table",
                [
                    Column::new("name", "Session").width(rems(28.)),
                    Column::new("source", "Source").width(rems(8.)),
                    Column::new("models", "Models").width(rems(16.)),
                    Column::new("requests", "Req").width(rems(6.)).end(),
                    Column::new("tokens", "Tokens").width(rems(10.)).end(),
                    Column::new("credits", "Credits").width(rems(9.)).end(),
                    Column::new("cost", "\u{a5} list").width(rems(9.)).end(),
                    Column::new("first", "First seen").width(rems(10.)),
                ],
            )
            .rows(rows),
        ))
}
