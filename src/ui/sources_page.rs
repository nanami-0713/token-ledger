use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::forms::Switch;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AppContext as _, AsyncApp, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    PathPromptOptions, SharedString, StatefulInteractiveElement, Styled, Window, div, px, rems,
};

use super::LedgerApp;

/// What the ledger read and what it did not find: each source with its
/// standing, a folder picker that wires in any tool whose logs match a
/// known shape, and the config door for everything else.
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
        .children(app.config.load_note.clone().map(|note| {
            div()
                .mb_2()
                .p_3()
                .rounded(theme.radius(ely_gpui_component::theme::Radius::Md))
                .border_1()
                .border_color(theme.colors.warning)
                .bg(theme.colors.warning_subtle)
                .text_size(rems(0.85))
                .text_color(theme.colors.fg)
                .child(note)
        }))
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
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    Button::new("add-folder", "Add a folder of logs…")
                        .icon(IconName::FolderPlus)
                        .variant(ButtonVariant::Primary)
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                let picked = cx.prompt_for_paths(PathPromptOptions {
                                    files: false,
                                    directories: true,
                                    multiple: false,
                                    prompt: None,
                                });
                                let entity = entity.clone();
                                cx.spawn(async move |cx: &mut AsyncApp| {
                                    if let Ok(Ok(Some(paths))) = picked.await {
                                        if let Some(dir) = paths.first() {
                                            cx.update(|cx| {
                                                entity.update(cx, |app, cx| {
                                                    app.add_folder(dir.clone(), cx);
                                                });
                                            });
                                        }
                                    }
                                })
                                .detach();
                            }
                        }),
                )
                .children(app.folder_note.clone().map(|note| {
                    div()
                        .text_size(rems(0.85))
                        .text_color(theme.colors.fg_muted)
                        .child(note)
                })),
        )
        .child(
            div()
                .mt_4()
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
                        .child("Any other tool"),
                )
                .child(
                    div()
                        .text_size(rems(0.85))
                        .text_color(theme.colors.fg_muted)
                        .child(format!(
                            "The picker recognizes Claude Code, DSH, Codex and OpenAI-response \
                             shapes on its own. Anything else is a few lines in {}: point a glob \
                             at any JSONL log and name the fields that carry the model, the time, \
                             the input and the output.",
                            config_path.display()
                        )),
                ),
        )
}
