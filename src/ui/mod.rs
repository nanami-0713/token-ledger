mod models;
mod overview;
mod sessions;
mod sources_page;

use ely_gpui_component::Assets;
use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::navigation::NavItem;
use ely_gpui_component::theme::{ActiveTheme, Mode, Theme};
use gpui::{
    App, AppContext as _, Context, CursorStyle, DragMoveEvent, ElementId, EmptyView, IntoElement,
    ParentElement, Pixels, Render, SharedString, Styled, TitlebarOptions, Window, WindowOptions,
    actions, div, point, px, size, InteractiveElement, StatefulInteractiveElement,
};

use crate::core::aggregate::{Ledger, Span};
use crate::core::billing::Billing;
use crate::core::record::UsageRecord;
use crate::core::sources::{self, Config, ScanExtras, SourceStatus};

actions!(
    ledger,
    [
        Quit,
        Rescan,
        ToggleTheme,
        PageOverview,
        PageModels,
        PageSessions,
        PageSources,
    ]
);

/// How far the sidebar's edge may be dragged, in px. The floor keeps the
/// nav legible; the ceiling keeps the charts more than half the window.
const SIDEBAR_MIN: f32 = 200.0;
const SIDEBAR_MAX: f32 = 480.0;

/// The window refreshes itself on this cadence. A scan is a full re-read,
/// but it runs off-thread now, so the price is a few seconds of background
/// IO and numbers that are never stale for long.
const AUTO_REFRESH_SECS: u64 = 60;

/// Drag marker for the sidebar's resize strip; the width itself rides the
/// mouse in `on_drag_move`.
struct SidebarResize;

/// One short human number for every chart axis and tooltip: k/M/B, a
/// decimal only when the value has one.
pub(crate) fn human_f64(value: f64) -> String {
    let (unit, factor) = if value >= 1e9 {
        ("B", 1e9)
    } else if value >= 1e6 {
        ("M", 1e6)
    } else if value >= 1e3 {
        ("k", 1e3)
    } else {
        return format!("{value:.0}");
    };
    let scaled = value / factor;
    if (scaled - scaled.round()).abs() < 1e-9 {
        format!("{}{unit}", scaled.round() as i64)
    } else {
        format!("{scaled:.1}{unit}")
    }
}

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
    /// The sidebar's width, dragged along its right edge.
    pub sidebar_w: Pixels,
    /// Set while a scan runs off-thread; pages stand in for a placeholder.
    pub scanning: bool,
    /// A rescan was asked for mid-scan; it runs when the current one lands.
    pub pending_rescan: bool,
    pub scanned_at: String,
    /// What the last Add-a-folder probe decided, shown on the Sources page.
    pub folder_note: Option<String>,
}

impl LedgerApp {
    fn with(page: usize, dark: bool, cx: &mut Context<Self>) -> Self {
        let config = Config::load();
        let billing = config.billing();
        let app = Self {
            config,
            billing,
            // The first scan runs off-thread; the window shows a placeholder
            // until `apply_scan` lands the real ledgers.
            ledger: Ledger::default(),
            span_ledger: Ledger::default(),
            span: Span::All,
            records: Vec::new(),
            extras: ScanExtras::default(),
            statuses: Vec::new(),
            page: match page {
                1 => Page::Models,
                2 => Page::Sessions,
                3 => Page::Sources,
                _ => Page::Overview,
            },
            dark: dark || cx.theme().mode() == Mode::Dark,
            sidebar_w: px(224.),
            scanning: true,
            pending_rescan: false,
            // Empty until the first scan lands: the placeholder, not a
            // timestamp, says what is going on.
            scanned_at: String::new(),
            folder_note: None,
        };
        app.apply_mode(cx);
        app.scan_in_background(cx);
        app
    }

    /// One full scan on the background executor; `apply_scan` lands the
    /// outcome back on the entity. Inputs are captured up front, so a
    /// config edit mid-scan cannot bleed into a running scan.
    fn scan_in_background(&self, cx: &Context<Self>) {
        let config = self.config.clone();
        let billing = self.config.billing();
        cx.spawn(
            async move |this: gpui::WeakEntity<LedgerApp>, cx: &mut gpui::AsyncApp| {
                let outcome = cx
                    .background_spawn(async move {
                        let (records, statuses, extras) = sources::scan_all(&config);
                        let ledger = Ledger::build(&records, &billing, extras.tool_ms);
                        (records, statuses, extras, ledger)
                    })
                    .await;
                this.update(cx, |app, cx| app.apply_scan(outcome, cx)).ok();
            },
        )
        .detach();
    }

    /// One timer in flight at any moment: every scan's landing schedules the
    /// next, so refreshes never pile up and the chain never breaks.
    fn schedule_auto_refresh(&self, cx: &Context<Self>) {
        cx.spawn(
            async move |this: gpui::WeakEntity<LedgerApp>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(AUTO_REFRESH_SECS))
                    .await;
                this.update(cx, |app, cx| {
                    // If a scan is already in flight, it lands through
                    // `apply_scan`, which schedules the next timer itself.
                    if !app.scanning {
                        app.rescan(cx);
                    }
                })
                .ok();
            },
        )
        .detach();
    }

    pub fn rescan(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            // A scan is in flight; remember the press and run one more
            // round when it lands, so toggles made mid-scan are not lost.
            self.pending_rescan = true;
            cx.notify();
            return;
        }
        self.scanning = true;
        cx.notify();
        self.scan_in_background(cx);
    }

    /// Land one scan's outcome: the assignments of the old synchronous
    /// rescan, in their old order, plus the end-of-scan bookkeeping.
    fn apply_scan(
        &mut self,
        (records, statuses, extras, ledger): (
            Vec<UsageRecord>,
            Vec<SourceStatus>,
            ScanExtras,
            Ledger,
        ),
        cx: &mut Context<Self>,
    ) {
        self.billing = self.config.billing();
        self.records = records;
        self.extras = extras;
        self.ledger = ledger;
        self.refresh_span();
        self.statuses = statuses;
        self.scanned_at = now_text();
        self.scanning = false;
        if self.pending_rescan {
            self.pending_rescan = false;
            // The follow-up scan's own landing reschedules the timer.
            self.rescan(cx);
        } else {
            self.schedule_auto_refresh(cx);
        }
        cx.notify();
    }

    /// Rebuild the windowed ledger after a span change or a rescan.
    pub fn refresh_span(&mut self) {
        let now = jiff::Zoned::now().timestamp().as_millisecond();
        match self.span.cutoff_ms(now) {
            Some(cutoff) => {
                let scoped: Vec<UsageRecord> = self
                    .records
                    .iter()
                    .filter(|r| r.ts_ms >= cutoff)
                    .cloned()
                    .collect();
                self.span_ledger = Ledger::build(&scoped, &self.billing, self.extras.tool_ms);
            }
            // The all-history view is the main ledger itself — the same
            // records, billing and tool time already built into it — so
            // clone that instead of re-walking every record a second time.
            None => self.span_ledger = self.ledger.clone(),
        }
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
        let sidebar_body = div()
            .id("sidebar")
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
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
                    .child(if self.scanning {
                        "scanning…".to_string()
                    } else {
                        format!("scanned at {}", self.scanned_at)
                    }),
            );
        // The strip straddles the sidebar's right border; dragging it moves
        // the border, clamped, and the cursor talks east-west over it.
        let sidebar = div()
            .relative()
            .flex_none()
            .w(self.sidebar_w)
            .h_full()
            .child(sidebar_body)
            .child(
                div()
                    .id("sidebar-resize")
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .w(px(8.))
                    .flex_none()
                    .cursor(CursorStyle::ResizeLeftRight)
                    .hover(|strip| strip.bg(colors.border.opacity(0.6)))
                    .on_drag(SidebarResize, |_, _, _, cx| cx.new(|_| EmptyView))
                    .on_drag_move({
                        let entity = entity.clone();
                        move |event: &DragMoveEvent<SidebarResize>, _, cx| {
                            entity.update(cx, |app, cx| {
                                let x = f32::from(event.event.position.x);
                                app.sidebar_w = px(x.clamp(SIDEBAR_MIN, SIDEBAR_MAX));
                                cx.notify();
                            })
                        }
                    }),
            );
        let content = if self.scanning {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .size_full()
                .gap_2()
                .child(
                    div()
                        .text_size(gpui::rems(1.25))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(colors.fg)
                        .child("Reading logs…"),
                )
                .child(
                    div()
                        .text_size(gpui::rems(0.8))
                        .text_color(colors.fg_muted)
                        .child("every model call, one ledger"),
                )
        } else {
            match self.page {
                Page::Overview => overview::render(self, window, cx),
                Page::Models => models::render(self, window, cx),
                Page::Sessions => sessions::render(self, window, cx),
                Page::Sources => sources_page::render(self, window, cx),
            }
        };
        div()
            .id("root")
            .flex()
            .size_full()
            .bg(colors.bg)
            .text_color(colors.fg)
            // Page-switching and scan/theme keys, handled at the root so
            // they reach the app no matter where focus sits.
            .on_action(cx.listener(|app, _: &PageOverview, _, cx| {
                app.page = Page::Overview;
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &PageModels, _, cx| {
                app.page = Page::Models;
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &PageSessions, _, cx| {
                app.page = Page::Sessions;
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &PageSources, _, cx| {
                app.page = Page::Sources;
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &Rescan, _, cx| {
                app.rescan(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &ToggleTheme, _, cx| {
                app.toggle_theme(cx);
                cx.notify();
            }))
            .child(sidebar)
            .child(
                div()
                    .id("page")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_scroll()
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
            cx.bind_keys([
                gpui::KeyBinding::new("cmd-q", Quit, None),
                gpui::KeyBinding::new("cmd-1", PageOverview, None),
                gpui::KeyBinding::new("cmd-2", PageModels, None),
                gpui::KeyBinding::new("cmd-3", PageSessions, None),
                gpui::KeyBinding::new("cmd-4", PageSources, None),
                gpui::KeyBinding::new("cmd-r", Rescan, None),
                gpui::KeyBinding::new("cmd-d", ToggleTheme, None),
            ]);
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

