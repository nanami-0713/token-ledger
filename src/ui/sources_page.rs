use ely_gpui_component::forms::Switch;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, px, rems};

use super::LedgerApp;

/// What the ledger read and what it did not find: each source with its
/// standing, plus the door for sources the config adds.
pub fn render(app: &mut LedgerApp, _window: &mut Window, cx: &mut Context<LedgerApp>) -> gpui::Div {
    let entity = cx.entity();
    let theme = cx.theme();
    let config_path = crate::core::sources::config_path();
    let rows: Vec<_> = app
        .statuses
        .iter()
        .map(|status| {
            let id = status.id.clone();
            div()
                .id(ElementId::Name(SharedString::from(id.clone())))
                .flex()
                .items_center()
                .gap_4()
                .py_3()
                .border_b_1()
                .border_color(theme.colors.border)
                .child(
                    div()
                        .w(px(180.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(status.label.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(rems(0.85))
                        .text_color(theme.colors.fg_muted)
                        .child(format!(
                            "{} · {} records · {}",
                            if status.present {
                                "reading"
                            } else if status.enabled {
                                "no data found"
                            } else {
                                "off"
                            },
                            status.records,
                            status.detail
                        )),
                )
                .child(Switch::new(
                    ElementId::Name(SharedString::from(format!("{id}-switch"))),
                    status.enabled,
                )
                .on_change({
                    let entity = entity.clone();
                    let id = id.clone();
                    move |on, _, cx| {
                        entity.update(cx, |app, cx| app.toggle_source(&id, on, cx))
                    }
                }))
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .max_w(px(880.))
        .child(
            div()
                .text_size(rems(0.9))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.colors.fg_muted)
                .child("Data sources"),
        )
        .children(rows)
        .child(
            div()
                .mt_6()
                .p_4()
                .rounded(theme.radius(ely_gpui_component::theme::Radius::Md))
                .border_1()
                .border_color(theme.colors.border)
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child("Add any other tool"),
                )
                .child(
                    div()
                        .text_size(rems(0.85))
                        .text_color(theme.colors.fg_muted)
                        .child(format!(
                            "A source is a few lines in {}. Point a glob at any JSONL log, name the \
                             fields that carry the model, the time, the input and the output, and \
                             its calls join the same ledgers as the built-ins.",
                            config_path.display()
                        )),
                ),
        )
}
