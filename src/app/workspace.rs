use super::{
    ACCENT, Action, App, BACKGROUND, BORDER, Form, MUTED, ProjectTab, ResourceForm, SURFACE, TEXT,
    section,
};
use crate::{
    compose::{ResourceKind, Service},
    docker_hub::ImageReference,
    storage::Language,
};
use eframe::egui::{
    self, Align, Button, Color32, Context, FontFamily, FontId, Frame, Key, KeyboardShortcut,
    Layout, Margin, Modifiers, RichText, ScrollArea, Stroke, TextEdit, TextStyle, Ui,
    ViewportCommand, vec2,
};

const CHROME: Color32 = Color32::from_rgb(21, 26, 33);
const SELECTED: Color32 = Color32::from_rgb(26, 60, 59);
const UNSAVED: Color32 = Color32::from_rgb(244, 199, 122);
const NAVIGATION_WIDTH: f32 = 224.0;
const MIN_NAVIGATION_WIDTH: f32 = 192.0;
const NARROW_WINDOW: f32 = 760.0;
const INLINE_INSPECTOR: f32 = 1050.0;
const INSPECTOR_WIDTH: f32 = 322.0;
const MIN_ENVIRONMENT_WIDTH: f32 = 240.0;
const MIN_EDITOR_WIDTH: f32 = 360.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum ServiceTab {
    #[default]
    Configuration,
    Connections,
    Advanced,
    ImageInfo,
}

impl ServiceTab {
    fn title(self, language: Language) -> &'static str {
        match self {
            Self::Configuration => language.text("Konfiguration", "Configuration"),
            Self::Connections => language.text("Verknüpfungen", "Connections"),
            Self::Advanced => language.text("Erweitert", "Advanced"),
            Self::ImageInfo => language.text("Image-Info", "Image info"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Inspector {
    Environment,
    History,
}

impl Inspector {
    fn title(self, language: Language) -> &'static str {
        match self {
            Self::Environment => ".env",
            Self::History => language.text("Historie", "History"),
        }
    }
}

#[derive(Clone, Copy)]
enum EditCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

impl EditCommand {
    fn shortcut(self) -> KeyboardShortcut {
        let (modifiers, key) = match self {
            Self::Undo => (Modifiers::COMMAND, Key::Z),
            Self::Redo => (Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z),
            Self::Cut => (Modifiers::COMMAND, Key::X),
            Self::Copy => (Modifiers::COMMAND, Key::C),
            Self::Paste => (Modifiers::COMMAND, Key::V),
            Self::Delete => (Modifiers::NONE, Key::Delete),
            Self::SelectAll => (Modifiers::COMMAND, Key::A),
        };
        KeyboardShortcut::new(modifiers, key)
    }

    fn run(self, ctx: &Context, target: egui::Id) {
        ctx.memory_mut(|memory| memory.request_focus(target));
        match self {
            Self::Cut => ctx.send_viewport_cmd(ViewportCommand::RequestCut),
            Self::Copy => ctx.send_viewport_cmd(ViewportCommand::RequestCopy),
            Self::Paste => ctx.send_viewport_cmd(ViewportCommand::RequestPaste),
            _ => {
                let shortcut = self.shortcut();
                ctx.input_mut(|input| {
                    input.events.push(egui::Event::Key {
                        key: shortcut.logical_key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: shortcut.modifiers,
                    });
                });
            }
        }
        ctx.request_repaint();
    }
}

pub(super) fn configure_theme(ctx: &Context) {
    ctx.style_mut(|style| {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BACKGROUND;
        visuals.window_fill = SURFACE;
        visuals.window_stroke = Stroke::new(1.0, BORDER);
        visuals.window_corner_radius = 12.into();
        visuals.extreme_bg_color = BACKGROUND;
        visuals.faint_bg_color = CHROME;
        visuals.hyperlink_color = ACCENT;
        visuals.error_fg_color = super::ERROR;
        visuals.warn_fg_color = UNSAVED;
        visuals.selection.bg_fill = SELECTED;
        visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(60, 109, 104));
        visuals.widgets.noninteractive.fg_stroke.color = TEXT;
        visuals.widgets.noninteractive.bg_stroke.color = BORDER;
        visuals.widgets.inactive.fg_stroke.color = TEXT;
        visuals.widgets.inactive.bg_fill = BACKGROUND;
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(34, 42, 52);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(44, 55, 66);
        visuals.widgets.hovered.bg_fill = visuals.widgets.hovered.weak_bg_fill;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, MUTED);
        visuals.widgets.active.weak_bg_fill = SELECTED;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
        visuals.widgets.open.weak_bg_fill = SELECTED;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, BORDER);
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = 6.into();
            widget.expansion = 0.0;
        }
        style.visuals = visuals;
        style.text_styles.extend([
            (TextStyle::Heading, FontId::proportional(23.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]);
        style.spacing.item_spacing = vec2(8.0, 8.0);
        style.spacing.button_padding = vec2(10.0, 5.0);
        style.spacing.interact_size.y = 29.0;
        style.spacing.text_edit_width = 360.0;
        style.spacing.scroll = egui::style::ScrollStyle::floating();
    });
}

impl App {
    pub(super) fn workspace(&mut self, ctx: &Context) {
        let width = ctx.screen_rect().width();
        let narrow = width < NARROW_WINDOW;
        let inline_inspector = width >= INLINE_INSPECTOR;
        self.toolbar(ctx, narrow, inline_inspector);

        if self.document.is_some() && !narrow {
            let inspector_space = if inline_inspector {
                INSPECTOR_WIDTH
            } else {
                0.0
            };
            let max_width = (width - MIN_EDITOR_WIDTH - inspector_space).max(MIN_NAVIGATION_WIDTH);
            egui::SidePanel::left("project navigation")
                .default_width(NAVIGATION_WIDTH)
                .width_range(MIN_NAVIGATION_WIDTH..=max_width)
                .resizable(true)
                .frame(chrome_frame(14, 18))
                .show(ctx, |ui| self.project_navigation(ui, false));
        }

        if inline_inspector && let Some(inspector) = self.active_inspector(inline_inspector) {
            let panel = match inspector {
                Inspector::Environment => {
                    let max_width = (ctx.available_rect().width() - MIN_EDITOR_WIDTH)
                        .max(MIN_ENVIRONMENT_WIDTH);
                    egui::SidePanel::right("environment inspector")
                        .default_width(INSPECTOR_WIDTH)
                        .width_range(MIN_ENVIRONMENT_WIDTH..=max_width)
                        .resizable(true)
                }
                Inspector::History => egui::SidePanel::right("history inspector")
                    .exact_width(INSPECTOR_WIDTH)
                    .resizable(false),
            };
            panel
                .frame(chrome_frame(16, 18))
                .show(ctx, |ui| self.inspector_view(ui, inspector, false));
        }

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(BACKGROUND)
                    .inner_margin(Margin::symmetric(
                        if narrow { 16 } else { 24 },
                        if narrow { 12 } else { 20 },
                    )),
            )
            .show(ctx, |ui| {
                if !inline_inspector && let Some(inspector) = self.inspector {
                    self.error_banner(ui);
                    self.inspector_view(ui, inspector, true);
                    return;
                }
                self.error_banner(ui);
                if narrow && self.document.is_some() && self.compact_navigation(ui) {
                    return;
                }
                self.content(ui);
            });
    }

    fn toolbar(&mut self, ctx: &Context, narrow: bool, inline_inspector: bool) {
        egui::TopBottomPanel::top("toolbar")
            .frame(chrome_frame(16, 10))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    self.file_menu(ui);
                    self.edit_menu(ui);
                    ui.separator();
                    brand(ui);
                    if !narrow && self.document.is_some() {
                        ui.separator();
                        self.file_identity(ui);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            self.inspector_buttons(ui, inline_inspector);
                        });
                    }
                });

                if narrow && self.document.is_some() {
                    ui.horizontal(|ui| {
                        self.file_identity(ui);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            self.inspector_buttons(ui, inline_inspector);
                        });
                    });
                }
            });
    }

    fn file_menu(&mut self, ui: &mut Ui) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        ui.menu_button(t("Datei", "File"), |ui| {
            for (label, key, action) in [
                (
                    t("Neues Projekt …", "New project …"),
                    Key::N,
                    Action::NewCompose,
                ),
                (t("Öffnen …", "Open …"), Key::O, Action::Open),
            ] {
                if menu_item(
                    ui,
                    true,
                    label,
                    KeyboardShortcut::new(Modifiers::COMMAND, key),
                ) {
                    self.ask(action, ui.ctx());
                }
            }
            ui.separator();
            if menu_item(
                ui,
                self.can_save(),
                t("Speichern", "Save"),
                KeyboardShortcut::new(Modifiers::COMMAND, Key::S),
            ) {
                self.save(false);
            }
            let has_document = self.document.is_some();
            if menu_item(
                ui,
                has_document,
                t("Speichern unter …", "Save as …"),
                KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S),
            ) {
                self.save(true);
            }
            if menu_item(
                ui,
                has_document,
                t("Datei schließen", "Close file"),
                KeyboardShortcut::new(Modifiers::COMMAND, Key::W),
            ) {
                self.ask(Action::CloseDocument, ui.ctx());
            }
            ui.separator();
            if ui.button(t("Einstellungen", "Settings")).clicked() {
                ui.close_menu();
                self.settings = Some(language);
                self.settings_failed = false;
            }
        });
    }

    fn edit_menu(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let language = self.language;
        let t = |de, en| language.text(de, en);
        let focused = ctx
            .memory(|memory| memory.focused())
            .filter(|id| TextEdit::load_state(&ctx, *id).is_some());
        // The menu can take keyboard focus from the text field it was opened for.
        let target = focused.or(self.edit_menu_target).filter(|id| {
            ctx.read_response(*id)
                .is_some_and(|response| response.enabled())
        });
        let mut open = false;
        let menu = ui.menu_button(t("Bearbeiten", "Edit"), |ui| {
            open = true;
            for (command, label) in [
                (EditCommand::Undo, t("Rückgängig", "Undo")),
                (EditCommand::Redo, t("Wiederholen", "Redo")),
                (EditCommand::Cut, t("Ausschneiden", "Cut")),
                (EditCommand::Copy, t("Kopieren", "Copy")),
                (EditCommand::Paste, t("Einfügen", "Paste")),
                (EditCommand::Delete, t("Löschen", "Delete")),
                (EditCommand::SelectAll, t("Alles auswählen", "Select all")),
            ] {
                if matches!(command, EditCommand::Cut | EditCommand::SelectAll) {
                    ui.separator();
                }
                if menu_item(ui, target.is_some(), label, command.shortcut())
                    && let Some(target) = target
                {
                    command.run(&ctx, target);
                }
            }
        });
        self.edit_menu_target = if open || menu.response.is_pointer_button_down_on() {
            target
        } else {
            focused
        };
    }

    fn file_identity(&self, ui: &mut Ui) {
        let Some(document) = &self.document else {
            return;
        };
        let language = self.language;
        let t = |de, en| language.text(de, en);
        let file = document
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        ui.allocate_ui_with_layout(
            vec2((ui.available_width() * 0.25).clamp(100.0, 240.0), 29.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add(egui::Label::new(RichText::new(file.as_ref()).monospace()).truncate())
                    .on_hover_text(document.path.display().to_string());
            },
        );
        let (label, color) = if document.dirty() {
            (t("Ungespeichert", "Unsaved"), UNSAVED)
        } else {
            (t("Gespeichert", "Saved"), MUTED)
        };
        ui.label(RichText::new(label).small().color(color));
    }

    pub(super) fn active_inspector(&self, inline: bool) -> Option<Inspector> {
        let show_environment = inline
            && self.environment_sidebar_enabled
            && self.project_tab == ProjectTab::Services
            && self.selected.is_some();
        self.inspector
            .or(show_environment.then_some(Inspector::Environment))
    }

    pub(super) fn close_inspector(&mut self, inspector: Inspector, inline: bool) {
        self.inspector = None;
        if inline && inspector == Inspector::Environment {
            self.environment_sidebar_enabled = false;
        }
    }

    fn inspector_buttons(&mut self, ui: &mut Ui, inline: bool) {
        for inspector in [Inspector::History, Inspector::Environment] {
            let active = self.active_inspector(inline) == Some(inspector);
            if choice(ui, Button::new(inspector.title(self.language)), active) {
                if active {
                    self.close_inspector(inspector, inline);
                } else {
                    if inline && inspector == Inspector::Environment {
                        self.environment_sidebar_enabled = true;
                    }
                    self.inspector = Some(inspector);
                }
            }
        }
    }

    fn inspector_view(&mut self, ui: &mut Ui, inspector: Inspector, full_width: bool) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        ui.horizontal(|ui| {
            ui.heading(inspector.title(self.language));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if full_width {
                    t("Zurück zum Editor", "Back to editor")
                } else {
                    t("Schließen", "Close")
                };
                if ui.button(label).clicked() {
                    self.close_inspector(inspector, !full_width);
                    ui.ctx().request_repaint();
                }
            });
        });
        ui.add_space(4.0);
        match inspector {
            Inspector::Environment => self.environment_content(ui),
            Inspector::History => self.history_content(ui),
        }
    }

    fn compact_navigation(&mut self, ui: &mut Ui) -> bool {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        ui.horizontal(|ui| {
            let label = if self.navigation_open {
                t("Zum Editor", "Back to editor")
            } else {
                t("Projekt", "Project")
            };
            if choice(ui, Button::new(label), self.navigation_open) {
                self.navigation_open = !self.navigation_open;
            }
            if !self.navigation_open {
                ui.label(RichText::new(project_title(self.project_tab, language)).color(MUTED));
            }
        });
        ui.add_space(4.0);
        if self.navigation_open {
            Frame::new()
                .fill(CHROME)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(10)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.project_navigation(ui, true);
                });
        }
        self.navigation_open
    }

    fn project_navigation(&mut self, ui: &mut Ui, compact: bool) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        let Some(document) = &self.document else {
            return;
        };
        let sections = [
            (ProjectTab::Services, document.services.len()),
            (
                ProjectTab::Resources(ResourceKind::Network),
                document.resources.list(ResourceKind::Network).entries.len(),
            ),
            (
                ProjectTab::Resources(ResourceKind::Volume),
                document.resources.list(ResourceKind::Volume).entries.len(),
            ),
        ];
        let mut change_section = None;
        let mut section_names = |ui: &mut Ui| {
            for (tab, count) in sections {
                let title = project_title(tab, language);
                let active = self.project_tab == tab;
                let clicked = if compact {
                    choice(ui, Button::new(format!("{title}  {count}")), active)
                } else {
                    navigation_row(ui, title, count, active)
                };
                if clicked {
                    change_section = Some(tab);
                }
            }
        };
        if compact {
            ui.horizontal_wrapped(section_names);
        } else {
            section_names(ui);
        }
        if let Some(tab) = change_section
            && self.project_tab != tab
        {
            self.project_tab = tab;
            self.resource_query.clear();
            self.resource_focus = None;
            self.jump_to_resource = false;
        }

        let Some(document) = &self.document else {
            return;
        };
        let tab = self.project_tab;
        let (query, add_label, hint, can_add) = match tab {
            ProjectTab::Services => (
                &mut self.service_query,
                t("+ Service hinzufügen", "+ Add service"),
                t("Services suchen …", "Find services …"),
                true,
            ),
            ProjectTab::Resources(ResourceKind::Network) => (
                &mut self.resource_query,
                t("+ Netzwerk hinzufügen", "+ Add network"),
                t("Netzwerke suchen …", "Find networks …"),
                document.resources.networks.can_add,
            ),
            ProjectTab::Resources(ResourceKind::Volume) => (
                &mut self.resource_query,
                t("+ Volume hinzufügen", "+ Add volume"),
                t("Volumes suchen …", "Find volumes …"),
                document.resources.volumes.can_add,
            ),
        };

        ui.separator();
        let mut search_and_create = |ui: &mut Ui| {
            let available = ui.available_width();
            let search_width = if compact {
                available - button_row_width(ui, &[add_label]) - ui.spacing().item_spacing.x
            } else {
                available
            };
            let search = ui.add_sized(
                [search_width.max(1.0), 30.0],
                egui::TextEdit::singleline(query)
                    .id_salt(("search project", tab))
                    .hint_text(hint),
            );
            let add = ui.add_enabled(can_add, primary_button(add_label));
            focus_ring(ui, &add);
            let add = if can_add {
                add
            } else {
                add.on_disabled_hover_text(t(
                    "Definitionen sind aufgrund geteilter, geerbter oder nicht unterstützter YAML-Werte schreibgeschützt.",
                    "Definitions are read-only because the YAML uses shared, inherited, or unsupported values.",
                ))
            };
            (search.changed(), add.clicked())
        };
        let (changed, add) = if compact {
            ui.horizontal(search_and_create).inner
        } else {
            search_and_create(ui)
        };
        let query = query.trim().to_ascii_lowercase();
        if add {
            self.navigation_open = false;
            match tab {
                ProjectTab::Services => self.open_new_service(ui.ctx()),
                ProjectTab::Resources(kind) => self.start_form(
                    Form::Resource(ResourceForm::new(kind, String::new(), None, true)),
                    ui.ctx(),
                ),
            }
        }

        let Some(document) = &self.document else {
            return;
        };
        let (names, active): (Vec<_>, _) = match tab {
            ProjectTab::Services => (
                document
                    .services
                    .iter()
                    .map(|service| service.name.as_str())
                    .collect(),
                self.selected
                    .and_then(|index| document.services.get(index))
                    .map(|service| service.name.as_str()),
            ),
            ProjectTab::Resources(kind) => (
                document
                    .resources
                    .list(kind)
                    .entries
                    .iter()
                    .map(|resource| resource.key.as_str())
                    .collect(),
                self.selected_resource(kind)
                    .map(|resource| resource.key.as_str()),
            ),
        };
        ui.add_space(4.0);
        let mut list = ScrollArea::vertical()
            .id_salt((self.document_view, tab, "project entries"))
            .auto_shrink([false, true]);
        if changed {
            list = list.vertical_scroll_offset(0.0);
        }
        let mut selected = None;
        let mut jumped = false;
        list.show(ui, |ui| {
            let mut found = false;
            for (index, name) in names.iter().copied().enumerate() {
                if query.is_empty() || name.to_ascii_lowercase().contains(&query) {
                    found = true;
                    let response = ui
                        .push_id(name, |ui| entry_row(ui, name, active == Some(name)))
                        .inner;
                    if self.jump_to_resource && active == Some(name) {
                        response.scroll_to_me(None);
                        jumped = true;
                    }
                    if response.clicked() {
                        selected = Some(index);
                    }
                }
            }
            if !found && !names.is_empty() {
                ui.label(RichText::new(t("Keine Treffer", "No matches")).color(MUTED));
            }
        });
        if jumped {
            self.jump_to_resource = false;
        }
        if let Some(index) = selected {
            match tab {
                ProjectTab::Services => self.select_service(index),
                ProjectTab::Resources(_) => {
                    self.resource_focus = Some(names[index].to_owned());
                    self.navigation_open = false;
                }
            }
        }
    }

    pub(super) fn select_service(&mut self, index: usize) {
        if self.selected != Some(index) {
            self.selected = Some(index);
            self.service_tab = ServiceTab::default();
        }
        self.project_tab = ProjectTab::Services;
        self.navigation_open = false;
        self.resource_focus = None;
        self.jump_to_resource = false;
    }

    pub(super) fn select_first_service(&mut self) {
        self.service_tab = ServiceTab::default();
        self.selected = self
            .document
            .as_ref()
            .filter(|document| !document.services.is_empty())
            .map(|_| 0);
    }

    fn open_new_service(&mut self, ctx: &Context) {
        self.navigation_open = false;
        self.start_form(
            Form::NewService {
                name: String::new(),
                image: String::new(),
            },
            ctx,
        );
    }

    pub(super) fn content(&mut self, ui: &mut Ui) {
        let Some(document) = &self.document else {
            self.welcome(ui);
            return;
        };
        if document.has_include {
            ui.label(
                RichText::new(self.language.text(
                    "Diese Ansicht zeigt die Services aus der geöffneten Datei.",
                    "This view shows services from the open file.",
                ))
                .color(MUTED),
            );
        }
        match self.project_tab {
            ProjectTab::Resources(kind) => {
                ScrollArea::vertical()
                    .id_salt((
                        self.document_view,
                        kind,
                        self.selected_resource(kind)
                            .map(|resource| resource.key.as_str()),
                    ))
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.resource_view(ui, kind));
            }
            ProjectTab::Services => {
                if let Some(service) = self
                    .selected
                    .and_then(|index| document.services.get(index))
                    .cloned()
                {
                    self.service_detail(ui, &service);
                } else {
                    self.empty_services(ui);
                }
            }
        }
    }

    fn welcome(&mut self, ui: &mut Ui) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        ui.add_space((ui.available_height() * 0.20).clamp(24.0, 140.0));
        ui.vertical_centered(|ui| {
            Frame::new()
                .fill(SURFACE)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(14)
                .inner_margin(28)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().clamp(1.0, 420.0));
                    ui.vertical_centered(|ui| {
                        brand_mark(ui, 42.0);
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(t("Compose-Projekt öffnen", "Open a Compose project"))
                                .size(27.0)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(t(
                                "Wähle eine Compose-Datei oder ziehe sie in dieses Fenster.",
                                "Choose a Compose file or drop it into this window.",
                            ))
                            .color(MUTED),
                        );
                        ui.add_space(12.0);
                        let open = t("Compose-Datei öffnen", "Open Compose file");
                        let new = t("Neues Projekt", "New project");
                        let width = button_row_width(ui, &[open, new]);
                        ui.horizontal(|ui| {
                            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                            if primary_action(ui, open) {
                                self.ask(Action::Open, ui.ctx());
                            }
                            if ui.button(new).clicked() {
                                self.ask(Action::NewCompose, ui.ctx());
                            }
                        });
                    });
                });
        });
    }

    fn empty_services(&mut self, ui: &mut Ui) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        section(ui, t("Keine Services", "No services"), |ui| {
            ui.label(RichText::new(t(
                "Füge einen Service hinzu, um sein Image, seine Ports und Variablen zu konfigurieren.",
                "Add a service to configure its image, ports, and variables.",
            )).color(MUTED));
            if primary_action(ui, t("+ Service hinzufügen", "+ Add service")) {
                self.open_new_service(ui.ctx());
            }
        });
    }

    fn service_detail(&mut self, ui: &mut Ui, service: &Service) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        ui.add(egui::Label::new(RichText::new(&service.name).size(32.0).strong()).truncate())
            .on_hover_text(&service.name);
        let image = self.hub_image(service);
        if self.service_tab == ServiceTab::ImageInfo && image.is_none() {
            self.service_tab = ServiceTab::Configuration;
        }
        ui.horizontal_wrapped(|ui| {
            for tab in [
                ServiceTab::Configuration,
                ServiceTab::Connections,
                ServiceTab::Advanced,
                ServiceTab::ImageInfo,
            ] {
                if tab != ServiceTab::ImageInfo || image.is_some() {
                    if choice(
                        ui,
                        Button::new(tab.title(self.language)),
                        self.service_tab == tab,
                    ) {
                        self.service_tab = tab;
                    }
                }
            }
        });
        ui.separator();
        ScrollArea::vertical()
            .id_salt((self.document_view, &service.name, self.service_tab))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(4.0);
                match self.service_tab {
                    ServiceTab::Configuration => {
                        if !service.image_editable || !service.restart_editable || !service.ports_editable || !service.variables_editable {
                            ui.label(RichText::new(t(
                                "Geteilte, geerbte oder nicht unterstützte YAML-Werte sind schreibgeschützt.",
                                "Shared, inherited, or unsupported YAML values are read-only.",
                            )).color(MUTED));
                        }
                        self.service_configuration(ui, service);
                    }
                    ServiceTab::Connections => self.service_parameter_sections(ui, service),
                    ServiceTab::Advanced => self.additional_parameters(ui, service),
                    ServiceTab::ImageInfo => {
                        if let Some((name, reference)) = image {
                            self.docker_hub_section(ui, &name, &reference, service);
                        }
                    }
                }
            });
    }

    fn hub_image(&self, service: &Service) -> Option<(String, ImageReference)> {
        let image = service.image.as_deref()?;
        let name = if image.contains('$') {
            let preview = self.document.as_ref()?.environment.preview(image);
            if preview.unresolved {
                return None;
            }
            preview.text
        } else {
            image.to_owned()
        };
        let reference = ImageReference::parse(&name)?;
        Some((name, reference))
    }
}

fn menu_item(ui: &mut Ui, enabled: bool, label: &str, shortcut: KeyboardShortcut) -> bool {
    let shortcut = ui.ctx().format_shortcut(&shortcut);
    if ui
        .add_enabled(enabled, Button::new(label).shortcut_text(shortcut))
        .clicked()
    {
        ui.close_menu();
        true
    } else {
        false
    }
}

fn project_title(tab: ProjectTab, language: Language) -> &'static str {
    match tab {
        ProjectTab::Services => "Services",
        ProjectTab::Resources(kind) => kind.title(language),
    }
}

fn navigation_row(ui: &mut Ui, title: &str, count: usize, selected: bool) -> bool {
    navigation_button(
        ui,
        Button::new(RichText::new(title).strong())
            .shortcut_text(RichText::new(count.to_string()).small().color(MUTED)),
        title,
        selected,
    )
    .clicked()
}

fn entry_row(ui: &mut Ui, name: &str, selected: bool) -> egui::Response {
    navigation_button(
        ui,
        Button::new(RichText::new(name).monospace()).truncate(),
        name,
        selected,
    )
}

fn navigation_button(
    ui: &mut Ui,
    button: Button<'_>,
    label: &str,
    selected: bool,
) -> egui::Response {
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
        // Keep project entries aligned with the section labels.
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            let response = ui.add(
                button
                    .min_size(vec2(ui.available_width(), 30.0))
                    .selected(selected)
                    .corner_radius(6),
            );
            focus_ring(ui, &response);
            response.on_hover_text(label)
        })
        .inner
    })
    .inner
}

fn choice(ui: &mut Ui, button: Button<'_>, selected: bool) -> bool {
    let response = ui.add(button.selected(selected).corner_radius(6));
    focus_ring(ui, &response);
    response.clicked()
}

fn chrome_frame(horizontal: i8, vertical: i8) -> Frame {
    Frame::new()
        .fill(CHROME)
        .stroke(Stroke::new(1.0, BORDER))
        .inner_margin(Margin::symmetric(horizontal, vertical))
}

fn brand(ui: &mut Ui) {
    brand_mark(ui, 28.0);
    ui.label(RichText::new(crate::APP_NAME).size(19.0).strong());
}

fn brand_mark(ui: &mut Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 7, SELECTED);
    for (column, row) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let edge = size * 0.18;
        let min = rect.min
            + vec2(
                size * 0.29 + column as f32 * edge * 1.30,
                size * 0.29 + row as f32 * edge * 1.30,
            );
        painter.rect_filled(egui::Rect::from_min_size(min, vec2(edge, edge)), 1, ACCENT);
    }
}

pub(super) fn button_row_width(ui: &Ui, labels: &[&str]) -> f32 {
    let font = TextStyle::Button.resolve(ui.style());
    let text_width = ui.fonts(|fonts| {
        labels
            .iter()
            .map(|label| {
                fonts
                    .layout_no_wrap((*label).to_owned(), font.clone(), TEXT)
                    .size()
                    .x
                    .ceil()
            })
            .sum::<f32>()
    });
    text_width
        + (labels.len() as f32) * (2.0 * ui.spacing().button_padding.x)
        + (labels.len().saturating_sub(1) as f32) * ui.spacing().item_spacing.x
}

fn primary_button(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(BACKGROUND).strong())
        .fill(ACCENT)
        .stroke(Stroke::new(1.0, ACCENT))
}

pub(super) fn primary_action(ui: &mut Ui, label: &str) -> bool {
    let response = ui.add(primary_button(label));
    focus_ring(ui, &response);
    response.clicked()
}

fn focus_ring(ui: &Ui, response: &egui::Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            8,
            Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Outside,
        );
    }
}
