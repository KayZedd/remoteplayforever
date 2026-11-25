use crate::config::{Config, GuestControllerMode};
use crate::i18n::{Language, tr};
use crate::runner::{ActiveLaunch, LaunchPlan, start_launch};
use crate::session::{HostCommand, HostEvent, InputKind};
use crate::steam::{SteamApp, SteamInstallation};
use crate::update::UpdateInfo;
use eframe::egui;
use egui::{Color32, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

const ACCENT: Color32 = Color32::from_rgb(46, 160, 86);
const OK: Color32 = Color32::from_rgb(110, 220, 130);
const WARN: Color32 = Color32::from_rgb(255, 184, 82);
const ERROR: Color32 = Color32::from_rgb(255, 120, 120);
const INFO: Color32 = Color32::from_rgb(130, 195, 255);
const CARD: Color32 = Color32::from_rgb(30, 33, 38);
const CARD_ACTIVE: Color32 = Color32::from_rgb(26, 40, 32);

#[derive(PartialEq)]
enum Tab {
    Play,
    SteamGames,
    Guide,
    Settings,
}

/// What the session card shows: the host's events folded into plain state.
#[derive(Default)]
struct SessionView {
    status: String,
    error: Option<String>,
    link: Option<String>,
    ready: bool,
    guest_connected: bool,
    guests: Vec<String>,
    finished: bool,
}

impl SessionView {
    /// Folds one host event into the view. Returns a guest link that should go on the clipboard.
    fn apply(&mut self, event: HostEvent, copy_link: bool) -> Option<String> {
        match event {
            HostEvent::Status(msg) => self.status = msg,
            HostEvent::Link(url) => {
                self.status = if copy_link {
                    tr(
                        "Guest link copied to the clipboard. Send it to a friend.",
                        "Link dla gościa skopiowany do schowka. Wyślij go znajomemu.",
                    )
                } else {
                    tr(
                        "Guest link ready. Copy it and send it to a friend.",
                        "Link dla gościa gotowy. Skopiuj go i wyślij znajomemu.",
                    )
                }
                .into();
                self.link = Some(url.clone());
                return copy_link.then_some(url);
            }
            HostEvent::Ready => self.ready = true,
            HostEvent::GuestConnected => self.guest_connected = true,
            HostEvent::Player { name, .. } => {
                if !self.guests.contains(&name) {
                    self.guests.push(name);
                }
            }
            HostEvent::Error(msg) => self.error = Some(msg),
            HostEvent::Stopped => self.status = tr("Session ended.", "Sesja zakończona.").into(),
        }
        None
    }
}

/// A launched game and the state of its Remote Play Together session, as shown in the session card.
struct ActiveSession {
    launch: ActiveLaunch,
    view: SessionView,
    keyboard: bool,
    mouse: bool,
    controller: bool,
}

/// First dropped file that is a Windows executable.
fn dropped_executable(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths
        .into_iter()
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")))
}

/// Window icon (64x64 RGBA, drawn into assets/icon-64.rgba).
fn app_icon() -> egui::IconData {
    egui::IconData {
        rgba: include_bytes!("../assets/icon-64.rgba").to_vec(),
        width: 64,
        height: 64,
    }
}

/// How often the Steam status in the header is re-read.
const STEAM_STATUS_REFRESH: Duration = Duration::from_secs(2);

pub struct RemotePlayApp {
    config: Config,
    steam_inst: Option<SteamInstallation>,
    current_tab: Tab,
    selected_game_path: String,
    game_args: String,
    status_message: Option<(String, bool)>, // (message, is_error)
    search_query: String,
    installed_apps: Vec<SteamApp>,
    session: Option<ActiveSession>,
    autostart: Option<(PathBuf, Vec<String>)>,
    steam_running: bool,
    steam_checked: Instant,
    update_rx: Option<Receiver<UpdateInfo>>,
    update: Option<UpdateInfo>,
}

/// The guide, shared with the console interface: (title, text).
pub fn guide_steps() -> [(&'static str, &'static str); 7] {
    [
        (
            tr("1. Launch a game", "1. Uruchom grę"),
            tr(
                "Start Steam and sign in. Pick a game and press \"Launch with Remote Play Together\".",
                "Włącz Steam i zaloguj się. Wybierz grę i kliknij \"Uruchom z Remote Play Together\".",
            ),
        ),
        (
            tr("2. Send the link", "2. Wyślij link"),
            tr(
                "A few seconds later the guest link is on your clipboard. Paste it to a friend (Steam, Discord). One link is for one person; press \"Another guest\" for the next one.",
                "Po kilku sekundach link dla gościa trafia do schowka. Wklej go znajomemu (Steam, Discord). Jeden link to jedna osoba, dla kolejnej kliknij \"Kolejny gość\".",
            ),
        ),
        (
            tr("3. Your friend joins", "3. Znajomy dołącza"),
            tr(
                "They open the link in Steam or in the free Steam Link app (PC, Mac, Android, iOS). They need neither the game nor a fast computer.",
                "Otwiera link w Steam albo w darmowej aplikacji Steam Link (PC, Mac, Android, iOS). Nie musi mieć gry ani mocnego komputera.",
            ),
        ),
        (
            tr("4. Controls", "4. Sterowanie"),
            tr(
                "The session card turns the guests' keyboard, mouse and controller on and off, live. Steam presents a guest's controller as the next player.",
                "W panelu sesji włączasz i wyłączasz gościom klawiaturę, mysz i kontroler, na żywo. Steam podaje pada gościa jako kolejnego gracza.",
            ),
        ),
        (
            tr(
                "5. Controller ignored by a non-Steam game?",
                "5. Pad nie działa w grze spoza Steam?",
            ),
            tr(
                "In the game: Shift+Tab -> Controller settings -> for every controller (yours and the guests') choose a layout from the \"Gamepad\" template. Once is enough.",
                "W grze: Shift+Tab -> Ustawienia kontrolera -> dla każdego pada (swojego i gości) wybierz układ z szablonu \"Gamepad\". Wystarczy raz.",
            ),
        ),
        (
            tr(
                "6. More than 60 FPS for the guest",
                "6. Więcej niż 60 klatek u gościa",
            ),
            tr(
                "The guest sets the limit on their side: Steam -> Settings -> Remote Play -> advanced client options -> Framerate limit (up to 240) and Quality \"Fast\". Their display has to support it.",
                "Limit ustawia gość u siebie: Steam -> Ustawienia -> Remote Play -> zaawansowane ustawienia klienta -> Limit klatek (do 240) i Jakość \"Szybko\". Potrzebny monitor o takim odświeżaniu.",
            ),
        ),
        (
            tr(
                "7. Steam games and non-Steam games",
                "7. Gry ze Steam i spoza Steam",
            ),
            tr(
                "A Steam game is started by Steam under its own AppID and guests see the game window. A non-Steam game runs with the Steam overlay and guests see your whole desktop, so close private windows first.",
                "Gra ze Steam startuje przez Steam pod własnym AppID, a goście widzą okno gry. Gra spoza Steam startuje z nakładką Steam, a goście widzą cały pulpit, więc najpierw zamknij prywatne okna.",
            ),
        ),
    ]
}

fn controller_mode_name(mode: GuestControllerMode) -> &'static str {
    match mode {
        GuestControllerMode::Auto => tr(
            "Automatic (detect per game)",
            "Automatycznie (wykrywa dla gry)",
        ),
        GuestControllerMode::Enabled => tr("On", "Włączony"),
        GuestControllerMode::Disabled => tr(
            "Off (keyboard and mouse only)",
            "Wyłączony (tylko klawiatura i mysz)",
        ),
    }
}

fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::new(21.0, Proportional));
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(14.5, Proportional));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::new(14.5, Proportional));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::new(12.0, Proportional));
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
    });
}

/// A padded, rounded panel spanning the available width.
fn card<R>(ui: &mut egui::Ui, fill: Color32, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
        .inner
}

fn note(ui: &mut egui::Ui, color: Color32, text: impl Into<String>) {
    ui.label(RichText::new(text).color(color).small());
}

/// Filled status dot (the bundled font has no solid circle glyph).
fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(16.0).strong());
}

impl RemotePlayApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        autostart: Option<(PathBuf, Vec<String>)>,
    ) -> Self {
        apply_theme(&cc.egui_ctx);

        let config = Config::load();
        let steam_inst = SteamInstallation::detect().ok();
        let mut installed_apps = steam_inst
            .as_ref()
            .map(|s| s.get_installed_apps())
            .unwrap_or_default();
        installed_apps.sort_by_key(|a| a.name.to_lowercase());
        let update_rx = config
            .check_for_updates
            .then(crate::update::check_in_background);

        let initial_path = autostart
            .as_ref()
            .map(|(path, _)| path.display().to_string())
            .or_else(|| {
                config
                    .recent_games
                    .iter()
                    .find(|g| g.path.is_file())
                    .map(|g| g.path.display().to_string())
            })
            .unwrap_or_default();

        Self {
            config,
            steam_inst,
            current_tab: Tab::Play,
            selected_game_path: initial_path,
            game_args: String::new(),
            status_message: None,
            search_query: String::new(),
            installed_apps,
            session: None,
            autostart,
            steam_running: crate::steam::active_user() != 0,
            steam_checked: Instant::now(),
            update_rx,
            update: None,
        }
    }

    /// Open the window; `autostart` is a game to launch right away (`--run`, shortcuts, drag & drop).
    pub fn run(autostart: Option<(PathBuf, Vec<String>)>) -> eframe::Result {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([820.0, 720.0])
                .with_min_inner_size([640.0, 520.0])
                .with_title("RemotePlayForever")
                .with_icon(app_icon()),
            ..Default::default()
        };

        eframe::run_native(
            "RemotePlayForever",
            options,
            Box::new(|cc| Ok(Box::new(Self::new(cc, autostart)))),
        )
    }

    fn session_active(&self) -> bool {
        self.session.as_ref().is_some_and(|s| !s.view.finished)
    }

    fn launch_selected(&mut self, path_str: &str, args_str: &str) {
        if self.session_active() {
            self.status_message = Some((
                tr(
                    "A session is already running. End it before launching another game.",
                    "Sesja już trwa. Zakończ ją, zanim uruchomisz kolejną grę.",
                )
                .into(),
                true,
            ));
            return;
        }

        let path = crate::runner::clean_target_path(Path::new(path_str));
        if !path.exists() {
            self.status_message = Some((
                format!(
                    "{}: {path_str}",
                    tr("File does not exist", "Plik nie istnieje")
                ),
                true,
            ));
            return;
        }

        let args: Vec<String> = args_str.split_whitespace().map(|s| s.to_string()).collect();
        self.config.add_recent(&path, &args);
        let inputs = self.config.guest_inputs(&path);

        match start_launch(&path, &args, inputs) {
            Ok(launch) => {
                self.status_message = None;
                self.session = Some(ActiveSession {
                    launch,
                    view: SessionView {
                        status: tr("Starting...", "Uruchamianie...").into(),
                        ..SessionView::default()
                    },
                    keyboard: inputs.keyboard,
                    mouse: inputs.mouse,
                    controller: inputs.controller,
                });
            }
            Err(e) => {
                self.status_message = Some((
                    format!("{}: {e:#}", tr("Launch failed", "Błąd uruchamiania")),
                    true,
                ));
            }
        }
    }

    fn poll_session(&mut self, ctx: &egui::Context) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.view.finished {
            return;
        }

        let (events, finished) = session.launch.poll();
        for event in events {
            if let Some(link) = session
                .view
                .apply(event, self.config.copy_link_automatically)
            {
                ctx.copy_text(link);
            }
        }
        session.view.finished = finished;
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("🎮 RemotePlayForever");
            ui.label(
                RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                    .weak()
                    .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (color, text) = if self.steam_inst.is_none() {
                    (ERROR, tr("Steam not found", "Nie wykryto Steam"))
                } else if self.steam_running {
                    (OK, tr("Steam is running", "Steam działa"))
                } else {
                    (
                        ERROR,
                        tr("Start Steam and sign in", "Uruchom Steam i zaloguj się"),
                    )
                };
                ui.label(RichText::new(text).color(color).strong());
                dot(ui, color);
            });
        });
        ui.label(
            RichText::new(tr(
                "Steam Remote Play Together for any game",
                "Steam Remote Play Together dla dowolnej gry",
            ))
            .weak(),
        );

        if let Some(update) = self.update.clone() {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "⬆ {} {}",
                        tr("New version available:", "Dostępna nowa wersja:"),
                        update.version
                    ))
                    .color(INFO)
                    .strong(),
                );
                if ui.button(tr("Download", "Pobierz")).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&update.url));
                }
                if ui
                    .small_button("✖")
                    .on_hover_text(tr(
                        "Hide until the next start",
                        "Ukryj do następnego uruchomienia",
                    ))
                    .clicked()
                {
                    self.update = None;
                }
            });
        }

        if self.steam_inst.is_some() && !self.steam_running {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                note(
                    ui,
                    WARN,
                    tr(
                        "Steam is not running. Remote Play Together needs it signed in.",
                        "Steam nie działa. Remote Play Together wymaga zalogowanego Steam.",
                    ),
                );
                if ui
                    .small_button(tr("Start Steam", "Uruchom Steam"))
                    .clicked()
                {
                    let _ = std::process::Command::new("explorer")
                        .arg("steam://open/main")
                        .spawn();
                }
            });
        }

        if let Some((msg, is_err)) = self.status_message.clone() {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(msg)
                        .color(if is_err { ERROR } else { OK })
                        .strong(),
                );
                if ui.small_button("✖").clicked() {
                    self.status_message = None;
                }
            });
        }
    }

    fn ui_session_card(&mut self, ui: &mut egui::Ui) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let mut close_card = false;
        let fill = if session.view.finished {
            CARD
        } else {
            CARD_ACTIVE
        };

        card(ui, fill, |ui| {
            ui.horizontal(|ui| {
                let (color, state) = if session.view.finished {
                    (Color32::GRAY, tr("ended", "zakończona"))
                } else if session.view.ready {
                    (OK, tr("live", "aktywna"))
                } else {
                    (WARN, tr("starting", "startuje"))
                };
                dot(ui, color);
                ui.label(
                    RichText::new(format!(
                        "{}: {}",
                        tr("Session", "Sesja"),
                        session.launch.game_name
                    ))
                    .size(16.0)
                    .strong(),
                );
                ui.label(RichText::new(state).color(color));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if session.view.finished {
                        if ui.button(tr("✖ Close", "✖ Zamknij")).clicked() {
                            close_card = true;
                        }
                    } else if ui
                        .button(tr("⏹ End session", "⏹ Zakończ sesję"))
                        .on_hover_text(tr(
                            "Disconnects the guests. The game keeps running.",
                            "Rozłącza gości. Gra działa dalej.",
                        ))
                        .clicked()
                    {
                        session.launch.send(HostCommand::Stop);
                        session.view.status =
                            tr("Ending the session...", "Kończenie sesji...").into();
                    }
                });
            });

            ui.label(RichText::new(&session.view.status).weak());
            if let Some(ref err) = session.view.error {
                ui.label(RichText::new(format!("❌ {err}")).color(ERROR).strong());
            }
            if session.view.finished {
                return;
            }

            if let Some(link) = session.view.link.clone() {
                ui.add_space(4.0);
                ui.label(RichText::new(tr("Guest link", "Link dla gościa")).strong());
                ui.horizontal(|ui| {
                    let mut text = link.clone();
                    ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width((ui.available_width() - 250.0).max(120.0)),
                    );
                    if ui
                        .button(tr("📋 Copy", "📋 Kopiuj"))
                        .on_hover_text(tr(
                            "Copy the guest link to the clipboard",
                            "Skopiuj link dla gościa do schowka",
                        ))
                        .clicked()
                    {
                        ui.ctx().copy_text(link);
                        session.view.status = tr(
                            "Link copied to the clipboard.",
                            "Link skopiowany do schowka.",
                        )
                        .into();
                    }
                    if ui
                        .button(tr("➕ Another guest", "➕ Kolejny gość"))
                        .on_hover_text(tr(
                            "One link is for one person. Create a new one for the next player.",
                            "Jeden link jest dla jednej osoby. Utwórz nowy dla kolejnego gracza.",
                        ))
                        .clicked()
                    {
                        session.launch.send(HostCommand::AddLink);
                        session.view.status = tr(
                            "Creating a link for another guest...",
                            "Tworzenie linku dla kolejnego gościa...",
                        )
                        .into();
                    }
                });
            }

            if session.view.ready {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Guests may use:", "Goście mogą używać:")).strong());
                    for (label, kind, value) in [
                        (
                            tr("⌨ Keyboard", "⌨ Klawiatura"),
                            InputKind::Keyboard,
                            &mut session.keyboard,
                        ),
                        (
                            tr("🖱 Mouse", "🖱 Mysz"),
                            InputKind::Mouse,
                            &mut session.mouse,
                        ),
                        (
                            tr("🎮 Controller", "🎮 Kontroler"),
                            InputKind::Controller,
                            &mut session.controller,
                        ),
                    ] {
                        let hint = match kind {
                            InputKind::Keyboard => tr(
                                "Let guests type in the game. Takes effect immediately.",
                                "Pozwól gościom pisać w grze. Działa od razu.",
                            ),
                            InputKind::Mouse => tr(
                                "Let guests move the mouse. Takes effect immediately.",
                                "Pozwól gościom używać myszy. Działa od razu.",
                            ),
                            InputKind::Controller => tr(
                                "Let guests use their controllers. Takes effect immediately.",
                                "Pozwól gościom używać padów. Działa od razu.",
                            ),
                        };
                        if ui.checkbox(value, label).on_hover_text(hint).changed() {
                            session.launch.send(HostCommand::SetInput(kind, *value));
                        }
                    }
                });

                if !session.view.guests.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "👥 {}: {}",
                            tr("Connected", "Połączeni"),
                            session.view.guests.join(", ")
                        ))
                        .color(OK),
                    );
                } else if session.view.guest_connected {
                    ui.label(
                        RichText::new(tr("👥 Guest connected", "👥 Gość połączony")).color(OK),
                    );
                } else {
                    ui.label(
                        RichText::new(tr("Waiting for a guest...", "Czekam na gościa...")).weak(),
                    );
                }
            }

            if session.launch.plan == LaunchPlan::External {
                ui.add_space(2.0);
                note(
                    ui,
                    WARN,
                    tr(
                        "⚠ Guests see your whole desktop, not just the game window. Close private windows.",
                        "⚠ Goście widzą cały Twój pulpit, nie tylko okno gry. Zamknij prywatne okna.",
                    ),
                );
                ui.collapsing(
                    RichText::new(tr(
                        "Controller ignored by the game?",
                        "Pad nie działa w grze?",
                    ))
                    .small(),
                    |ui| {
                        note(ui, Color32::LIGHT_GRAY, guide_steps()[4].1);
                    },
                );
            }
        });

        if close_card {
            self.session = None;
        }
    }
}

impl eframe::App for RemotePlayApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some((path, args)) = self.autostart.take() {
            self.game_args = args.join(" ");
            let (path, args) = (path.display().to_string(), self.game_args.clone());
            self.launch_selected(&path, &args);
        }
        self.poll_session(ui.ctx());
        self.poll_background(ui.ctx());
        self.handle_drops(ui.ctx());

        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                self.ui_header(ui);
                ui.add_space(6.0);
                self.ui_session_card(ui);
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (Tab::Play, tr("🚀 Launch", "🚀 Uruchom grę")),
                        (Tab::SteamGames, tr("📚 Steam games", "📚 Gry ze Steam")),
                        (Tab::Guide, tr("❓ Guide", "❓ Poradnik")),
                        (Tab::Settings, tr("⚙ Settings", "⚙ Ustawienia")),
                    ] {
                        ui.selectable_value(
                            &mut self.current_tab,
                            tab,
                            RichText::new(label).size(15.0),
                        );
                    }
                });
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.current_tab {
                        Tab::Play => self.ui_play_tab(ui),
                        Tab::SteamGames => self.ui_steam_games_tab(ui),
                        Tab::Guide => self.ui_guide_tab(ui),
                        Tab::Settings => self.ui_settings_tab(ui),
                    });
            });
    }
}

impl RemotePlayApp {
    /// Refreshes the Steam status every couple of seconds and picks up the update lookup result.
    fn poll_background(&mut self, ctx: &egui::Context) {
        if self.steam_checked.elapsed() >= STEAM_STATUS_REFRESH {
            self.steam_running = crate::steam::active_user() != 0;
            self.steam_checked = Instant::now();
        }
        ctx.request_repaint_after(STEAM_STATUS_REFRESH);

        if let Some(rx) = &self.update_rx
            && let Ok(info) = rx.try_recv()
        {
            self.update = Some(info);
            self.update_rx = None;
        }
    }

    /// Dropping a game's .exe on the window selects it; the overlay tells the user it will work.
    fn handle_drops(&mut self, ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_overlay"),
            ));
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(190));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                tr("Drop the game's .exe here", "Upuść tu plik .exe gry"),
                egui::FontId::proportional(24.0),
                Color32::WHITE,
            );
        }

        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if dropped.is_empty() {
            return;
        }
        match dropped_executable(dropped) {
            Some(path) => {
                self.selected_game_path = path.display().to_string();
                self.current_tab = Tab::Play;
                self.status_message = None;
            }
            None => {
                self.status_message = Some((
                    tr(
                        "Only .exe files can be launched. Drop the game's executable.",
                        "Można uruchomić tylko pliki .exe. Upuść plik wykonywalny gry.",
                    )
                    .into(),
                    true,
                ));
            }
        }
    }

    fn ui_play_tab(&mut self, ui: &mut egui::Ui) {
        let session_active = self.session_active();

        card(ui, CARD, |ui| {
            section_title(ui, tr("Game", "Gra"));
            let mut enter_pressed = false;
            ui.horizontal(|ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.selected_game_path)
                        .hint_text(tr(
                            "Path to the game's .exe (or drop the file here)",
                            "Ścieżka do pliku .exe gry (albo upuść plik tutaj)",
                        ))
                        .desired_width((ui.available_width() - 130.0).max(120.0)),
                );
                enter_pressed = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button(tr("📂 Browse...", "📂 Przeglądaj...")).clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter(
                            tr("Executables (*.exe)", "Pliki wykonywalne (*.exe)"),
                            &["exe"],
                        )
                        .set_title(tr(
                            "Pick the game executable (.exe)",
                            "Wybierz plik wykonywalny gry (.exe)",
                        ))
                        .pick_file()
                {
                    self.selected_game_path = path.display().to_string();
                }
            });

            let path = crate::runner::clean_target_path(Path::new(&self.selected_game_path));
            let has_selected = !self.selected_game_path.trim().is_empty() && path.is_file();

            if has_selected {
                match LaunchPlan::for_target(&path) {
                    LaunchPlan::SteamApp { appid } => note(
                        ui,
                        OK,
                        format!(
                            "✔ {} (AppID {appid}). {}",
                            tr("Steam game", "Gra ze Steam"),
                            tr(
                                "Steam starts it, guests see the game window.",
                                "Wystartuje przez Steam, goście zobaczą okno gry."
                            ),
                        ),
                    ),
                    LaunchPlan::External => note(
                        ui,
                        INFO,
                        tr(
                            "ℹ Non-Steam game. Runs with the Steam overlay (Shift+Tab, guest controllers); guests see your whole desktop.",
                            "ℹ Gra spoza Steam. Wystartuje z nakładką Steam (Shift+Tab, pady gości), goście zobaczą cały pulpit.",
                        ),
                    ),
                }
            } else if !self.selected_game_path.trim().is_empty() {
                note(ui, ERROR, tr("No such file.", "Nie ma takiego pliku."));
            } else {
                note(
                    ui,
                    Color32::GRAY,
                    tr(
                        "Pick a game's .exe with Browse, drop it on this window, or choose one on the Steam games tab.",
                        "Wybierz plik .exe gry przyciskiem Przeglądaj, upuść go na okno albo wybierz grę na karcie Gry ze Steam.",
                    ),
                );
            }

            ui.add_space(4.0);
            egui::Grid::new("launch_options")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(tr("Arguments", "Argumenty"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.game_args)
                            .hint_text(tr(
                                "optional, e.g. -fullscreen",
                                "opcjonalnie, np. -fullscreen",
                            ))
                            .desired_width(f32::INFINITY),
                    );
                    ui.end_row();

                    ui.label(tr("Guest controller", "Kontroler dla gości"));
                    let selected_label = match self.config.guest_controller_mode {
                        GuestControllerMode::Auto if !has_selected => {
                            tr("Automatic", "Automatycznie")
                        }
                        GuestControllerMode::Auto
                            if self.config.resolve_controller_allowed(&path) =>
                        {
                            tr(
                                "Automatic (this game: on)",
                                "Automatycznie (ta gra: włączony)",
                            )
                        }
                        GuestControllerMode::Auto => tr(
                            "Automatic (this game: off)",
                            "Automatycznie (ta gra: wyłączony)",
                        ),
                        mode => controller_mode_name(mode),
                    };
                    self.controller_mode_combo(ui, "play_controller_mode", selected_label);
                    ui.end_row();
                });

            ui.add_space(8.0);
            let launch_btn = egui::Button::new(
                RichText::new(tr(
                    "▶  LAUNCH WITH REMOTE PLAY TOGETHER",
                    "▶  URUCHOM Z REMOTE PLAY TOGETHER",
                ))
                .size(16.0)
                .strong()
                .color(Color32::WHITE),
            )
            .fill(ACCENT);
            let launch = ui
                .add_enabled_ui(has_selected && !session_active, |ui| {
                    ui.add_sized([ui.available_width(), 44.0], launch_btn)
                })
                .inner;
            if (launch.clicked() || (enter_pressed && has_selected && !session_active))
                && !self.selected_game_path.trim().is_empty()
            {
                let (path, args) = (self.selected_game_path.clone(), self.game_args.clone());
                self.launch_selected(&path, &args);
            }
            if self.steam_inst.is_some() && !self.steam_running && !session_active {
                note(
                    ui,
                    WARN,
                    tr(
                        "Steam is not running. Start it and sign in first.",
                        "Steam nie działa. Najpierw uruchom go i zaloguj się.",
                    ),
                );
            }
            if session_active {
                note(
                    ui,
                    Color32::GRAY,
                    tr(
                        "A session is running. End it to launch another game.",
                        "Sesja trwa. Zakończ ją, aby uruchomić kolejną grę.",
                    ),
                );
            }

            if ui
                .add_enabled(
                    has_selected,
                    egui::Button::new(tr(
                        "🖥 Create desktop shortcut",
                        "🖥 Utwórz skrót na pulpicie",
                    ))
                    .small(),
                )
                .on_hover_text(tr(
                    "Double-clicking the shortcut launches this game and opens a session.",
                    "Dwuklik w skrót uruchamia tę grę i od razu otwiera sesję.",
                ))
                .clicked()
            {
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Game");
                self.status_message = Some(
                    match crate::installer::create_desktop_shortcut(name, &path) {
                        Ok(p) => (
                            format!(
                                "{}: {}",
                                tr("Shortcut created", "Utworzono skrót"),
                                p.display()
                            ),
                            false,
                        ),
                        Err(e) => (format!("{e:#}"), true),
                    },
                );
            }
        });

        ui.add_space(10.0);
        section_title(ui, tr("Recent", "Ostatnio uruchamiane"));

        let recent_games: Vec<_> = self
            .config
            .recent_games
            .iter()
            .filter(|g| g.path.is_file())
            .cloned()
            .collect();
        if recent_games.is_empty() {
            ui.label(
                RichText::new(tr(
                    "Nothing yet. Games you launch show up here.",
                    "Brak historii. Uruchomione gry pojawią się tutaj.",
                ))
                .weak(),
            );
        }
        for game in recent_games {
            card(ui, CARD, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&game.name).strong());
                        ui.label(
                            RichText::new(game.path.display().to_string())
                                .weak()
                                .small(),
                        );
                        if !game.last_played.is_empty() {
                            note(
                                ui,
                                Color32::GRAY,
                                format!(
                                    "{} {} UTC",
                                    tr("Last played", "Ostatnio"),
                                    game.last_played
                                ),
                            );
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("✖")
                            .on_hover_text(tr("Remove from the list", "Usuń z listy"))
                            .clicked()
                        {
                            self.config.remove_recent(&game.path);
                        }
                        if ui
                            .add_enabled(!session_active, egui::Button::new(tr("▶ Play", "▶ Graj")))
                            .clicked()
                        {
                            self.selected_game_path = game.path.display().to_string();
                            self.game_args = game.args.join(" ");
                            self.launch_selected(
                                &self.selected_game_path.clone(),
                                &self.game_args.clone(),
                            );
                        }
                    });
                });
            });
        }
    }

    /// Combo box bound to the guest controller setting; saves on change.
    fn controller_mode_combo(&mut self, ui: &mut egui::Ui, id: &str, selected_label: &str) {
        let mut mode = self.config.guest_controller_mode;
        egui::ComboBox::from_id_salt(id)
            .selected_text(selected_label)
            .width(280.0)
            .show_ui(ui, |ui| {
                for option in [
                    GuestControllerMode::Auto,
                    GuestControllerMode::Enabled,
                    GuestControllerMode::Disabled,
                ] {
                    ui.selectable_value(&mut mode, option, controller_mode_name(option));
                }
            });
        if mode != self.config.guest_controller_mode {
            self.config.guest_controller_mode = mode;
            let _ = self.config.save();
        }
    }

    fn ui_steam_games_tab(&mut self, ui: &mut egui::Ui) {
        section_title(ui, tr("Installed Steam games", "Zainstalowane gry Steam"));
        ui.label(RichText::new(tr("Steam starts the game and the app opens a Remote Play Together session for it.", "Gra wystartuje przez Steam, a program otworzy dla niej sesję Remote Play Together.")).weak());

        ui.add_space(2.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.search_query)
                .hint_text(tr("🔍 Search...", "🔍 Szukaj gry..."))
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);

        let session_active = self.session_active();
        let query = self.search_query.to_lowercase();
        let apps: Vec<_> = self
            .installed_apps
            .iter()
            .filter(|a| a.executable.is_some() && a.appid != crate::runner::NON_STEAM_SESSION_APPID)
            .filter(|a| query.is_empty() || a.name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        if apps.is_empty() {
            let text = if self.steam_inst.is_none() {
                tr(
                    "Steam was not found on this computer.",
                    "Nie znaleziono Steam na tym komputerze.",
                )
            } else if self.installed_apps.is_empty() {
                tr(
                    "No installed Steam games found.",
                    "Nie znaleziono zainstalowanych gier Steam.",
                )
            } else {
                tr(
                    "No games match the search.",
                    "Brak gier pasujących do wyszukiwania.",
                )
            };
            ui.label(RichText::new(text).weak());
        } else {
            note(
                ui,
                Color32::GRAY,
                format!("{} {}", apps.len(), tr("games", "gier")),
            );
        }

        for app in apps {
            card(ui, CARD, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&app.name).strong());
                        ui.label(RichText::new(format!("AppID {}", app.appid)).weak().small());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let Some(ref exe) = app.executable else {
                            return;
                        };
                        if ui
                            .add_enabled(!session_active, egui::Button::new(tr("▶ Play", "▶ Graj")))
                            .clicked()
                        {
                            let path_str = exe.display().to_string();
                            self.selected_game_path = path_str.clone();
                            self.launch_selected(&path_str, "");
                        }
                    });
                });
            });
        }
    }

    fn ui_guide_tab(&self, ui: &mut egui::Ui) {
        for (title, body) in guide_steps() {
            card(ui, CARD, |ui| {
                ui.label(RichText::new(title).strong());
                ui.label(body);
            });
        }
    }

    fn ui_settings_tab(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;

        card(ui, CARD, |ui| {
            section_title(ui, tr("Interface", "Interfejs"));
            egui::Grid::new("settings_interface")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(tr("Language", "Język"));
                    egui::ComboBox::from_id_salt("language")
                        .selected_text(self.config.language.native_name())
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            for language in Language::ALL {
                                changed |= ui
                                    .selectable_value(
                                        &mut self.config.language,
                                        language,
                                        language.native_name(),
                                    )
                                    .changed();
                            }
                        });
                    ui.end_row();
                });
            changed |= ui
                .checkbox(
                    &mut self.config.copy_link_automatically,
                    tr(
                        "Copy each new guest link to the clipboard",
                        "Kopiuj każdy nowy link dla gościa do schowka",
                    ),
                )
                .changed();
            changed |= ui
                .checkbox(
                    &mut self.config.check_for_updates,
                    tr(
                        "Check GitHub for a new version on start",
                        "Przy starcie sprawdzaj na GitHubie, czy jest nowa wersja",
                    ),
                )
                .on_hover_text(tr(
                    "One anonymous request to github.com. Nothing about you or your games is sent.",
                    "Jedno anonimowe zapytanie do github.com. Nic o Tobie ani Twoich grach nie jest wysyłane.",
                ))
                .changed();
        });

        ui.add_space(4.0);
        card(ui, CARD, |ui| {
            section_title(ui, tr("Guests", "Goście"));
            ui.label(RichText::new(tr("What guests may use when they join. You can still change it live during a session.", "Z czego goście mogą korzystać po dołączeniu. W trakcie sesji nadal możesz to zmieniać na żywo.")).weak());
            changed |= ui
                .checkbox(
                    &mut self.config.guest_keyboard,
                    tr("⌨ Keyboard", "⌨ Klawiatura"),
                )
                .changed();
            changed |= ui
                .checkbox(&mut self.config.guest_mouse, tr("🖱 Mouse", "🖱 Mysz"))
                .changed();
            ui.horizontal(|ui| {
                ui.label(tr("🎮 Controller", "🎮 Kontroler"));
                self.controller_mode_combo(
                    ui,
                    "settings_controller_mode",
                    controller_mode_name(self.config.guest_controller_mode),
                );
            });
        });

        ui.add_space(4.0);
        card(ui, CARD, |ui| {
            section_title(ui, tr("Data", "Dane"));
            ui.horizontal(|ui| {
                if ui
                    .button(tr("🗑 Clear recent games", "🗑 Wyczyść ostatnio uruchamiane"))
                    .clicked()
                {
                    self.config.recent_games.clear();
                    changed = true;
                }
                if ui
                    .button(tr("📂 Open log folder", "📂 Otwórz folder z logiem"))
                    .clicked()
                    && let Some(dir) = crate::runner::get_log_path().parent()
                {
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
            });
            note(
                ui,
                Color32::GRAY,
                format!(
                    "{}: {}",
                    tr("Settings file", "Plik ustawień"),
                    Config::config_path().display()
                ),
            );
            note(
                ui,
                Color32::GRAY,
                format!(
                    "{}: {}",
                    tr("Session log", "Log sesji"),
                    crate::runner::get_log_path().display()
                ),
            );
        });

        if changed {
            crate::i18n::set_language(self.config.language);
            let _ = self.config.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn dropped_exe_is_found_case_insensitively() {
        assert_eq!(
            dropped_executable(paths(&["notes.txt", r"C:\Games\Game.EXE", "b.exe"])),
            Some(PathBuf::from(r"C:\Games\Game.EXE"))
        );
    }

    #[test]
    fn dropped_non_executables_are_rejected() {
        assert_eq!(
            dropped_executable(paths(&["a.txt", "b.lnk", "noext"])),
            None
        );
        assert_eq!(dropped_executable(paths(&[])), None);
    }

    #[test]
    fn link_event_is_copied_only_when_enabled() {
        let mut view = SessionView::default();
        let url = "https://s.team/rpt/1".to_string();
        assert_eq!(
            view.apply(HostEvent::Link(url.clone()), true),
            Some(url.clone())
        );
        assert_eq!(view.link.as_deref(), Some(url.as_str()));

        let mut view = SessionView::default();
        assert_eq!(view.apply(HostEvent::Link(url.clone()), false), None);
        assert_eq!(view.link.as_deref(), Some(url.as_str()));
    }

    #[test]
    fn events_fold_into_the_view() {
        let mut view = SessionView::default();
        view.apply(HostEvent::Status("hi".into()), false);
        view.apply(HostEvent::Ready, false);
        view.apply(HostEvent::GuestConnected, false);
        view.apply(HostEvent::Error("boom".into()), false);
        assert_eq!(view.status, "hi");
        assert!(view.ready && view.guest_connected);
        assert_eq!(view.error.as_deref(), Some("boom"));
    }

    #[test]
    fn a_guest_is_listed_once() {
        let mut view = SessionView::default();
        for _ in 0..3 {
            view.apply(
                HostEvent::Player {
                    slot: 1,
                    steam: 7,
                    name: "Ala".into(),
                },
                false,
            );
        }
        assert_eq!(view.guests, ["Ala"]);
    }
}
