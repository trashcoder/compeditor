mod additional_parameters;
mod resources;
mod service_parameters;

use additional_parameters::ParameterForm;
use resources::ResourceForm;
use service_parameters::AssignmentForm;

use crate::{
    compose::{
        Compose, Edit, Error, LongPort, Port, ProjectEdit, ResourceKind, SectionKind, Service,
        Variable,
    },
    docker_hub::{self, DockerHubInfo, DockerHubTag, ImageReference, ParameterRequirement},
    environment::{self, Environment, Preview, SaveError},
    history::{Change, Notice, Target},
    storage::{self, Language},
};
use eframe::egui::{self, Color32, Frame, Id, Layout, RichText, Stroke, Ui, Vec2};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

const BACKGROUND: Color32 = Color32::from_rgb(18, 24, 33);
const SURFACE: Color32 = Color32::from_rgb(26, 35, 47);
const BORDER: Color32 = Color32::from_rgb(48, 62, 78);
const TEXT: Color32 = Color32::from_rgb(232, 239, 246);
const MUTED: Color32 = Color32::from_rgb(163, 181, 199);
const ACCENT: Color32 = Color32::from_rgb(112, 225, 205);
const ERROR: Color32 = Color32::from_rgb(255, 174, 168);

enum Action {
    NewCompose,
    Open,
    OpenPath(PathBuf),
    CloseDocument,
    Quit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum ProjectTab {
    #[default]
    Services,
    Resources(ResourceKind),
}

enum Form {
    Resource(ResourceForm),
    Parameter(ParameterForm),
    Assignment(AssignmentForm),
    SectionYaml {
        kind: SectionKind,
        yaml: String,
        add: bool,
    },
    NewService {
        name: String,
        image: String,
    },
    Image {
        repository: String,
        tag: Option<String>,
        source: String,
    },
    Port(Option<usize>, Port),
    Variable {
        index: Option<usize>,
        key: String,
        value: String,
        inherit: bool,
    },
}

enum FormSubmission {
    Edit(Edit),
    Project(ProjectEdit),
    NewService { name: String, image: Option<String> },
}

struct EnvEditor {
    key: String,
    value: String,
    original: Option<String>,
    error: Option<SaveError>,
    jump: bool,
}

pub struct App {
    config_path: Option<PathBuf>,
    language: Language,
    settings: Option<Language>,
    settings_failed: bool,
    document: Option<Compose>,
    document_view: u64,
    show_env_values: bool,
    env_editor: Option<EnvEditor>,
    selected: Option<usize>,
    project_tab: ProjectTab,
    resource_focus: Option<String>,
    jump_to_resource: bool,
    form: Option<Form>,
    form_error: Option<Error>,
    error: Option<Error>,
    pending: Option<Action>,
    close_allowed: bool,
    docker_hub_image: Option<String>,
    docker_hub_rx: Option<Receiver<Result<DockerHubInfo, String>>>,
    docker_hub_info: Option<Result<DockerHubInfo, String>>,
    docker_tags_key: Option<String>,
    docker_tags_rx: Option<Receiver<Result<Vec<DockerHubTag>, String>>>,
    docker_tags: Option<Result<Vec<DockerHubTag>, String>>,
}

impl App {
    pub fn new(ctx: &egui::Context, path: Option<PathBuf>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BACKGROUND;
        visuals.window_fill = SURFACE;
        visuals.override_text_color = Some(TEXT);
        visuals.widgets.inactive.bg_fill = SURFACE;
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(37, 49, 62);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(41, 59, 72);
        visuals.selection.bg_fill = Color32::from_rgb(28, 100, 91);
        visuals.hyperlink_color = ACCENT;
        visuals.error_fg_color = ERROR;
        ctx.set_visuals(visuals);
        ctx.style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(10.0, 8.0);
            style.spacing.button_padding = egui::vec2(11.0, 6.0);
            style.spacing.interact_size.y = 28.0;
            style.spacing.text_edit_width = 360.0;
        });
        let config_path = storage::config_path();
        let mut app = Self {
            language: Language::load(config_path.as_deref()),
            config_path,
            settings: None,
            settings_failed: false,
            document: None,
            document_view: 0,
            show_env_values: false,
            env_editor: None,
            selected: None,
            project_tab: ProjectTab::Services,
            resource_focus: None,
            jump_to_resource: false,
            form: None,
            form_error: None,
            error: None,
            pending: None,
            close_allowed: false,
            docker_hub_image: None,
            docker_hub_rx: None,
            docker_hub_info: None,
            docker_tags_key: None,
            docker_tags_rx: None,
            docker_tags: None,
        };
        if let Some(path) = path {
            app.open(path);
        }
        app
    }

    fn dirty(&self) -> bool {
        self.document.as_ref().is_some_and(Compose::dirty)
    }

    fn can_save(&self) -> bool {
        self.document.as_ref().is_some_and(Compose::can_save)
    }

    fn open(&mut self, path: PathBuf) {
        match Compose::open(&path) {
            Ok(document) => self.set_document(Some(document)),
            Err(error) => self.error = Some(error),
        }
    }

    fn set_document(&mut self, document: Option<Compose>) {
        self.document = document;
        self.document_view += 1;
        self.show_env_values = false;
        self.env_editor = None;
        self.selected = None;
        self.project_tab = ProjectTab::Services;
        self.resource_focus = None;
        self.jump_to_resource = false;
        self.form = None;
        self.form_error = None;
        self.error = None;
        self.docker_hub_image = None;
        self.docker_hub_rx = None;
        self.docker_hub_info = None;
    }

    fn ask(&mut self, action: Action, ctx: &egui::Context) {
        if self.dirty() || self.form.is_some() {
            self.pending = Some(action);
        } else {
            self.run(action, ctx);
        }
    }

    fn run(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::NewCompose => {
                let language = self.language;
                let mut dialog = rfd::FileDialog::new().set_title(language.text(
                    "Ordner für neue Compose-Datei auswählen",
                    "Choose folder for new Compose file",
                ));
                if let Some(parent) = self.document.as_ref().and_then(|doc| doc.path.parent()) {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(directory) = dialog.pick_folder() {
                    match Compose::create_in(&directory) {
                        Ok(document) => self.set_document(Some(document)),
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            Action::Open => {
                let lang = self.language;
                let mut dialog = rfd::FileDialog::new()
                    .set_title(lang.text("Compose-Datei öffnen", "Open Compose file"))
                    .add_filter("YAML", &["yaml", "yml"]);
                if let Some(parent) = self.document.as_ref().and_then(|doc| doc.path.parent()) {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(path) = dialog.pick_file() {
                    self.open(path);
                }
            }
            Action::OpenPath(path) => self.open(path),
            Action::CloseDocument => self.set_document(None),
            Action::Quit => {
                self.close_allowed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn save(&mut self, save_as: bool) -> bool {
        let Some(document) = &mut self.document else {
            return false;
        };
        let path = if save_as {
            let mut dialog = rfd::FileDialog::new()
                .add_filter("YAML", &["yaml", "yml"])
                .set_title(
                    self.language
                        .text("Compose-Datei speichern unter", "Save Compose file as"),
                )
                .set_file_name(
                    document
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                );
            if let Some(parent) = document.path.parent() {
                dialog = dialog.set_directory(parent);
            }
            let Some(mut path) = dialog.save_file() else {
                return false;
            };
            if path.extension().is_none() {
                path.set_extension("yaml");
                if path.exists() {
                    self.error = Some(Error::Input(
                        "Diese Datei existiert bereits. Wähle den vollständigen Dateinamen.",
                        "This file already exists. Choose the full file name.",
                    ));
                    return false;
                }
            }
            path
        } else {
            document.path.clone()
        };
        let previous_path = document.path.clone();
        match document.save(path) {
            Ok(()) => {
                if document.path != previous_path {
                    self.show_env_values = false;
                    self.env_editor = None;
                }
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    fn apply(&mut self, edit: Edit) -> bool {
        let Some((document, index)) = self.document.as_mut().zip(self.selected) else {
            return false;
        };
        match document.edit(index, edit) {
            Ok(()) => {
                self.error = None;
                self.form_error = None;
                true
            }
            Err(error) => {
                if self.form.is_some() {
                    self.form_error = Some(error);
                } else {
                    self.error = Some(error);
                }
                false
            }
        }
    }

    fn apply_project(&mut self, edit: ProjectEdit) -> bool {
        let Some(document) = &mut self.document else {
            return false;
        };
        match document.edit_project(self.selected, edit) {
            Ok(()) => {
                self.error = None;
                self.form_error = None;
                true
            }
            Err(error) => {
                if self.form.is_some() {
                    self.form_error = Some(error);
                } else {
                    self.error = Some(error);
                }
                false
            }
        }
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        let t = |de, en| self.language.text(de, en);
        let open = t("Öffnen", "Open");
        let save = t("Speichern", "Save");
        let save_as = t("Speichern unter", "Save As");
        let close = t("Datei schließen", "Close file");
        let new_compose = t("Neu", "New");
        let settings = t("Einstellungen", "Settings");
        egui::TopBottomPanel::top("toolbar")
            .frame(
                Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(20, 12)),
            )
            .show(ctx, |ui| {
                let compact = ui.available_width() < 590.0;
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Docksie").size(23.0).strong().color(ACCENT));
                    ui.add_space(8.0);
                    ui.separator();
                    if ui.button(new_compose).clicked() {
                        self.ask(Action::NewCompose, ctx);
                    }
                    if ui.button(open).clicked() {
                        self.ask(Action::Open, ctx);
                    }
                    if ui
                        .add_enabled(self.can_save(), egui::Button::new(save))
                        .clicked()
                    {
                        self.save(false);
                    }
                    if ui
                        .add_enabled(self.document.is_some(), egui::Button::new(save_as))
                        .clicked()
                    {
                        self.save(true);
                    }
                    if ui
                        .add_enabled(self.document.is_some(), egui::Button::new(close))
                        .clicked()
                    {
                        self.ask(Action::CloseDocument, ctx);
                    }
                    let mut settings_button = |ui: &mut Ui| {
                        if ui.button(settings).clicked() {
                            self.settings = Some(self.language);
                            self.settings_failed = false;
                        }
                    };
                    if compact {
                        settings_button(ui);
                    } else {
                        ui.with_layout(Layout::right_to_left(egui::Align::Center), settings_button);
                    }
                });
            });
    }

    fn project_navigation(&mut self, ui: &mut Ui) {
        if self.document.is_none() {
            return;
        }
        let language = self.language;
        ui.horizontal_wrapped(|ui| {
            let tabs = [
                (ProjectTab::Services, "Services"),
                (
                    ProjectTab::Resources(ResourceKind::Network),
                    ResourceKind::Network.title(language),
                ),
                (
                    ProjectTab::Resources(ResourceKind::Volume),
                    ResourceKind::Volume.title(language),
                ),
            ];
            for (tab, title) in tabs {
                if ui
                    .selectable_label(self.project_tab == tab, title)
                    .clicked()
                    && self.project_tab != tab
                {
                    self.project_tab = tab;
                    self.selected = None;
                    self.resource_focus = None;
                    self.jump_to_resource = false;
                }
            }
        });
        ui.add_space(6.0);
    }

    fn content(&mut self, ui: &mut Ui) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        if let Some(message) = self.error.as_ref().map(|error| error.message(language)) {
            Frame::new()
                .fill(SURFACE)
                .stroke(Stroke::new(1.0, ERROR))
                .corner_radius(10)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(message).color(ERROR));
                        if ui.button(t("Schließen", "Dismiss")).clicked() {
                            self.error = None;
                        }
                    });
                });
            ui.add_space(8.0);
        }
        let Some(document) = self.document.as_ref() else {
            ui.add_space(95.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(t("Compose-Datei öffnen", "Open a Compose file"))
                        .size(32.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(t(
                        "Wähle eine YAML-Datei oder ziehe sie in dieses Fenster.",
                        "Choose a YAML file or drop it into this window.",
                    ))
                    .color(MUTED),
                );
                ui.add_space(12.0);
                if primary(ui, t("Datei auswählen", "Choose file")) {
                    self.ask(Action::Open, ui.ctx());
                }
                if ui
                    .button(t("Neue Compose-Datei", "New Compose file"))
                    .clicked()
                {
                    self.ask(Action::NewCompose, ui.ctx());
                }
            });
            return;
        };

        if let ProjectTab::Resources(kind) = self.project_tab {
            self.resource_view(ui, kind);
            return;
        }

        if let Some(index) = self.selected {
            if ui.button(t("Alle Services", "All services")).clicked() {
                self.selected = None;
                return;
            }
            if let Some(service) = document.services.get(index) {
                let service = service.clone();
                ui.add_space(8.0);
                self.details(ui, &service);
            }
            return;
        }

        ui.label(RichText::new(t("Services", "Services")).size(34.0).strong());
        let mut add_service = false;
        if ui.button(t("Service hinzufügen", "Add service")).clicked() {
            add_service = true;
        }
        let file = document
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(file.as_ref()).color(MUTED))
                .on_hover_text(document.path.display().to_string());
            if document.dirty() {
                ui.label(RichText::new(t("Ungespeichert", "Unsaved")).color(ACCENT));
            }
        });
        if document.has_include {
            ui.label(
                RichText::new(t(
                    "Externe include-Dateien sind in dieser Ansicht ausgeblendet.",
                    "External include files are outside this view.",
                ))
                .color(MUTED),
            );
        }
        ui.add_space(14.0);
        if document.services.is_empty() {
            ui.label(t(
                "Diese Datei enthält keine Services.",
                "This file has no services.",
            ));
        }
        let width = ui.available_width();
        let cols = ((width + 12.0) / 280.0).floor().clamp(1.0, 4.0) as usize;
        let card_width = ((width - (cols.saturating_sub(1) as f32 * 10.0)) / cols as f32).floor();
        for (row, services) in document.services.chunks(cols).enumerate() {
            ui.horizontal_top(|ui| {
                for (col, service) in services.iter().enumerate() {
                    if service_card(
                        ui,
                        service,
                        &document.environment,
                        language,
                        card_width,
                        row * cols + col,
                    ) {
                        self.selected = Some(row * cols + col);
                    }
                }
            });
            ui.add_space(3.0);
        }
        if add_service {
            self.start_form(
                Form::NewService {
                    name: String::new(),
                    image: String::new(),
                },
                ui.ctx(),
            );
        }
    }

    fn details(&mut self, ui: &mut Ui, service: &Service) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        ui.label(RichText::new(&service.name).size(33.0).strong());
        if self.dirty() {
            ui.label(RichText::new(t("Ungespeichert", "Unsaved")).color(ACCENT));
        }
        if !service.image_editable
            || !service.restart_editable
            || !service.ports_editable
            || !service.variables_editable
        {
            ui.label(RichText::new(t("Schreibgeschützte Felder verwenden geteilte, geerbte oder nicht unterstützte YAML-Werte.", "Read-only fields use shared, inherited, or unsupported YAML values.")).color(MUTED));
        }
        ui.add_space(9.0);
        section(ui, "Image", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(
                        service
                            .image
                            .as_deref()
                            .unwrap_or(t("Nicht gesetzt", "Not set")),
                    )
                    .monospace(),
                );
                if ui
                    .add_enabled(
                        service.image_editable,
                        egui::Button::new(t("Bearbeiten", "Edit")),
                    )
                    .clicked()
                {
                    self.start_form(
                        image_form(service.image.clone().unwrap_or_default()),
                        ui.ctx(),
                    );
                }
                if service.image.is_some()
                    && ui
                        .add_enabled(
                            service.image_editable,
                            egui::Button::new(t("Entfernen", "Remove")),
                        )
                        .clicked()
                {
                    self.apply(Edit::Image(None));
                }
            });
            if let Some((image, document)) = service.image.as_deref().zip(self.document.as_ref()) {
                preview_line(
                    ui,
                    Some(image),
                    &document.environment.preview(image),
                    language,
                    true,
                );
            }
            if let Some(image) = &service.image {
                self.env_links(ui, environment::references(image));
            }
        });
        let mut restart = service.restart.clone();
        section(ui, t("Neustart-Richtlinie", "Restart policy"), |ui| {
            ui.label(
                RichText::new(t(
                    "Legt fest, ob Docker den Container nach dem Beenden neu startet.",
                    "Controls whether Docker restarts the container after it exits.",
                ))
                .color(MUTED),
            );
            let selected_text = restart
                .as_deref()
                .map(|value| restart_policy_label(value, language))
                .unwrap_or(t(
                    "Compose-Standard (nicht gesetzt)",
                    "Compose default (unset)",
                ))
                .to_owned();
            ui.add_enabled_ui(service.restart_editable, |ui| {
                egui::ComboBox::from_id_salt(("restart-policy", &service.name))
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut restart,
                            None,
                            t("Standard (nicht gesetzt)", "Default (unset)"),
                        );
                        for (value, label) in [
                            ("no", t("Nie (no)", "Never (no)")),
                            ("always", t("Immer (always)", "Always")),
                            (
                                "on-failure",
                                t("Bei Fehler (unbegrenzt)", "On failure (unlimited)"),
                            ),
                            (
                                "on-failure:3",
                                t("Bei Fehler (3 Versuche)", "On failure (3 retries)"),
                            ),
                            (
                                "on-failure:5",
                                t("Bei Fehler (5 Versuche)", "On failure (5 retries)"),
                            ),
                            (
                                "on-failure:10",
                                t("Bei Fehler (10 Versuche)", "On failure (10 retries)"),
                            ),
                            (
                                "unless-stopped",
                                t("Außer manuell gestoppt", "Unless stopped"),
                            ),
                        ] {
                            ui.selectable_value(&mut restart, Some(value.to_owned()), label);
                        }
                    });
            });
        });
        if restart != service.restart {
            self.apply(Edit::Restart(restart));
        }
        let resolved_image = service.image.as_deref().and_then(|image| {
            if image.contains('$') {
                self.document
                    .as_ref()
                    .map(|document| document.environment.preview(image))
                    .filter(|preview| !preview.unresolved && !preview.text.is_empty())
                    .map(|preview| preview.text)
            } else {
                Some(image.to_owned())
            }
        });
        if let Some(image) = resolved_image.as_deref()
            && let Some(reference) = ImageReference::parse(image)
        {
            self.docker_hub_section(ui, image, &reference, service);
        }
        section(ui, "Ports", |ui| {
            ui.label(
                RichText::new(t(
                    "Host-Port (veröffentlicht) → Container-Port (intern). Erreichbarkeit von außen hängt von Host-IP und Firewall ab.",
                    "Host port (published) → container port (internal). External access depends on the host IP and firewall.",
                ))
                .color(MUTED),
            );
            for (index, port) in service.ports.iter().enumerate() {
                ui.push_id(index, |ui| {
                    let summary = port.summary();
                    let label = port_label(port, language);
                    let ctx = ui.ctx().clone();
                    row(ui, &label, service.ports_editable, language, |edit| {
                        if edit {
                            self.start_form(Form::Port(Some(index), port.clone()), &ctx);
                        } else {
                            self.apply(Edit::RemovePort(index));
                        }
                    });
                    if let Some(document) = &self.document {
                        preview_line(
                            ui,
                            Some(&summary),
                            &document.environment.preview(&summary),
                            language,
                            true,
                        );
                    }
                    self.env_links(ui, environment::references(&summary));
                });
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        service.ports_editable,
                        egui::Button::new(t("Port hinzufügen", "Add port")),
                    )
                    .clicked()
                {
                    self.start_form(Form::Port(None, Port::Short(String::new())), ui.ctx());
                }
                if ui
                    .add_enabled(
                        service.ports_editable,
                        egui::Button::new(t("Ausführlichen Port hinzufügen", "Add detailed port")),
                    )
                    .clicked()
                {
                    self.start_form(Form::Port(None, Port::Long(LongPort::default())), ui.ctx());
                }
            });
        });
        self.service_parameter_sections(ui, service);
        self.additional_parameters(ui, service);
        section(ui, t("Umgebungsvariablen", "Environment variables"), |ui| {
            if service
                .variables
                .iter()
                .any(|var| var.value.as_ref().is_none_or(|value| value.contains('$')))
            {
                ui.checkbox(
                    &mut self.show_env_values,
                    t("Vorschauwerte anzeigen", "Show preview values"),
                );
            }
            for (index, var) in service.variables.iter().enumerate() {
                ui.push_id(index, |ui| {
                    let ctx = ui.ctx().clone();
                    let label = match var.value.as_deref() {
                        Some("") => format!("{} = \"\"", var.key),
                        Some(value) => format!("{} = {value}", var.key),
                        None => format!("{} · {}", var.key, t("Aus Umgebung", "From environment")),
                    };
                    row(ui, &label, service.variables_editable, language, |edit| {
                        if edit {
                            self.start_form(variable_form(Some(index), var.clone()), &ctx);
                        } else {
                            self.apply(Edit::RemoveVariable(index));
                        }
                    });
                    let names = match var.value.as_deref() {
                        Some(value) => environment::references(value),
                        None => vec![var.key.as_str()],
                    };
                    self.env_links(ui, names);
                    if let Some(document) = &self.document {
                        let original = var.value.as_deref();
                        let preview =
                            match original {
                                Some(value) if value.contains('$') => {
                                    document.environment.preview(value)
                                }
                                None => document.environment.value(&var.key).cloned().unwrap_or(
                                    Preview {
                                        unresolved: true,
                                        ..Preview::default()
                                    },
                                ),
                                Some(_) => return,
                            };
                        preview_line(ui, original, &preview, language, self.show_env_values);
                    }
                });
            }
            if ui
                .add_enabled(
                    service.variables_editable,
                    egui::Button::new(t("Variable hinzufügen", "Add variable")),
                )
                .clicked()
            {
                self.start_form(
                    variable_form(
                        None,
                        Variable {
                            key: String::new(),
                            value: Some(String::new()),
                        },
                    ),
                    ui.ctx(),
                );
            }
        });
    }

    fn docker_hub_section(
        &mut self,
        ui: &mut Ui,
        image: &str,
        reference: &ImageReference,
        service: &Service,
    ) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        if self.docker_hub_image.as_deref() != Some(image) {
            self.docker_hub_image = Some(image.to_owned());
            self.docker_hub_info = None;
            let (sender, receiver) = mpsc::channel();
            self.docker_hub_rx = Some(receiver);
            let reference = reference.clone();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let _ = sender.send(docker_hub::fetch(&reference));
                ctx.request_repaint();
            });
        }
        if let Some(receiver) = &self.docker_hub_rx
            && let Ok(result) = receiver.try_recv()
        {
            self.docker_hub_info = Some(result);
            self.docker_hub_rx = None;
        }
        let parameters = self
            .docker_hub_info
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map(|info| docker_hub::documented_environment_parameters(&info.full_description))
            .unwrap_or_default();
        let documentation_available = self
            .docker_hub_info
            .as_ref()
            .is_some_and(|result| result.is_ok());
        let mut configure_parameter = None;
        section(ui, t("Docker Hub", "Docker Hub"), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.hyperlink_to(
                    t("Auf Docker Hub öffnen", "Open on Docker Hub"),
                    reference.url(),
                );
                if let Some(tag) = &reference.reference {
                    ui.label(format!("{} {tag}", t("Tag:", "Tag:")));
                }
            });
            match &self.docker_hub_info {
                None => {
                    ui.label(
                        RichText::new(t(
                            "Informationen werden geladen …",
                            "Loading image information …",
                        ))
                        .color(MUTED),
                    );
                }
                Some(Err(error)) if error == "not_found" => {
                    ui.label(
                        RichText::new(t(
                            "Dieses Image wurde auf Docker Hub nicht gefunden oder ist privat.",
                            "This image was not found on Docker Hub or is private.",
                        ))
                        .color(MUTED),
                    );
                }
                Some(Err(_)) => {
                    ui.label(
                        RichText::new(t(
                            "Docker Hub ist nicht erreichbar. Prüfe deine Internetverbindung.",
                            "Docker Hub is unavailable. Check your internet connection.",
                        ))
                        .color(MUTED),
                    );
                }
                Some(Ok(info)) => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{}: {}", t("Pulls", "Pulls"), info.pull_count));
                        ui.separator();
                        ui.label(format!("{}: {}", t("Sterne", "Stars"), info.star_count));
                        if !info.last_updated.is_empty() {
                            ui.separator();
                            ui.label(format!(
                                "{}: {}",
                                t("Aktualisiert", "Updated"),
                                info.last_updated
                            ));
                        }
                    });
                    let description = if info.full_description.trim().is_empty() {
                        &info.description
                    } else {
                        &info.full_description
                    };
                    if !description.trim().is_empty() {
                        egui::ScrollArea::vertical()
                            .max_height(180.0)
                            .show(ui, |ui| ui.label(description));
                    }
                }
            }
        });
        section(
            ui,
            t(
                "Dokumentierte Image-Parameter",
                "Documented image parameters",
            ),
            |ui| {
                ui.label(
                    RichText::new(t(
                        "Pflicht/optional wird nur angezeigt, wenn die Docker-Hub-Dokumentation es ausdrücklich angibt.",
                        "Required/optional is shown only when Docker Hub documentation states it explicitly.",
                    ))
                    .color(MUTED),
                );
                if !documentation_available {
                    ui.label(
                        RichText::new(t(
                            "Die Image-Dokumentation ist noch nicht verfügbar.",
                            "Image documentation is not available yet.",
                        ))
                        .color(MUTED),
                    );
                } else if parameters.is_empty() {
                    ui.label(RichText::new(t(
                        "In der Image-Beschreibung wurde keine Umgebungsvariablen-Tabelle gefunden. Siehe die Image-Dokumentation oben.",
                        "No environment-variable table was found in the image description. See the image documentation above.",
                    )).color(MUTED));
                }
                for parameter in &parameters {
                    ui.push_id(&parameter.name, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&parameter.name).monospace().strong());
                            let requirement = match parameter.requirement {
                                ParameterRequirement::Required => t("Pflicht", "Required"),
                                ParameterRequirement::Optional => t("Optional", "Optional"),
                                ParameterRequirement::Unspecified => {
                                    t("Nicht angegeben", "Not specified")
                                }
                            };
                            ui.label(RichText::new(requirement).color(MUTED));
                            if let Some(default) = &parameter.default_value {
                                ui.label(format!("{} {default}", t("Standard:", "Default:")));
                            }
                        });
                        if !parameter.description.is_empty() {
                            ui.label(RichText::new(&parameter.description).color(MUTED));
                        }
                        let already_set = service
                            .variables
                            .iter()
                            .any(|variable| variable.key == parameter.name);
                        if already_set {
                            ui.label(
                                RichText::new(t(
                                    "Bereits im Service gesetzt",
                                    "Already set in service",
                                ))
                                .small()
                                .color(ACCENT),
                            );
                        } else if service.variables_editable
                            && ui
                                .small_button(t(
                                    "Für Service konfigurieren",
                                    "Configure for service",
                                ))
                                .clicked()
                        {
                            configure_parameter = Some(parameter.clone());
                        }
                    });
                    ui.separator();
                }
            },
        );
        if let Some(parameter) = configure_parameter {
            self.start_form(
                variable_form(
                    None,
                    Variable {
                        key: parameter.name,
                        value: Some(parameter.default_value.unwrap_or_default()),
                    },
                ),
                ui.ctx(),
            );
        }
    }

    fn env_links(&mut self, ui: &mut Ui, names: Vec<&str>) {
        if names.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for name in names {
                if ui
                    .link(RichText::new(name).color(ACCENT).underline())
                    .on_hover_text(self.language.text("In .env öffnen", "Open in .env"))
                    .clicked()
                {
                    self.select_env(name);
                    ui.ctx().request_repaint();
                }
            }
        });
    }

    fn select_env(&mut self, key: &str) {
        if let Some(editor) = &mut self.env_editor
            && editor.key == key
        {
            editor.jump = true;
            return;
        }
        let original = self
            .document
            .as_ref()
            .and_then(|document| document.environment.value(key))
            .map(|value| value.text.clone());
        self.env_editor = Some(EnvEditor {
            key: key.into(),
            value: original.clone().unwrap_or_default(),
            original,
            error: None,
            jump: true,
        });
    }

    fn environment_sidebar(&mut self, ctx: &egui::Context) {
        let Some(document) = &mut self.document else {
            return;
        };
        let t = |de, en| self.language.text(de, en);
        let mut selection = None;
        egui::SidePanel::right("dotenv")
            .default_width(288.0)
            .min_width(190.0)
            .max_width((ctx.available_rect().width() * 0.47).max(190.0))
            .frame(
                Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::same(14)),
            )
            .show(ctx, |ui| {
                ui.heading(".env");
                ui.label(
                    RichText::new(t(
                        "Vorschau aus .env und Standardwerten",
                        "Preview from .env and defaults",
                    ))
                    .color(MUTED),
                );
                if ui.small_button(t("Neu laden", "Reload")).clicked() {
                    document.reload_environment();
                    if let Some(editor) = &mut self.env_editor {
                        let edited = editor.original.as_deref().unwrap_or_default() != editor.value;
                        editor.original = document
                            .environment
                            .value(&editor.key)
                            .map(|value| value.text.clone());
                        if !edited {
                            editor.value = editor.original.clone().unwrap_or_default();
                        }
                        editor.error = None;
                    }
                    self.show_env_values = false;
                }
                match &document.environment {
                    Environment::Missing => {
                        ui.label(
                            RichText::new(t("Keine .env im Ordner", "No .env in this folder"))
                                .color(MUTED),
                        );
                    }
                    Environment::Invalid(line) => {
                        ui.colored_label(
                            ERROR,
                            format!(
                                "{} {line}",
                                t("Ungültiger Eintrag in Zeile", "Invalid entry on line")
                            ),
                        );
                    }
                    Environment::Unreadable => {
                        ui.colored_label(
                            ERROR,
                            t(
                                "Die .env-Datei konnte nicht gelesen werden.",
                                "Could not read the .env file.",
                            ),
                        );
                    }
                    Environment::Loaded(_) => {}
                }
                let language = self.language;
                let writable = matches!(
                    document.environment,
                    Environment::Missing | Environment::Loaded(_)
                );
                let mut editor = self.env_editor.as_mut();
                let mut save = false;
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt((self.document_view, "dotenv rows"))
                    .show(ui, |ui| {
                        if let Some(editor) = editor.as_deref_mut()
                            && document.environment.value(&editor.key).is_none()
                        {
                            save = env_value_editor(ui, editor, writable, language);
                        }
                        if let Environment::Loaded(file) = &document.environment {
                            for (key, value) in &file.values {
                                if let Some(editor) = editor.as_deref_mut()
                                    && editor.key == *key
                                {
                                    save = env_value_editor(ui, editor, writable, language);
                                } else if env_list_row(ui, key, value, language) {
                                    selection = Some(key.clone());
                                }
                            }
                        }
                    });
                if save && let Some(editor) = editor {
                    editor.error = document.save_env_value(&editor.key, &editor.value).err();
                    if editor.error.is_none() {
                        editor.jump = editor.original.is_none();
                        editor.original = Some(editor.value.clone());
                    }
                }
            });
        if let Some(key) = selection {
            self.select_env(&key);
        }
    }

    fn history_sidebar(&mut self, ctx: &egui::Context) {
        let Some(document) = &self.document else {
            return;
        };
        let language = self.language;
        let t = |de, en| language.text(de, en);
        let mut restore_target = None;
        egui::SidePanel::right("history")
            .default_width(220.0)
            .min_width(150.0)
            .max_width((ctx.available_rect().width() * 0.25).max(150.0))
            .frame(Frame::new().fill(SURFACE).inner_margin(egui::Margin::same(14)))
            .show(ctx, |ui| {
                ui.heading(t("Historie", "History"));
                if let Some(notice) = &document.history.notice {
                    ui.colored_label(ERROR, match notice {
                        Notice::Unreadable => t("Die gespeicherte Historie konnte nicht gelesen werden.", "Could not read the saved history."),
                        Notice::WriteFailed => t("Die Historie konnte nicht gespeichert werden.", "Could not save the history."),
                        Notice::Changed => t("Die Historie wurde extern geändert. Öffne die Compose-Datei erneut.", "The history changed externally. Reopen the Compose file."),
                    });
                }
                ui.separator();
                egui::ScrollArea::vertical().id_salt((self.document_view, "history entries"))
                    .show(ui, |ui| {
                        let saved_count = document.history.active_count();
                        for (index, draft) in document.drafts.iter().enumerate().rev() {
                            let current = index + 1 == document.drafts.len();
                            if history_entry(ui, saved_count + index + 1, &draft.change, t("Ungespeichert", "Unsaved"), current, language) {
                                restore_target = Some(Target::Draft(index));
                            }
                        }
                        for (index, version) in document.history.versions.iter().enumerate().rev() {
                            let current = index + 1 == saved_count && document.drafts.is_empty();
                            if history_entry(ui, index + 1, &version.change, &version.time(language), current, language) {
                                restore_target = Some(Target::Saved(index));
                            }
                        }
                    });
            });
        if let Some(target) = restore_target {
            self.restore_version(target, ctx);
        }
    }

    fn restore_version(&mut self, target: Target, ctx: &egui::Context) {
        let Some(document) = &mut self.document else {
            return;
        };
        let service = self
            .selected
            .and_then(|index| document.services.get(index))
            .map(|service| service.name.clone());
        match document.restore(target) {
            Ok(focus) => {
                self.error = None;
                self.form = None;
                self.form_error = None;
                self.selected = service.and_then(|name| {
                    document
                        .services
                        .iter()
                        .position(|service| service.name == name)
                });
                if matches!(target, Target::Saved(_)) {
                    let previous = self.env_editor.take().map(|editor| editor.key);
                    let jump = focus.is_some();
                    if let Some(key) = focus.or(previous) {
                        self.select_env(&key);
                        if let Some(editor) = &mut self.env_editor {
                            editor.jump = jump;
                        }
                    }
                }
                ctx.request_repaint();
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn start_form(&mut self, form: Form, ctx: &egui::Context) {
        if let Form::Image { source, .. } = &form {
            self.request_docker_tags(source, ctx, false);
        }
        self.form = Some(form);
        self.form_error = None;
    }

    fn request_docker_tags(&mut self, image: &str, ctx: &egui::Context, force: bool) {
        let Some(reference) = ImageReference::parse(image) else {
            self.docker_tags_key = None;
            self.docker_tags = None;
            self.docker_tags_rx = None;
            return;
        };
        let key = format!("{}/{}", reference.namespace, reference.repository);
        if !force
            && self.docker_tags_key.as_deref() == Some(&key)
            && (self.docker_tags_rx.is_some() || self.docker_tags.is_some())
        {
            return;
        }
        self.docker_tags_key = Some(key);
        self.docker_tags = None;
        let (sender, receiver) = mpsc::channel();
        self.docker_tags_rx = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(docker_hub::fetch_tags(&reference));
            ctx.request_repaint();
        });
    }

    fn form_dialog(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.docker_tags_rx
            && let Ok(result) = receiver.try_recv()
        {
            self.docker_tags = Some(result);
            self.docker_tags_rx = None;
        }
        if let Some(Form::Image {
            repository, tag, ..
        }) = self.form.as_mut()
        {
            normalize_image_form(repository, tag);
        }
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let language = self.language;
        let t = |de, en| language.text(de, en);
        let tags_key = self.docker_tags_key.clone();
        let tags = self.docker_tags.clone();
        let loading_tags = self.docker_tags_rx.is_some();
        let mut apply: Option<FormSubmission> = None;
        let mut cancel = false;
        let mut reload_tags = None;
        let response = egui::Modal::new(Id::new("edit")).show(ctx, |ui| {
            ui.set_width(390.0);
            egui::ScrollArea::vertical().max_height((ctx.screen_rect().height() - 100.0).max(180.0))
                .show(ui, |ui| form_fields(ui, form, language, self.document.as_ref(), self.selected));
            if let Form::Image {
                repository,
                tag,
                ..
            } = form
            {
                ui.add_space(6.0);
                if let Some(reference) = form_image_reference(repository, tag) {
                    let key = format!("{}/{}", reference.namespace, reference.repository);
                    if tags_key.as_deref() != Some(&key) {
                        ui.label(
                            RichText::new(t(
                                "Die Tag-Liste gehört zu einem anderen Image.",
                                "The tag list belongs to a different image.",
                            ))
                            .color(MUTED),
                        );
                        if ui
                            .button(t("Tags für dieses Image laden", "Load tags for this image"))
                            .clicked()
                        {
                            reload_tags = Some(form_image_value(repository, tag));
                        }
                    } else {
                        match &tags {
                            Some(Ok(tags)) => {
                                if tags.is_empty() {
                                    ui.label(t("Keine Tags gefunden.", "No tags found."));
                                } else {
                                    ui.label(t(
                                        "Docker-Hub-Tag auswählen",
                                        "Select a Docker Hub tag",
                                    ));
                                    image_tag_picker(ui, tag, tags, language);
                                    if ui
                                        .small_button(t("Tags aktualisieren", "Refresh tags"))
                                        .clicked()
                                    {
                                        reload_tags = Some(form_image_value(repository, tag));
                                    }
                                }
                            }
                            Some(Err(error)) if error == "not_found" => {
                                ui.label(
                                    RichText::new(t(
                                        "Repository auf Docker Hub nicht gefunden oder privat.",
                                        "Repository not found on Docker Hub or private.",
                                    ))
                                    .color(MUTED),
                                );
                            }
                            Some(Err(error)) => {
                                let message = match error.as_str() {
                                    "network" => t("Keine Verbindung zu Docker Hub. Prüfe die Internetverbindung.", "Could not connect to Docker Hub. Check the internet connection."),
                                    "access" => t("Docker Hub verweigert den Zugriff. Das Repository ist möglicherweise privat.", "Docker Hub denied access. The repository may be private."),
                                    "rate_limited" => t("Docker Hub hat die Abfrage begrenzt. Bitte später erneut versuchen.", "Docker Hub rate-limited this request. Try again later."),
                                    "invalid_response" => t("Docker Hub hat eine unerwartete Antwort gesendet.", "Docker Hub returned an unexpected response."),
                                    _ => t("Docker Hub ist gerade nicht verfügbar.", "Docker Hub is currently unavailable."),
                                };
                                ui.label(RichText::new(message).color(MUTED));
                                if ui
                                    .small_button(t("Erneut versuchen", "Try again"))
                                    .clicked()
                                {
                                    reload_tags = Some(form_image_value(repository, tag));
                                }
                            }
                            None if loading_tags => {
                                ui.label(
                                    RichText::new(t("Tags werden geladen …", "Loading tags …"))
                                        .color(MUTED),
                                );
                            }
                            None => {
                                if ui.button(t("Tags laden", "Load tags")).clicked() {
                                    reload_tags = Some(form_image_value(repository, tag));
                                }
                            }
                        }
                    }
                } else {
                    ui.label(
                        RichText::new(t(
                            "Für dieses Image können keine Docker-Hub-Tags geladen werden.",
                            "Docker Hub tags cannot be loaded for this image.",
                        ))
                        .color(MUTED),
                    );
                }
            }
            if let Some(error) = &self.form_error {
                ui.colored_label(ERROR, error.message(language));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                apply = primary(ui, t("Übernehmen", "Apply"))
                    .then(|| form_submission(form));
                cancel = ui.button(t("Abbrechen", "Cancel")).clicked();
            });
        });
        if let Some(image) = reload_tags {
            self.request_docker_tags(&image, ctx, true);
        }
        if let Some(submission) = apply {
            let applied = match submission {
                FormSubmission::Edit(edit) => self.apply(edit),
                FormSubmission::Project(edit) => self.apply_project(edit),
                FormSubmission::NewService { name, image } => self.add_service(name, image),
            };
            if applied {
                self.form = None;
            }
        } else if cancel || response.should_close() {
            self.form = None;
        }
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        let Some(choice) = self.settings.as_mut() else {
            return;
        };
        let t = |de, en| self.language.text(de, en);
        let mut save = false;
        let mut cancel = false;
        let response = egui::Modal::new(Id::new("settings")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading(t("Einstellungen", "Settings"));
            ui.add_space(10.0);
            ui.label(t("Sprache", "Language"));
            ui.radio_value(choice, Language::German, "Deutsch");
            ui.radio_value(choice, Language::English, "English");
            if self.settings_failed {
                ui.colored_label(
                    ERROR,
                    t(
                        "Die Spracheinstellung konnte nicht gespeichert werden.",
                        "Could not save the language setting.",
                    ),
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                save = primary(ui, t("Speichern", "Save"));
                cancel = ui.button(t("Abbrechen", "Cancel")).clicked();
            });
        });
        if save {
            if choice.save(self.config_path.as_deref()).is_ok() {
                self.language = *choice;
                self.settings = None;
            } else {
                self.settings_failed = true;
            }
        } else if cancel || response.should_close() {
            self.settings = None;
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            return;
        }
        let lang = self.language;
        let t = |de, en| lang.text(de, en);
        let mut proceed = false;
        let mut cancel = false;
        let mut save = false;
        let response = egui::Modal::new(Id::new("unsaved")).show(ctx, |ui| {
            ui.set_width(390.0);
            ui.heading(t("Ungespeicherte Änderungen", "Unsaved changes"));
            ui.label(t(
                "Möchtest du deine Änderungen speichern?",
                "Do you want to save your changes?",
            ));
            if let Some(error) = &self.error {
                ui.colored_label(ERROR, error.message(lang));
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                save = primary(ui, t("Speichern", "Save"));
                proceed = ui.button(t("Verwerfen", "Discard")).clicked();
                cancel = ui.button(t("Abbrechen", "Cancel")).clicked();
            });
            if matches!(self.error, Some(Error::Conflict))
                && ui.button(t("Speichern unter", "Save As")).clicked()
                && self.apply_form_if_present()
                && self.save(true)
            {
                proceed = true;
            }
        });
        if save {
            proceed = self.apply_form_if_present() && self.save(false);
        }
        if proceed {
            let action = self.pending.take().unwrap();
            self.run(action, ctx);
        } else if cancel || response.should_close() {
            self.pending = None;
        }
    }

    fn apply_form_if_present(&mut self) -> bool {
        let Some(form) = self.form.as_ref() else {
            return true;
        };
        let submission = form_submission(form);
        let applied = match submission {
            FormSubmission::Edit(edit) => self.apply(edit),
            FormSubmission::Project(edit) => self.apply_project(edit),
            FormSubmission::NewService { name, image } => self.add_service(name, image),
        };
        if applied {
            self.form = None;
        } else {
            self.error = self.form_error.take();
        }
        applied
    }

    fn add_service(&mut self, name: String, image: Option<String>) -> bool {
        let Some(document) = &mut self.document else {
            return false;
        };
        match document.add_service(&name, image.as_deref()) {
            Ok(index) => {
                self.selected = Some(index);
                self.form_error = None;
                self.error = None;
                true
            }
            Err(error) => {
                self.form_error = Some(error);
                false
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.close_allowed
            && (self.dirty() || self.form.is_some())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.pending.is_none() {
                self.ask(Action::Quit, ctx);
            }
        }
        if self.pending.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q))
        {
            self.ask(Action::Quit, ctx);
        }
        if self.pending.is_none()
            && self.settings.is_none()
            && self.document.is_some()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::W))
        {
            self.ask(Action::CloseDocument, ctx);
        }
        if self.pending.is_none() && self.settings.is_none() && self.form.is_none() {
            if let Some(path) = ctx.input(|i| {
                i.raw
                    .dropped_files
                    .first()
                    .and_then(|file| file.path.clone())
            }) {
                self.ask(Action::OpenPath(path), ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
                self.ask(Action::Open, ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
                && self.can_save()
            {
                self.save(false);
            }
        }
        self.toolbar(ctx);
        self.history_sidebar(ctx);
        self.environment_sidebar(ctx);
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(22, 21)),
            )
            .show(ctx, |ui| {
                self.project_navigation(ui);
                let active_service = self
                    .document
                    .as_ref()
                    .and_then(|document| document.services.get(self.selected?))
                    .map(|service| service.name.as_str());
                let detail_view = (self.document_view, self.project_tab, active_service);
                egui::ScrollArea::vertical()
                    .id_salt(detail_view)
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.content(ui));
            });
        if self.pending.is_some() {
            self.confirm_dialog(ctx);
        } else if self.settings.is_some() {
            self.settings_dialog(ctx);
        } else if self.form.is_some() {
            self.form_dialog(ctx);
        }
    }
}

fn primary(ui: &mut Ui, label: &str) -> bool {
    ui.add(
        egui::Button::new(RichText::new(label).color(BACKGROUND).strong())
            .fill(ACCENT)
            .stroke(Stroke::NONE),
    )
    .clicked()
}

fn section(ui: &mut Ui, title: &str, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(12)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(title).size(18.0).strong());
            ui.add_space(3.0);
            content(ui);
        });
    ui.add_space(4.0);
}

fn row(ui: &mut Ui, label: &str, editable: bool, lang: Language, mut action: impl FnMut(bool)) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(label).monospace());
        if editable {
            if ui.button(lang.text("Bearbeiten", "Edit")).clicked() {
                action(true);
            }
            if ui.button(lang.text("Entfernen", "Remove")).clicked() {
                action(false);
            }
        }
    });
}

fn port_label(port: &Port, language: Language) -> String {
    let t = |de, en| language.text(de, en);
    match port {
        Port::Short(value) => {
            let (mapping, protocol) = value
                .split_once('/')
                .map_or((value.as_str(), ""), |(mapping, protocol)| {
                    (mapping, protocol)
                });
            let parts: Vec<_> = mapping.rsplitn(3, ':').collect();
            let label = match parts.as_slice() {
                [container] => format!(
                    "{} → {} {container}",
                    t("Host-Port dynamisch", "Host port dynamic"),
                    t("Container-Port", "Container port")
                ),
                [container, host] => format!(
                    "{} {host} → {} {container}",
                    t("Host-Port", "Host port"),
                    t("Container-Port", "Container port")
                ),
                [container, host, ip] => format!(
                    "{} {ip}:{host} → {} {container}",
                    t("Host", "Host"),
                    t("Container-Port", "Container port")
                ),
                _ => value.clone(),
            };
            if protocol.is_empty() {
                label
            } else {
                format!("{label}/{protocol}")
            }
        }
        Port::Long(port) => {
            let host = if port.published.is_empty() {
                t("Host-Port dynamisch", "Host port dynamic").to_owned()
            } else if port.host_ip.is_empty() {
                format!("{} {}", t("Host-Port", "Host port"), port.published)
            } else {
                format!("{} {}:{}", t("Host", "Host"), port.host_ip, port.published)
            };
            let mut label = format!(
                "{host} → {} {}",
                t("Container-Port", "Container port"),
                port.target
            );
            if !port.protocol.is_empty() {
                label.push('/');
                label.push_str(&port.protocol);
            }
            label
        }
        Port::Other(value) => value.clone(),
    }
}

fn restart_policy_label(value: &str, language: Language) -> &str {
    match value {
        "no" => language.text("Nie (no)", "Never (no)"),
        "always" => language.text("Immer (always)", "Always"),
        "on-failure" => language.text("Bei Fehler (unbegrenzt)", "On failure (unlimited)"),
        "on-failure:3" => language.text("Bei Fehler (3 Versuche)", "On failure (3 retries)"),
        "on-failure:5" => language.text("Bei Fehler (5 Versuche)", "On failure (5 retries)"),
        "on-failure:10" => language.text("Bei Fehler (10 Versuche)", "On failure (10 retries)"),
        "unless-stopped" => language.text("Außer manuell gestoppt", "Unless stopped"),
        _ => value,
    }
}

fn history_entry(
    ui: &mut Ui,
    number: usize,
    change: &Change,
    date: &str,
    current: bool,
    language: Language,
) -> bool {
    ui.label(
        RichText::new(format!("#{number} · {date}"))
            .small()
            .color(MUTED),
    );
    ui.label(RichText::new(change.title(language)).strong());
    if let Some(detail) = change.detail(language) {
        ui.label(RichText::new(detail).monospace().size(11.0).color(MUTED));
    }
    let restore = if current {
        ui.label(RichText::new(language.text("Aktueller Stand", "Current version")).color(ACCENT));
        false
    } else {
        ui.link(language.text("Wiederherstellen", "Restore"))
            .clicked()
    };
    ui.separator();
    restore
}

fn env_list_row(ui: &mut Ui, key: &str, value: &Preview, language: Language) -> bool {
    ui.columns(2, |cols| {
        let name = cols[0]
            .add(egui::Label::new(key).truncate().sense(egui::Sense::click()))
            .on_hover_text(key);
        let displayed = display_value(&value.text);
        let mut text = RichText::new(displayed).monospace();
        if value.unresolved {
            text = text.color(ERROR);
        }
        let response = cols[1].add(
            egui::Label::new(text)
                .truncate()
                .sense(egui::Sense::click()),
        );
        if value.unresolved {
            response.clone().on_hover_text(format!(
                "{}\n{displayed}",
                language.text("Nicht aufgelöst", "Unresolved"),
            ));
        }
        name.clicked() || response.clicked()
    })
}

fn env_value_editor(
    ui: &mut Ui,
    editor: &mut EnvEditor,
    writable: bool,
    language: Language,
) -> bool {
    let t = |de, en| language.text(de, en);
    let row = ui.scope(|ui| {
        ui.columns(2, |cols| {
            cols[0]
                .add(egui::Label::new(RichText::new(&editor.key).color(ACCENT)).truncate())
                .on_hover_text(&editor.key);
            if editor.original.is_none() {
                cols[0].label(RichText::new(t("Neu", "New")).small().color(MUTED));
            }
            cols[1].push_id(&editor.key, |ui| {
                let field = ui.add(
                    egui::TextEdit::multiline(&mut editor.value)
                        .desired_rows(1)
                        .desired_width(f32::INFINITY)
                        .hint_text(t("Wert", "Value")),
                );
                if editor.jump && writable {
                    field.request_focus();
                }
            });
        });
        if let Some(error) = &editor.error {
            let message = match error {
                SaveError::Conflict => t(
                    "Die .env wurde extern geändert. Lade sie vor dem Speichern neu.",
                    "The .env changed externally. Reload it before saving.",
                ),
                SaveError::InvalidName => t(
                    "Der Variablenname ist ungültig.",
                    "The variable name is invalid.",
                ),
                _ => t(
                    "Die .env konnte nicht gespeichert werden.",
                    "Could not save .env.",
                ),
            };
            ui.colored_label(ERROR, message);
        }
        ui.add_enabled(
            writable && editor.original.as_deref() != Some(&editor.value),
            egui::Button::new(t("In .env speichern", "Save to .env")),
        )
        .on_hover_text(t(
            "Der eingegebene Wert wird wörtlich gespeichert.",
            "The entered value is saved literally.",
        ))
        .clicked()
    });
    if editor.jump {
        row.response.scroll_to_me(Some(egui::Align::Center));
        editor.jump = false;
    }
    row.inner
}

fn display_value(text: &str) -> &str {
    if text.is_empty() { "\"\"" } else { text }
}

fn preview_line(
    ui: &mut Ui,
    original: Option<&str>,
    preview: &Preview,
    language: Language,
    show_value: bool,
) {
    if preview.unresolved {
        ui.colored_label(
            ERROR,
            language.text("Vorschau: Nicht aufgelöst", "Preview: Unresolved"),
        );
    } else if show_value && original != Some(&preview.text) {
        ui.label(
            RichText::new(format!(
                "{} {}",
                language.text("Vorschau:", "Preview:"),
                display_value(&preview.text)
            ))
            .monospace()
            .color(ACCENT),
        );
    }
}

fn card_preview(
    ui: &mut Ui,
    original: &str,
    environment: &Environment,
    language: Language,
    color: Color32,
) {
    let preview = environment.preview(original);
    let label = if preview.unresolved {
        format!(
            "{} · {}",
            language.text("Nicht aufgelöst", "Unresolved"),
            original
        )
    } else {
        preview.text
    };
    let color = if preview.unresolved { ERROR } else { color };
    ui.add(egui::Label::new(RichText::new(&label).monospace().color(color)).truncate())
        .on_hover_text(&label);
}

fn card_ports(ui: &mut Ui, service: &Service, environment: &Environment, language: Language) {
    if service.ports.is_empty() {
        ui.label(
            RichText::new(language.text("Keine Port-Zuordnungen", "No port mappings"))
                .color(ACCENT),
        );
        return;
    }
    ui.horizontal_wrapped(|ui| {
        let font_id = ui.style().text_styles[&egui::TextStyle::Monospace].clone();
        let mut has_group_on_row = false;
        for port in &service.ports {
            let original = port_label(port, language);
            let preview = environment.preview(&original);
            let label = if preview.unresolved {
                format!(
                    "{} · {}",
                    language.text("Nicht aufgelöst", "Unresolved"),
                    original
                )
            } else {
                preview.text
            };
            let color = if preview.unresolved { ERROR } else { ACCENT };
            let grouped_label = label.replace(' ', "\u{00a0}");
            let label_width = ui.fonts(|fonts| {
                fonts
                    .layout_no_wrap(grouped_label.clone(), font_id.clone(), color)
                    .size()
                    .x
            });
            let group_width = label_width + 14.0;
            if has_group_on_row && ui.available_size_before_wrap().x < group_width {
                ui.end_row();
            }
            Frame::new()
                .fill(BACKGROUND)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(5)
                .inner_margin(egui::Margin::symmetric(6, 3))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(grouped_label).monospace().color(color))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
            has_group_on_row = true;
        }
    });
}

fn service_card(
    ui: &mut Ui,
    service: &Service,
    environment: &Environment,
    language: Language,
    width: f32,
    index: usize,
) -> bool {
    let t = |de, en| language.text(de, en);
    let result = ui.push_id(index, |ui| {
        Frame::new()
            .fill(SURFACE)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(13)
            .inner_margin(14)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width((width - 30.0).max(1.0));
                    ui.set_min_height(128.0);
                    ui.add(
                        egui::Label::new(RichText::new(&service.name).size(23.0).strong())
                            .truncate(),
                    )
                    .on_hover_text(&service.name);
                    let build = service
                        .build_context
                        .as_ref()
                        .map(|context| format!("Build: {context}"));
                    let image = service
                        .image
                        .as_deref()
                        .or(build.as_deref())
                        .unwrap_or(t("Kein Image angegeben", "No image specified"));
                    card_preview(ui, image, environment, language, MUTED);
                    ui.add_space(3.0);
                    card_ports(ui, service, environment, language);
                    ui.add_space(6.0);
                    ui.add_sized(
                        Vec2::new(ui.available_width(), 29.0),
                        egui::Button::new(t("Details öffnen", "Open details")),
                    )
                    .clicked()
                })
                .inner
            })
            .inner
    });
    result.inner
}

fn variable_form(index: Option<usize>, variable: Variable) -> Form {
    Form::Variable {
        index,
        key: variable.key,
        inherit: variable.value.is_none(),
        value: variable.value.unwrap_or_default(),
    }
}

fn form_edit(form: &Form) -> Edit {
    match form {
        Form::Image {
            repository, tag, ..
        } => Edit::Image(Some(form_image_value(repository, tag))),
        Form::Port(index, port) => Edit::Port(*index, port.clone()),
        Form::Variable {
            index,
            key,
            value,
            inherit,
        } => Edit::Variable(
            *index,
            Variable {
                key: key.clone(),
                value: (!inherit).then(|| value.clone()),
            },
        ),
        _ => unreachable!("form uses its dedicated submission"),
    }
}

fn form_submission(form: &Form) -> FormSubmission {
    match form {
        Form::NewService { name, image } => FormSubmission::NewService {
            name: name.trim().to_owned(),
            image: (!image.trim().is_empty()).then(|| image.trim().to_owned()),
        },
        Form::Resource(form) => FormSubmission::Project(form.edit()),
        Form::Parameter(form) => FormSubmission::Project(form.edit()),
        Form::Assignment(form) => FormSubmission::Project(form.edit()),
        Form::SectionYaml { kind, yaml, add } => FormSubmission::Project(ProjectEdit::Parameter {
            key: kind.key().into(),
            add: *add,
            yaml: Some(yaml.clone()),
        }),
        _ => FormSubmission::Edit(form_edit(form)),
    }
}

fn form_fields(
    ui: &mut Ui,
    form: &mut Form,
    language: Language,
    document: Option<&Compose>,
    selected: Option<usize>,
) {
    let t = |de, en| language.text(de, en);
    match form {
        Form::Resource(form) => form.fields(ui, language),
        Form::Parameter(form) => form.fields(ui, language),
        Form::Assignment(form) => form.fields(ui, language, document, selected),
        Form::SectionYaml { kind, yaml, .. } => {
            ui.heading(kind.title(language));
            ui.add(
                egui::TextEdit::multiline(yaml)
                    .code_editor()
                    .desired_rows(10)
                    .desired_width(f32::INFINITY),
            );
        }

        Form::NewService { name, image } => {
            ui.heading(t("Service hinzufügen", "Add service"));
            field(ui, t("Servicename", "Service name"), name);
            field(ui, t("Image (optional)", "Image (optional)"), image);
            ui.label(
                RichText::new(t(
                    "Das Image kann später in den Service-Details geändert werden.",
                    "You can edit the image later in the service details.",
                ))
                .color(MUTED),
            );
        }
        Form::Image { repository, .. } => {
            ui.heading(t("Image bearbeiten", "Edit image"));
            field(ui, "Image", repository);
        }
        Form::Port(_, Port::Short(value)) => {
            ui.heading(t("Port-Zuordnung", "Port mapping"));
            field(
                ui,
                t(
                    "Zuordnung (Host-Port:Container-Port)",
                    "Mapping (host port:container port)",
                ),
                value,
            );
            ui.label(
                RichText::new(t(
                    "8080:80 bedeutet: Host-Port 8080 → Container-Port 80. Bei nur „80“ wählt Docker den Host-Port automatisch.",
                    "8080:80 means host port 8080 → container port 80. With only “80”, Docker assigns the host port automatically.",
                ))
                .color(MUTED),
            );
        }
        Form::Port(_, Port::Long(port)) => {
            ui.heading(t("Port-Zuordnung", "Port mapping"));
            field(
                ui,
                t(
                    "Container-Port (intern im Image)",
                    "Container port (internal to image)",
                ),
                &mut port.target,
            );
            field(
                ui,
                t(
                    "Host-Port (auf dem Rechner veröffentlicht)",
                    "Host port (published on host)",
                ),
                &mut port.published,
            );
            field(
                ui,
                t(
                    "Host-IP (127.0.0.1 = nur lokal)",
                    "Host IP (127.0.0.1 = local only)",
                ),
                &mut port.host_ip,
            );
            ui.label(
                RichText::new(t(
                    "Ohne Host-IP bindet Docker standardmäßig an alle Schnittstellen (0.0.0.0).",
                    "Without a host IP, Docker binds to all interfaces (0.0.0.0) by default.",
                ))
                .color(MUTED),
            );
            field(ui, t("Protokoll", "Protocol"), &mut port.protocol);
        }
        Form::Port(_, Port::Other(_)) => (),
        Form::Variable {
            key,
            value,
            inherit,
            ..
        } => {
            ui.heading(t("Umgebungsvariable", "Environment variable"));
            field(ui, "Name", key);
            ui.checkbox(
                inherit,
                t("Wert aus Umgebung übernehmen", "Use value from environment"),
            );
            ui.add_enabled_ui(!*inherit, |ui| {
                ui.label(t("Wert", "Value"));
                ui.add(
                    egui::TextEdit::multiline(value)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
            });
        }
    }
}

fn image_form(image: String) -> Form {
    let parsed = ImageReference::parse(&image);
    let tag = parsed.as_ref().and_then(|reference| {
        reference
            .reference
            .as_deref()
            .filter(|tag| !tag.starts_with('@'))
            .map(str::to_owned)
            .or_else(|| reference.reference.is_none().then(String::new))
    });
    let repository = if tag.is_some() {
        strip_image_tag(&image)
    } else {
        image.clone()
    };
    Form::Image {
        repository,
        tag,
        source: image,
    }
}

fn normalize_image_form(repository: &mut String, tag: &mut Option<String>) {
    if tag.is_some()
        && let Some(reference) = ImageReference::parse(repository)
        && let Some(value) = reference.reference.filter(|value| !value.starts_with('@'))
    {
        *repository = strip_image_tag(repository);
        *tag = Some(value);
    }
}

fn form_image_value(repository: &str, tag: &Option<String>) -> String {
    match tag.as_deref() {
        Some(tag) if !tag.is_empty() => format!("{repository}:{tag}"),
        _ => repository.to_owned(),
    }
}

fn form_image_reference(repository: &str, tag: &Option<String>) -> Option<ImageReference> {
    ImageReference::parse(&form_image_value(repository, tag))
}

fn strip_image_tag(image: &str) -> String {
    let name = image.split_once('@').map_or(image, |(name, _)| name);
    let after_slash = name.rfind('/').map_or(0, |index| index + 1);
    match name[after_slash..].rfind(':') {
        Some(index) => name[..after_slash + index].to_owned(),
        None => name.to_owned(),
    }
}

fn image_tag_picker(
    ui: &mut Ui,
    tag: &mut Option<String>,
    tags: &[DockerHubTag],
    language: Language,
) {
    let current = tag
        .as_deref()
        .filter(|tag| !tag.is_empty())
        .unwrap_or(language.text("Tag auswählen", "Select a tag"));
    let mut selected = tag.clone().unwrap_or_default();
    egui::ComboBox::from_id_salt("docker-hub-image-tags")
        .selected_text(current)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for tag in tags {
                        let label = tag.last_updated.as_deref().map_or_else(
                            || tag.name.clone(),
                            |updated| format!("{}  ·  {}", tag.name, updated),
                        );
                        ui.selectable_value(&mut selected, tag.name.clone(), label);
                    }
                });
        });
    *tag = Some(selected);
}

fn field(ui: &mut Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
}

#[cfg(test)]
mod image_form_tests {
    use super::*;

    #[test]
    fn separates_and_reassembles_docker_hub_image_tag() {
        let mut form = image_form("docker.io/appwrite/appwrite:2.7.3".into());
        let Form::Image {
            repository, tag, ..
        } = &mut form
        else {
            panic!("expected image form");
        };
        assert_eq!(repository, "docker.io/appwrite/appwrite");
        assert_eq!(tag.as_deref(), Some("2.7.3"));
        *tag = Some("2.7.4".into());
        let Edit::Image(Some(value)) = form_edit(&form) else {
            panic!("expected an image edit");
        };
        assert_eq!(value, "docker.io/appwrite/appwrite:2.7.4");
    }

    #[test]
    fn image_without_explicit_tag_keeps_tag_in_dropdown_state() {
        let form = image_form("nginx".into());
        let Form::Image {
            repository, tag, ..
        } = form
        else {
            panic!("expected image form");
        };
        assert_eq!(repository, "nginx");
        assert_eq!(tag.as_deref(), Some(""));
    }

    #[test]
    fn labels_host_and_container_ports_explicitly() {
        assert_eq!(
            port_label(&Port::Short("8080:80".into()), Language::German),
            "Host-Port 8080 → Container-Port 80"
        );
        assert_eq!(
            port_label(&Port::Short("127.0.0.1:8080:80".into()), Language::English),
            "Host 127.0.0.1:8080 → Container port 80"
        );
    }
}

#[cfg(test)]
mod project_view_tests {
    use super::*;
    use egui::epaint::Shape;

    fn rendered_labels(context: &egui::Context, draw: impl FnMut(&egui::Context)) -> Vec<String> {
        fn visit(shape: Shape, labels: &mut Vec<String>) {
            match shape {
                Shape::Text(text) => labels.push(text.galley.text().to_owned()),
                Shape::Vec(shapes) => shapes.into_iter().for_each(|shape| visit(shape, labels)),
                _ => (),
            }
        }
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 1500.0),
                )),
                ..Default::default()
            },
            draw,
        );
        let mut labels = Vec::new();
        for shape in output.shapes {
            visit(shape.shape, &mut labels);
        }
        labels
    }

    #[test]
    fn renders_project_tabs_and_service_sections_in_both_languages() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(
            &path,
            "services:\n  api:\n    networks: [edge]\n    volumes: [data:/data]\nnetworks:\n  edge: null\nvolumes:\n  data: null\n",
        )
        .unwrap();

        for language in [Language::German, Language::English] {
            let t = |de, en| language.text(de, en);
            let context = egui::Context::default();
            let mut app = App::new(&context, Some(path.clone()));
            app.language = language;
            app.selected = Some(0);
            let labels = rendered_labels(&context, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.project_navigation(ui);
                    app.content(ui);
                });
            });
            for label in [
                "Services",
                t("Netzwerke", "Networks"),
                "Volumes",
                "Labels",
                t("Abhängigkeiten", "Dependencies"),
                t("Weitere Parameter", "Additional parameters"),
                t("Umgebungsvariablen", "Environment variables"),
            ] {
                assert!(labels.iter().any(|text| text == label), "Missing {label}");
            }
            let position = |label| labels.iter().position(|text| text == label).unwrap();
            assert!(
                position(t("Abhängigkeiten", "Dependencies"))
                    < position(t("Umgebungsvariablen", "Environment variables"))
            );

            app.project_tab = ProjectTab::Resources(ResourceKind::Network);
            app.resource_focus = Some("edge".into());
            app.selected = None;
            let labels = rendered_labels(&context, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.content(ui));
            });
            for label in [
                "edge",
                "api",
                t(
                    "Diese Ressource wird von Services verwendet.",
                    "This resource is used by services.",
                ),
            ] {
                assert!(labels.iter().any(|text| text == label), "Missing {label}");
            }
        }
    }
}
