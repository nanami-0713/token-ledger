mod models;
mod overview;
mod sessions;
mod sources_page;

use ely_gpui_component::Assets;
use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::navigation::NavItem;
use ely_gpui_component::theme::{ActiveTheme, Mode, Theme};
use gpui::{
    App, AppContext as _, Context, ElementId, IntoElement, ParentElement, Render, SharedString,
    Styled, TitlebarOptions, Window, WindowOptions, actions, div, point, px, size,
    InteractiveElement, StatefulInteractiveElement,
};

use crate::core::aggregate::{Ledger, Span};
use crate::core::billing::Billing;
use crate::core::record::UsageRecord;
use crate::core::sources::{self, Config, ScanExtras, SourceStatus};

actions!(ledger, [Quit]);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Models,
    Sessions,
    Sources,
}

/// The whole app state: one scan's worth of records, kept until rescanned.
pub struct LedgerApp {
    pub config: Config,
    pub billing: Billing,
    /// One scan's answers over all history; pages never re-walk records.
    pub ledger: Ledger,
    /// The same answers inside the models page's lookback window.
    pub span_ledger: Ledger,
    pub span: Span,
    /// The scan's raw records, kept so a window change is a cheap filter.
    pub records: Vec<UsageRecord>,
    pub extras: ScanExtras,
    pub statuses: Vec<SourceStatus>,
    pub page: Page,
    pub dark: bool,
    pub scanned_at: String,
    /// What the last Add-a-folder probe decided, shown on the Sources page.
    pub folder_note: Option<String>,
}

impl LedgerApp {
    fn with(page: usize, dark: bool, cx: &mut Context<Self>) -> Self {
        let config = Config::load();
        let billing = config.billing();
        let (records, statuses, extras) = sources::scan_all(&config);
        let ledger = Ledger::build(&records, &billing, extras.tool_ms);
        let mut app = Self {
            config,
            billing,
            span_ledger: ledger.clone(),
            ledger,
            span: Span::All,
            records,
            extras,
            statuses,
            page: match page {
                1 => Page::Models,
                2 => Page::Sessions,
                3 => Page::Sources,
                _ => Page::Overview,
            },
            dark: dark || cx.theme().mode() == Mode::Dark,
            scanned_at: now_text(),
            folder_note: None,
        };
        app.apply_mode(cx);
        app
    }

    pub fn rescan(&mut self, cx: &mut Context<Self>) {
        self.billing = self.config.billing();
        let (records, statuses, extras) = sources::scan_all(&self.config);
        self.records = records;
        self.extras = extras;
        self.ledger = Ledger::build(&self.records, &self.billing, extras.tool_ms);
        self.refresh_span();
        self.statuses = statuses;
        self.scanned_at = now_text();
        cx.notify();
    }

    /// Rebuild the windowed ledger after a span change or a rescan.
    pub fn refresh_span(&mut self) {
        let now = jiff::Zoned::now().timestamp().as_millisecond();
        let cutoff = self.span.cutoff_ms(now);
        let scoped: Vec<UsageRecord> = match cutoff {
            Some(cutoff) => self.records.iter().filter(|r| r.ts_ms >= cutoff).cloned().collect(),
            None => self.records.clone(),
        };
        self.span_ledger = Ledger::build(&scoped, &self.billing, self.extras.tool_ms);
    }

    pub fn set_span(&mut self, span: Span, cx: &mut Context<Self>) {
        self.span = span;
        self.refresh_span();
        cx.notify();
    }

    /// A folder the picker returned: probe it, and add it when it matches a
    /// known log shape.
    pub fn add_folder(&mut self, dir: std::path::PathBuf, cx: &mut Context<Self>) {
        match sources::probe_folder(&dir) {
            Some(def) => {
                self.folder_note = Some(format!("{}: recognized and added", def.label));
                self.config.source.push(def);
                self.rescan(cx);
            }
            None => {
                self.folder_note = Some(format!(
                    "{}: no known log shape found; add it by hand in {}",
                    dir.display(),
                    sources::config_path().display()
                ));
                cx.notify();
            }
        }
    }

    pub fn toggle_source(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        if let Some(def) = self.config.source.iter_mut().find(|def| def.id == id) {
            def.enabled = on;
        }
        self.rescan(cx);
    }

    fn apply_mode(&self, cx: &mut App) {
        Theme::set_mode(
            if self.dark { Mode::Dark } else { Mode::Light },
            cx,
        );
    }

    pub fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.dark = !self.dark;
        self.apply_mode(cx);
        cx.notify();
    }
}

fn now_text() -> String {
    jiff::Zoned::now().strftime("%H:%M:%S").to_string()
}

impl Render for LedgerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let colors = cx.theme().colors.clone();
        let page = self.page;
        let nav = |id: &'static str,
                   icon: ely_gpui_component::primitives::IconName,
                   label: &'static str,
                   target: Page,
                   current: Page,
                   entity: &gpui::Entity<LedgerApp>| {
            NavItem::new(ElementId::Name(id.into()), icon, SharedString::from(label))
                .active(current == target)
                .on_click({
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |app, cx| {
                            app.page = target;
                            cx.notify();
                        })
                    }
                })
        };
        let sidebar = div()
            .id("sidebar")
            .flex()
            .flex_col()
            .gap_2()
            .w(px(224.))
            .h_full()
            .pt(px(48.))
            .px_4()
            .pb_4()
            .border_r_1()
            .border_color(colors.border)
            .child(
                div()
                    .px_2()
                    .pb_4()
                    .child(
                        div()
                            .text_size(gpui::rems(1.25))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(colors.fg)
                            .child("TokenLedger"),
                    )
                    .child(
                        div()
                            .text_size(gpui::rems(0.8))
                            .text_color(colors.fg_muted)
                            .child("every model call, one ledger"),
                    ),
            )
            .child(nav(
                "overview",
                ely_gpui_component::primitives::IconName::Gauge,
                "Overview",
                Page::Overview,
                page,
                &entity,
            ))
            .child(nav(
                "models",
                ely_gpui_component::primitives::IconName::ChartPie,
                "Models",
                Page::Models,
                page,
                &entity,
            ))
            .child(nav(
                "sessions",
                ely_gpui_component::primitives::IconName::History,
                "Sessions",
                Page::Sessions,
                page,
                &entity,
            ))
            .child(nav(
                "sources",
                ely_gpui_component::primitives::IconName::Database,
                "Sources",
                Page::Sources,
                page,
                &entity,
            ))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("rescan", "Rescan")
                            .variant(ButtonVariant::Ghost)
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |app, cx| app.rescan(cx))
                                }
                            }),
                    )
                    .child(
                        Button::new(
                            "theme",
                            if self.dark { "Light" } else { "Dark" },
                        )
                        .variant(ButtonVariant::Ghost)
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                entity.update(cx, |app, cx| app.toggle_theme(cx))
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .px_2()
                    .text_size(gpui::rems(0.75))
                    .text_color(colors.fg_muted)
                    .child(format!("scanned at {}", self.scanned_at)),
            );
        let content = match self.page {
            Page::Overview => overview::render(self, window, cx),
            Page::Models => models::render(self, window, cx),
            Page::Sessions => sessions::render(self, window, cx),
            Page::Sources => sources_page::render(self, window, cx),
        };
        div()
            .id("root")
            .flex()
            .size_full()
            .bg(colors.bg)
            .text_color(colors.fg)
            .child(sidebar)
            .child(
                div()
                    .id("page")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .p_8()
                    .child(content),
            )
    }
}

pub fn run(page: usize, dark: bool) -> anyhow::Result<()> {
    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            ely_gpui_component::init(cx);
            cx.bind_keys([gpui::KeyBinding::new("cmd-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            let bounds = gpui::Bounds::centered(None, size(px(1320.0), px(860.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("TokenLedger".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.0), px(18.0))),
                }),
                window_min_size: Some(size(px(960.0), px(600.0))),
                inactive_frame_interval: None,
                ..Default::default()
            };
            cx.open_window(options, |_, cx| cx.new(|cx| LedgerApp::with(page, dark, cx)))
                .expect("the ledger window failed to open");
            cx.activate(true);
        });
    Ok(())
}

