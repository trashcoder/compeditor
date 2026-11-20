mod additional_parameters;
mod resources;
mod service_parameters;
mod workspace;

use additional_parameters::ParameterForm;
use resources::ResourceForm;
use service_parameters::AssignmentForm;
use workspace::{Inspector, ServiceTab};

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
use eframe::egui::{self, Color32, Frame, Id, RichText, Stroke, Ui};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

const BACKGROUND: Color32 = Color32::from_rgb(17, 21, 27);
const SURFACE: Color32 = Color32::from_rgb(25, 31, 39);
const BORDER: Color32 = Color32::from_rgb(51, 62, 74);
const TEXT: Color32 = Color32::from_rgb(234, 240, 245);
const MUTED: Color32 = Color32::from_rgb(165, 180, 194);
const ACCENT: Color32 = Color32::from_rgb(112, 226, 206);
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
    service_tab: ServiceTab,
    service_query: String,
    resource_query: String,
    navigation_open: bool,
    inspector: Option<Inspector>,
    environment_sidebar_enabled: bool,
    edit_menu_target: Option<Id>,
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
        workspace::configure_theme(ctx);
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
            service_tab: ServiceTab::default(),
            service_query: String::new(),
            resource_query: String::new(),
            navigation_open: false,
            inspector: None,
            environment_sidebar_enabled: true,
            edit_menu_target: None,
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
        self.select_first_service();
        self.project_tab = ProjectTab::Services;
        self.service_query.clear();
        self.resource_query.clear();
        self.navigation_open = false;
        self.inspector = None;
        self.environment_sidebar_enabled = true;
        self.edit_menu_target = None;
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
        let new_resource = match &edit {
            ProjectEdit::Resource {
                kind,
                key,
                add: true,
                value: Some(_),
            } if self.project_tab == ProjectTab::Resources(*kind) => Some((*kind, key.clone())),
            _ => None,
        };
        let Some(document) = &mut self.document else {
            return false;
        };
        match document.edit_project(self.selected, edit) {
            Ok(()) => {
                self.error = None;
                if let Some((kind, key)) = new_resource {
                    self.open_resource(kind, key);
                }
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

    fn error_banner(&mut self, ui: &mut Ui) {
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
    }

    fn service_configuration(&mut self, ui: &mut Ui, service: &Service) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
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
            if service.image.is_none()
                && let Some(build) = &service.build_context
            {
                ui.label(
                    RichText::new(format!("Build: {build}"))
                        .monospace()
                        .color(MUTED),
                );
            }
            if let Some((image, document)) = service.image.as_deref().zip(self.document.as_ref()) {
                let preview = document.environment.preview(image);
                if !preview.unresolved || self.show_env_values {
                    preview_line(ui, Some(image), &preview, language);
                }
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
        section(ui, "Ports", |ui| {
            if service.ports.is_empty() {
                ui.label(
                    RichText::new(t("Keine Port-Zuordnungen", "No port mappings")).color(MUTED),
                );
            }
            ui.label(
                RichText::new(t(
                    "Der veröffentlichte Host-Port leitet Anfragen an den internen Container-Port weiter.",
                    "The published host port forwards requests to the internal container port.",
                ))
                .color(MUTED),
            );
            for (index, port) in service.ports.iter().enumerate() {
                ui.push_id(index, |ui| {
                    let label = port_label(port, language);
                    let summary = port.summary();
                    let preview = self
                        .document
                        .as_ref()
                        .map(|document| document.environment.preview(&summary));
                    match row_with_content(ui, service.ports_editable, language, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            self.env_reference_text(ui, "", &label);
                            if let Some(preview) = &preview
                                && if preview.unresolved {
                                    self.show_env_values
                                } else {
                                    preview.text != summary
                                }
                            {
                                ui.add_space(8.0);
                                preview_line(ui, Some(&summary), preview, language);
                            }
                        });
                    }) {
                        Some(RowAction::Edit) => {
                            self.start_form(Form::Port(Some(index), port.clone()), ui.ctx());
                        }
                        Some(RowAction::Remove) => {
                            self.apply(Edit::RemovePort(index));
                        }
                        None => {}
                    }
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
                        egui::Button::new(t("Erweiterte Port-Zuordnung", "Advanced port mapping")),
                    )
                    .clicked()
                {
                    self.start_form(Form::Port(None, Port::Long(LongPort::default())), ui.ctx());
                }
            });
        });
        section(ui, t("Umgebungsvariablen", "Environment variables"), |ui| {
            if service.variables.is_empty() {
                ui.label(
                    RichText::new(t(
                        "Keine Variablen für diesen Service",
                        "No variables for this service",
                    ))
                    .color(MUTED),
                );
            }
            let has_preview = service
                .image
                .as_ref()
                .is_some_and(|image| image.contains('$'))
                || service
                    .ports
                    .iter()
                    .any(|port| port.summary().contains('$'))
                || service
                    .variables
                    .iter()
                    .any(|var| var.value.as_ref().is_none_or(|value| value.contains('$')));
            if has_preview
                && ui
                    .checkbox(
                        &mut self.show_env_values,
                        t("Vorschauwerte anzeigen", "Show preview values"),
                    )
                    .changed()
            {
                ui.ctx().request_repaint();
            }
            for (index, var) in service.variables.iter().enumerate() {
                ui.push_id(index, |ui| {
                    match row_with_content(ui, service.variables_editable, language, |ui| {
                        self.service_variable_content(ui, var);
                    }) {
                        Some(RowAction::Edit) => {
                            self.start_form(variable_form(Some(index), var.clone()), ui.ctx());
                        }
                        Some(RowAction::Remove) => {
                            self.apply(Edit::RemoveVariable(index));
                        }
                        None => {}
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

    fn service_variable_content(&mut self, ui: &mut Ui, variable: &Variable) {
        let language = self.language;
        let original = variable.value.as_deref();
        let Some(document) = &self.document else {
            return;
        };
        let environment = &document.environment;
        let inherited = environment.value(&variable.key);
        let preview = match original {
            Some(value) if value.contains('$') => Some(environment.preview(value)),
            None => Some(inherited.cloned().unwrap_or(Preview {
                unresolved: true,
                ..Preview::default()
            })),
            Some(_) => None,
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let key = RichText::new(&variable.key).monospace();
            if let Some(value) = original {
                self.env_reference_text(ui, &format!("{} = ", variable.key), display_value(value));
            } else {
                self.env_link(ui, key, &variable.key);
                ui.monospace(language.text(" · Aus Umgebung", " · From environment"));
            }
            if self.show_env_values
                && let Some(preview) = preview
                && (preview.unresolved || original != Some(preview.text.as_str()))
            {
                ui.add_space(8.0);
                preview_line(ui, original, &preview, language);
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
                self.env_link(ui, RichText::new(name), name);
            }
        });
    }

    fn env_reference_text(&mut self, ui: &mut Ui, prefix: &str, text: &str) {
        let mut references = environment::reference_occurrences(text).peekable();
        if references.peek().is_none() {
            ui.monospace(format!("{prefix}{text}"));
            return;
        }

        if !prefix.is_empty() {
            ui.monospace(prefix);
        }
        let mut end = 0;
        for range in references {
            ui.monospace(&text[end..range.start]);
            let name = &text[range.clone()];
            self.env_link(ui, RichText::new(name).monospace(), name);
            end = range.end;
        }
        if end < text.len() {
            ui.monospace(&text[end..]);
        }
    }

    fn env_link(&mut self, ui: &mut Ui, label: RichText, key: &str) {
        let resolved = self
            .document
            .as_ref()
            .and_then(|document| document.environment.value(key))
            .is_some_and(|value| !value.unresolved);
        let color = if resolved { ACCENT } else { ERROR };
        let tooltip = if resolved || !self.show_env_values {
            self.language.text("In .env öffnen", "Open in .env")
        } else {
            self.language.text(
                "Variable fehlt oder ist nicht aufgelöst. In .env öffnen.",
                "Variable is missing or unresolved. Open in .env.",
            )
        };
        let galley = egui::WidgetText::from(label.color(color).underline()).into_galley(
            ui,
            Some(egui::TextWrapMode::Wrap),
            ui.available_width(),
            egui::TextStyle::Body,
        );
        if ui.link(galley).on_hover_text(tooltip).clicked() {
            self.select_env(key);
            ui.ctx().request_repaint();
        }
    }

    fn select_env(&mut self, key: &str) {
        self.inspector = Some(Inspector::Environment);
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

    fn environment_content(&mut self, ui: &mut Ui) {
        let Some(document) = &mut self.document else {
            return;
        };
        let t = |de, en| self.language.text(de, en);
        let mut selection = None;
        ui.label(
            RichText::new(t(
                "Dateiweite Werte für Compose-Platzhalter.",
                "File-wide values for Compose placeholders.",
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
        let unused = document.unused_environment_variables();
        if !unused.is_empty() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                t(
                    "Gelb · in dieser Compose-Datei ungenutzt",
                    "Yellow · unused in this Compose file",
                ),
            );
        }
        match &document.environment {
            Environment::Missing => {
                ui.label(
                    RichText::new(t("Keine .env im Ordner", "No .env in this folder")).color(MUTED),
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
            Environment::Loaded(file) if file.values.is_empty() => {
                ui.label(
                    RichText::new(t("Keine Variablen in .env", "No variables in .env"))
                        .color(MUTED),
                );
            }
            Environment::Loaded(_) => {
                ui.columns(2, |columns| {
                    columns[0].label(
                        RichText::new(t("Variable", "Variable"))
                            .small()
                            .color(MUTED),
                    );
                    columns[1].label(RichText::new(t("Wert", "Value")).small().color(MUTED));
                });
            }
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
                    save = env_value_editor(ui, editor, writable, false, language);
                }
                if let Environment::Loaded(file) = &document.environment {
                    for (key, value) in &file.values {
                        if let Some(editor) = editor.as_deref_mut()
                            && editor.key == *key
                        {
                            save = env_value_editor(
                                ui,
                                editor,
                                writable,
                                unused.contains(key.as_str()),
                                language,
                            );
                        } else if env_list_row(
                            ui,
                            key,
                            value,
                            unused.contains(key.as_str()),
                            self.show_env_values,
                            language,
                        ) {
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
        if let Some(key) = selection {
            self.select_env(&key);
            ui.ctx().request_repaint();
        }
    }

    fn history_content(&mut self, ui: &mut Ui) {
        let Some(document) = &self.document else {
            return;
        };
        let language = self.language;
        let t = |de, en| language.text(de, en);
        let mut restore_target = None;
        if let Some(notice) = &document.history.notice {
            ui.colored_label(
                ERROR,
                match notice {
                    Notice::Unreadable => t(
                        "Die gespeicherte Historie konnte nicht gelesen werden.",
                        "Could not read the saved history.",
                    ),
                    Notice::WriteFailed => t(
                        "Die Historie konnte nicht gespeichert werden.",
                        "Could not save the history.",
                    ),
                    Notice::Changed => t(
                        "Die Historie wurde extern geändert. Öffne die Compose-Datei erneut.",
                        "The history changed externally. Reopen the Compose file.",
                    ),
                },
            );
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt((self.document_view, "history entries"))
            .show(ui, |ui| {
                let saved_count = document.history.active_count();
                for (index, draft) in document.drafts.iter().enumerate().rev() {
                    let current = index + 1 == document.drafts.len();
                    if history_entry(
                        ui,
                        saved_count + index + 1,
                        &draft.change,
                        t("Ungespeichert", "Unsaved"),
                        current,
                        language,
                    ) {
                        restore_target = Some(Target::Draft(index));
                    }
                }
                for (index, version) in document.history.versions.iter().enumerate().rev() {
                    let current = index + 1 == saved_count && document.drafts.is_empty();
                    if history_entry(
                        ui,
                        index + 1,
                        &version.change,
                        &version.time(language),
                        current,
                        language,
                    ) {
                        restore_target = Some(Target::Saved(index));
                    }
                }
            });
        if let Some(target) = restore_target {
            self.restore_version(target, ui.ctx());
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
                if self.selected.is_none() {
                    self.select_first_service();
                }
                if matches!(target, Target::Saved(_)) {
                    let previous_inspector = self.inspector;
                    let previous = self.env_editor.take().map(|editor| editor.key);
                    let jump = focus.is_some();
                    if let Some(key) = focus.or(previous) {
                        self.select_env(&key);
                        if let Some(editor) = &mut self.env_editor {
                            editor.jump = jump;
                        }
                        if !jump {
                            self.inspector = previous_inspector;
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
            ui.set_width(420.0);
            egui::ScrollArea::vertical()
                .max_height((ctx.screen_rect().height() - 122.0).max(120.0))
                .show(ui, |ui| {
                    form_fields(ui, form, language, self.document.as_ref(), self.selected);
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
                });
            if let Some(error) = &self.form_error {
                ui.colored_label(ERROR, error.message(language));
            }
            ui.add_space(6.0);
            ui.separator();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                apply = primary(ui, t("Übernehmen", "Apply")).then(|| form_submission(form));
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
                self.service_query.clear();
                self.select_service(index);
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
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
                self.ask(Action::NewCompose, ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
                self.ask(Action::Open, ctx);
            }
            if ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::S,
                )
            }) {
                self.save(true);
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
                && self.can_save()
            {
                self.save(false);
            }
        }
        self.workspace(ctx);
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
    workspace::primary_action(ui, label)
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

enum RowAction {
    Edit,
    Remove,
}

fn row(ui: &mut Ui, label: &str, editable: bool, lang: Language, mut action: impl FnMut(bool)) {
    if let Some(selected) = row_with_content(ui, editable, lang, |ui| {
        ui.add(egui::Label::new(RichText::new(label).monospace()).wrap())
            .on_hover_text(label);
    }) {
        action(matches!(selected, RowAction::Edit));
    }
}

fn row_with_content(
    ui: &mut Ui,
    editable: bool,
    lang: Language,
    content: impl FnOnce(&mut Ui),
) -> Option<RowAction> {
    let edit = lang.text("Bearbeiten", "Edit");
    let remove = lang.text("Entfernen", "Remove");
    ui.horizontal(|ui| {
        let reserve = if editable {
            workspace::button_row_width(ui, &[edit, remove]) + ui.spacing().item_spacing.x
        } else {
            0.0
        };
        ui.allocate_ui_with_layout(
            egui::vec2((ui.available_width() - reserve).max(0.0), 29.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(ui.available_width());
                content(ui);
            },
        );
        if editable {
            let edit = ui.button(edit).clicked();
            let remove = ui.button(remove).clicked();
            if edit {
                return Some(RowAction::Edit);
            }
            if remove {
                return Some(RowAction::Remove);
            }
        }
        None
    })
    .inner
}

fn port_label(port: &Port, language: Language) -> String {
    let t = |de, en| language.text(de, en);
    match port {
        Port::Short(value) => {
            let Some((colons, protocol_start)) = short_port_delimiters(value) else {
                return value.clone();
            };
            let mapping = &value[..protocol_start.unwrap_or(value.len())];
            let protocol = protocol_start.map_or("", |start| &value[start + 1..]);
            let container_label = t("Container-Port", "Container port");
            let label = if let Some(host_end) = colons[1] {
                let host_label = if colons[0].is_none() {
                    t("Host-Port", "Host port")
                } else {
                    "Host"
                };
                format!(
                    "{host_label} {} → {container_label} {}",
                    &mapping[..host_end],
                    &mapping[host_end + 1..],
                )
            } else {
                format!(
                    "{} → {container_label} {mapping}",
                    t("Host-Port dynamisch", "Host port dynamic"),
                )
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

fn short_port_delimiters(value: &str) -> Option<([Option<usize>; 2], Option<usize>)> {
    let mut colons = [None; 2];
    let mut remaining = value;
    while let Some(index) = remaining.find([':', '/', '$']) {
        let offset = value.len() - remaining.len() + index;
        remaining = &remaining[index + 1..];
        match value.as_bytes()[offset] {
            b':' => colons = [colons[1], Some(offset)],
            b'/' => return Some((colons, Some(offset))),
            b'$' => {
                if let Some(rest) = remaining.strip_prefix('$') {
                    remaining = rest;
                } else if let Some(expression) = remaining.strip_prefix('{') {
                    let end = environment::closing_brace(expression)?;
                    remaining = &expression[end + 1..];
                }
            }
            _ => unreachable!(),
        }
    }
    Some((colons, None))
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

fn env_variable_name(ui: &mut Ui, key: &str, unused: bool, language: Language) -> egui::Response {
    let mut label = RichText::new(key).monospace();
    if unused {
        label = label.color(ui.visuals().warn_fg_color);
    }
    let response = ui.add(egui::Button::new(label).wrap());
    if unused {
        let explanation = language.text(
            "In dieser Compose-Datei ungenutzt",
            "Unused in this Compose file",
        );
        response.on_hover_text(format!("{key}\n{explanation}"))
    } else {
        response.on_hover_text(key)
    }
}

fn env_list_row(
    ui: &mut Ui,
    key: &str,
    value: &Preview,
    unused: bool,
    show_preview: bool,
    language: Language,
) -> bool {
    ui.columns(2, |cols| {
        let name = env_variable_name(&mut cols[0], key, unused, language);
        let displayed = display_value(&value.text);
        let mut text = RichText::new(displayed).monospace();
        if value.unresolved {
            text = text.color(ERROR);
        }
        let response = cols[1].add(egui::Button::new(text).truncate());
        let tooltip = if value.unresolved && show_preview {
            format!(
                "{}\n{displayed}",
                language.text("Nicht aufgelöst", "Unresolved")
            )
        } else {
            displayed.to_owned()
        };
        name.clicked() || response.on_hover_text(tooltip).clicked()
    })
}

fn env_value_editor(
    ui: &mut Ui,
    editor: &mut EnvEditor,
    writable: bool,
    unused: bool,
    language: Language,
) -> bool {
    let t = |de, en| language.text(de, en);
    let (name, field) = ui.columns(2, |cols| {
        let color = if unused {
            cols[0].visuals().warn_fg_color
        } else {
            ACCENT
        };
        let name = cols[0]
            .add(
                egui::Button::new(RichText::new(&editor.key).monospace().strong().color(color))
                    .wrap()
                    .selected(true)
                    .corner_radius(6),
            )
            .on_hover_text(&editor.key);
        if editor.original.is_none() {
            cols[0].label(
                RichText::new(t("Neue .env-Variable", "New .env variable"))
                    .small()
                    .color(MUTED),
            );
        }
        let min_height = cols[1].spacing().interact_size.y;
        let field = cols[1]
            .add_enabled(
                writable,
                egui::TextEdit::multiline(&mut editor.value)
                    .id_salt(&editor.key)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(1)
                    .desired_width(f32::INFINITY)
                    .min_size(egui::vec2(0.0, min_height))
                    .margin(egui::Margin::symmetric(10, 5))
                    .hint_text(t("Wert", "Value")),
            )
            .labelled_by(name.id);
        (name, field)
    });
    if writable && (editor.jump || name.clicked()) {
        field.request_focus();
    }
    if editor.jump {
        field.scroll_to_me(None);
        editor.jump = false;
    }
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
}

fn display_value(text: &str) -> &str {
    if text.is_empty() { "\"\"" } else { text }
}

fn preview_line(ui: &mut Ui, original: Option<&str>, preview: &Preview, language: Language) {
    if preview.unresolved {
        ui.colored_label(
            ERROR,
            language.text("Vorschau: Nicht aufgelöst", "Preview: Unresolved"),
        );
    } else if original != Some(&preview.text) {
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
                    "Mit 8080:80 leitet Docker Host-Port 8080 an Container-Port 80 weiter. Bei nur „80“ wählt Docker den Host-Port automatisch.",
                    "With 8080:80, Docker forwards host port 8080 to container port 80. With only “80”, Docker assigns the host port automatically.",
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
    let label = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(label.id);
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
        assert_eq!(
            port_label(
                &Port::Short("${HOST_PORT:-8080}:${TARGET_PORT:-80}/${PROTOCOL:-tcp}".into()),
                Language::English,
            ),
            "Host port ${HOST_PORT:-8080} → Container port ${TARGET_PORT:-80}/${PROTOCOL:-tcp}"
        );
    }
}

#[cfg(test)]
mod project_view_tests {
    use super::*;
    use egui::epaint::Shape;

    fn workspace_frame(
        app: &mut App,
        context: &egui::Context,
        size: [f32; 2],
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size[0], size[1]),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.workspace(ctx),
        )
    }

    fn text_shapes(output: egui::FullOutput) -> Vec<egui::epaint::TextShape> {
        fn visit(shape: Shape, text_shapes: &mut Vec<egui::epaint::TextShape>) {
            match shape {
                Shape::Text(text) if !text.galley.elided => text_shapes.push(text),
                Shape::Vec(shapes) => shapes
                    .into_iter()
                    .for_each(|shape| visit(shape, text_shapes)),
                _ => (),
            }
        }
        let mut text_shapes = Vec::new();
        for shape in output.shapes {
            visit(shape.shape, &mut text_shapes);
        }
        text_shapes
    }

    fn rendered_labels(app: &mut App, context: &egui::Context, size: [f32; 2]) -> Vec<String> {
        workspace_frame(app, context, size, vec![]);
        text_shapes(workspace_frame(app, context, size, vec![]))
            .into_iter()
            .map(|shape| shape.galley.text().to_owned())
            .collect()
    }

    fn mouse_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn text_position(output: egui::FullOutput, label: &str) -> egui::Pos2 {
        let item = text_shapes(output)
            .into_iter()
            .find(|item| item.galley.text() == label)
            .unwrap_or_else(|| panic!("Missing {label}"));
        item.visual_bounding_rect().center()
    }

    fn click(
        app: &mut App,
        frame: &impl Fn(&mut App, Vec<egui::Event>) -> egui::FullOutput,
        pos: egui::Pos2,
    ) -> egui::FullOutput {
        frame(
            app,
            vec![egui::Event::PointerMoved(pos), mouse_button(pos, true)],
        );
        frame(app, vec![mouse_button(pos, false)])
    }

    fn assert_labels(labels: &[String], expected: &[&str]) {
        for label in expected {
            assert!(labels.iter().any(|text| text == label), "Missing {label}");
        }
    }

    #[test]
    fn file_menu_is_leftmost_and_saves_the_compose_document() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(&path, "services:\n  app: {}\n").unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, Some(path.clone()));
        app.language = Language::English;
        assert!(app.apply(Edit::Image(Some("nginx:alpine".into()))));
        let size = [1080.0, 720.0];
        let frame = |app: &mut App, events| workspace_frame(app, &context, size, events);

        frame(&mut app, vec![]);
        let toolbar = frame(&mut app, vec![]);
        let file = text_position(toolbar.clone(), "File");
        assert!(file.x < text_position(toolbar.clone(), "Edit").x);
        assert!(file.x < text_position(toolbar.clone(), crate::APP_NAME).x);
        assert!(
            !text_shapes(toolbar)
                .iter()
                .any(|item| item.galley.text() == "Save")
        );

        click(&mut app, &frame, file);
        let save = text_position(frame(&mut app, vec![]), "Save");
        click(&mut app, &frame, save);
        assert!(!app.dirty());
        let saved = Compose::open(&path).unwrap();
        assert_eq!(saved.services[0].image.as_deref(), Some("nginx:alpine"));
    }

    #[test]
    fn edit_menu_targets_the_focused_environment_cell() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(&path, "services:\n  app: {}\n").unwrap();
        std::fs::write(directory.path().join(".env"), "APP_MODE=ready\n").unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, Some(path));
        app.language = Language::English;
        app.select_env("APP_MODE");
        let size = [1080.0, 720.0];
        let frame = |app: &mut App, events| workspace_frame(app, &context, size, events);
        let command = |app: &mut App, label| {
            let edit = text_position(frame(app, vec![]), "Edit");
            click(app, &frame, edit);
            let item = text_position(frame(app, vec![]), label);
            click(app, &frame, item)
        };
        let value = |app: &App| app.env_editor.as_ref().unwrap().value.clone();

        frame(&mut app, vec![]);
        command(&mut app, "Select all");
        let copied = command(&mut app, "Copy");
        assert!(
            copied.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::RequestCopy))
        );
        let clipboard = frame(&mut app, vec![egui::Event::Copy]);
        assert!(
            clipboard.platform_output.commands.iter().any(|cmd| {
                matches!(cmd, egui::OutputCommand::CopyText(text) if text == "ready")
            })
        );
        let cut = command(&mut app, "Cut");
        assert!(
            cut.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::RequestCut))
        );
        frame(&mut app, vec![egui::Event::Cut]);
        assert_eq!(value(&app), "");

        let pasted = command(&mut app, "Paste");
        assert!(
            pasted.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::RequestPaste))
        );
        frame(&mut app, vec![egui::Event::Paste("changed".into())]);
        assert_eq!(value(&app), "changed");
        command(&mut app, "Undo");
        assert_eq!(value(&app), "ready");
        command(&mut app, "Redo");
        assert_eq!(value(&app), "changed");

        command(&mut app, "Select all");
        command(&mut app, "Delete");
        assert_eq!(value(&app), "");
    }

    fn assert_text_color(shapes: &[egui::epaint::TextShape], name: &str, color: Color32) {
        let mut matches = shapes.iter().filter(|item| item.galley.text() == name);
        let text = matches.next().unwrap_or_else(|| panic!("Missing {name}"));
        assert!(matches.next().is_none(), "Repeated reference {name}");
        assert!(
            text.galley
                .job
                .sections
                .iter()
                .all(|section| section.format.color == color),
            "Unexpected color for {name}"
        );
    }

    #[test]
    fn unresolved_preview_notices_follow_the_preview_checkbox() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(
            &path,
            "services:\n  app:\n    image: 'nginx:$TAG'\n    ports: ['$PORT:80']\n    environment: [MODE]\n",
        ).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, Some(path));
        app.language = Language::English;
        let frame = |app: &mut App, events| workspace_frame(app, &context, [1440.0, 900.0], events);
        let notices = |output| {
            text_shapes(output)
                .iter()
                .filter(|shape| shape.galley.text() == "Preview: Unresolved")
                .count()
        };

        app.close_inspector(Inspector::Environment, true);
        frame(&mut app, vec![]);
        let initial = frame(&mut app, vec![]);
        let checkbox = text_position(initial.clone(), "Show preview values");
        assert_eq!(notices(initial.clone()), 0);
        let shapes = text_shapes(initial);
        for name in ["TAG", "PORT", "MODE"] {
            assert_text_color(&shapes, name, ERROR);
        }

        click(&mut app, &frame, checkbox);
        assert!(app.show_env_values);
        let enabled = frame(&mut app, vec![]);
        assert_eq!(notices(enabled.clone()), 3);
        click(
            &mut app,
            &frame,
            text_position(enabled, "Show preview values"),
        );
        assert!(!app.show_env_values);
        assert_eq!(notices(frame(&mut app, vec![])), 0);
    }

    #[test]
    fn service_ports_link_resolved_and_missing_env_references_in_the_mapping() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(
            &path,
            "services:\n  app:\n    ports:\n      - '$HOST_PORT:$MISSING'\n      - target: 443\n        published: '$TLS_PORT'\n",
        ).unwrap();
        std::fs::write(
            directory.path().join(".env"),
            "HOST_PORT=8080\nTLS_PORT=8443\n",
        )
        .unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, Some(path));
        app.language = Language::English;
        let size = [1440.0, 900.0];
        let frame = |app: &mut App, events| workspace_frame(app, &context, size, events);
        app.close_inspector(Inspector::Environment, true);
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let host = text_position(output.clone(), "HOST_PORT");
        let target = text_position(output.clone(), "MISSING");
        assert!((host.y - target.y).abs() < 3.0);
        let shapes = text_shapes(output);
        assert_text_color(&shapes, "HOST_PORT", ACCENT);
        assert_text_color(&shapes, "TLS_PORT", ACCENT);
        assert_text_color(&shapes, "MISSING", ERROR);
        assert!(
            shapes
                .iter()
                .any(|item| item.galley.text().contains("8443"))
        );

        click(&mut app, &frame, target);
        assert_eq!(app.inspector, Some(Inspector::Environment));
        let editor = app.env_editor.as_ref().unwrap();
        assert_eq!(editor.key, "MISSING");
        assert!(editor.original.is_none());
        frame(&mut app, vec![]);
        let edited = frame(&mut app, vec![egui::Event::Text("80".into())]);
        click(&mut app, &frame, text_position(edited, "Save to .env"));
        assert_eq!(
            app.document
                .as_ref()
                .unwrap()
                .environment
                .value("MISSING")
                .unwrap()
                .text,
            "80"
        );
        app.close_inspector(Inspector::Environment, true);
        frame(&mut app, vec![]);
        assert_text_color(&text_shapes(frame(&mut app, vec![])), "MISSING", ACCENT);
        click(&mut app, &frame, host);
        assert_eq!(app.env_editor.as_ref().unwrap().key, "HOST_PORT");
    }

    #[test]
    fn service_variables_link_resolved_and_missing_env_references_in_the_same_row() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(
            &path,
            "services:\n  app:\n    environment:\n      TARGET: '$BASE/$MISSING'\n      TOKEN:\n      BROKEN:\n      INHERITED_MISSING:\n      WITH_DEFAULT: '${MISSING_DEFAULT:-ok}'\n      STATIC: plain\n",
        ).unwrap();
        std::fs::write(
            directory.path().join(".env"),
            "BASE=example\nTOKEN=demo\nBROKEN=$ABSENT\n",
        )
        .unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, Some(path));
        app.language = Language::English;
        let size = [1440.0, 1000.0];
        let frame = |app: &mut App, events| workspace_frame(app, &context, size, events);
        app.close_inspector(Inspector::Environment, true);
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let base = text_position(output.clone(), "BASE");
        let missing = text_position(output.clone(), "MISSING");
        let variable = text_position(output.clone(), "TARGET = ");
        assert!((base.y - variable.y).abs() < 3.0);
        assert!((missing.y - variable.y).abs() < 3.0);
        let shapes = text_shapes(output);
        for name in ["BASE", "TOKEN"] {
            assert_text_color(&shapes, name, ACCENT);
        }
        for name in ["MISSING", "BROKEN", "INHERITED_MISSING", "MISSING_DEFAULT"] {
            assert_text_color(&shapes, name, ERROR);
        }
        assert!(
            shapes
                .iter()
                .any(|shape| shape.galley.text() == "STATIC = plain")
        );

        click(&mut app, &frame, missing);
        assert_eq!(app.inspector, Some(Inspector::Environment));
        assert_eq!(app.env_editor.as_ref().unwrap().key, "MISSING");
        app.close_inspector(Inspector::Environment, true);
        frame(&mut app, vec![]);
        click(&mut app, &frame, base);
        assert_eq!(app.inspector, Some(Inspector::Environment));
        assert_eq!(app.env_editor.as_ref().unwrap().key, "BASE");
    }

    #[test]
    fn clicking_an_environment_value_focuses_and_edits_the_same_cell() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(&path, "services:\n  app: {}\n").unwrap();
        std::fs::write(directory.path().join(".env"), "APP_MODE=ready\n").unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, Some(path.clone()));
        app.language = Language::English;
        let size = [1080.0, 720.0];
        let frame = |app: &mut App, events| workspace_frame(app, &context, size, events);
        frame(&mut app, vec![]);
        let value_position = text_position(frame(&mut app, vec![]), "ready");
        click(&mut app, &frame, value_position);
        frame(&mut app, vec![]);
        let focused = context.memory(|memory| memory.focused()).unwrap();
        assert!(
            context
                .read_response(focused)
                .unwrap()
                .rect
                .contains(value_position)
        );

        let edited = frame(&mut app, vec![egui::Event::Text("!".into())]);
        assert_eq!(app.env_editor.as_ref().unwrap().value, "ready!");
        assert_eq!(
            Environment::load(&path).value("APP_MODE").unwrap().text,
            "ready"
        );

        click(&mut app, &frame, text_position(edited, "Save to .env"));
        assert_eq!(
            Environment::load(&path).value("APP_MODE").unwrap().text,
            "ready!"
        );
    }

    #[test]
    fn shows_unused_dotenv_names_in_yellow_even_when_editing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(&path, "services:\n  app:\n    image: 'nginx:$TAG'\n").unwrap();
        std::fs::write(
            directory.path().join(".env"),
            "TAG=alpine\nUNUSED=example\n",
        )
        .unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, Some(path));
        app.language = Language::English;
        let size = [1080.0, 720.0];
        let frame = |app: &mut App, events| workspace_frame(app, &context, size, events);
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let unused = text_position(output.clone(), "UNUSED");
        let yellow = context.style().visuals.warn_fg_color;
        assert_text_color(&text_shapes(output), "UNUSED", yellow);
        click(&mut app, &frame, unused);
        assert_text_color(&text_shapes(frame(&mut app, vec![])), "UNUSED", yellow);
    }

    #[test]
    fn resizes_both_sidebars_and_wraps_long_variable_names() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        let key = "APPLICATION_AUTHENTICATION_PROVIDER_CONFIGURATION_ENDPOINT_OVERRIDE";
        std::fs::write(&path, "services:\n  app: {}\n").unwrap();
        std::fs::write(directory.path().join(".env"), format!("{key}=enabled\n")).unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, Some(path));
        let size = [1280.0, 720.0];
        assert_labels(
            &rendered_labels(&mut app, &context, size),
            &[key, "enabled"],
        );

        let panel = |id| {
            egui::containers::panel::PanelState::load(&context, Id::new(id))
                .unwrap()
                .rect
        };
        let frame = |app: &mut App, events| workspace_frame(app, &context, size, events);
        let drag = |app: &mut App, start, end| {
            frame(app, vec![egui::Event::PointerMoved(start)]);
            frame(app, vec![mouse_button(start, true)]);
            frame(app, vec![egui::Event::PointerMoved(end)]);
            frame(app, vec![mouse_button(end, false)]);
        };
        let left = panel("project navigation");
        let border = egui::pos2(left.right(), left.center().y);
        drag(&mut app, border, border + egui::vec2(120.0, 0.0));
        assert!(panel("project navigation").width() >= left.width() + 110.0);

        let right = panel("environment inspector");
        let border = egui::pos2(right.left(), right.center().y);
        drag(&mut app, border, border - egui::vec2(180.0, 0.0));
        assert!(panel("environment inspector").width() >= right.width() + 170.0);

        let resized_width = panel("environment inspector").width();
        app.inspector = Some(Inspector::History);
        frame(&mut app, vec![]);
        app.close_inspector(Inspector::History, true);
        frame(&mut app, vec![]);
        assert_eq!(panel("environment inspector").width(), resized_width);

        app.select_env(key);
        assert_labels(&rendered_labels(&mut app, &context, [540.0, 400.0]), &[key]);
    }

    #[test]
    fn resources_use_the_service_sidebar_layout_and_open_the_selected_definition() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(
            &path,
            concat!(
                "services: {app: {}}\n",
                "networks: {edge: {}, backend: {driver: macvlan}}\n",
                "volumes: {cache: {}, storage: {driver: custom}}\n",
            ),
        )
        .unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, Some(path));
        app.language = Language::English;
        let frame = |app: &mut App, events| workspace_frame(app, &context, [1080.0, 720.0], events);
        frame(&mut app, vec![]);
        let add_service = text_position(frame(&mut app, vec![]), "+ Add service");

        for (kind, tab, add_label, entry, driver) in [
            (
                ResourceKind::Network,
                "Networks",
                "+ Add network",
                "backend",
                "Driver: macvlan",
            ),
            (
                ResourceKind::Volume,
                "Volumes",
                "+ Add volume",
                "storage",
                "Driver: custom",
            ),
        ] {
            let tab_position = text_position(frame(&mut app, vec![]), tab);
            click(&mut app, &frame, tab_position);
            let list = frame(&mut app, vec![]);
            let add = text_position(list.clone(), add_label);
            let item = text_position(list, entry);
            assert!((add.y - add_service.y).abs() < 2.0);
            assert!(add.x < 200.0 && item.x < 200.0);
            click(&mut app, &frame, item);
            let detail = frame(&mut app, vec![]);
            assert!(text_position(detail, driver).x > 225.0);
            click(&mut app, &frame, add);
            assert!(matches!(app.form, Some(Form::Resource(_))));
            assert!(app.apply_project(ProjectEdit::Resource {
                kind,
                key: "new-definition".into(),
                add: true,
                value: Some(crate::compose::ResourceInput::Settings(Default::default())),
            }));
            assert_eq!(app.selected_resource(kind).unwrap().key, "new-definition");
            app.form = None;
        }
    }

    #[test]
    fn renders_project_navigation_and_service_tabs_in_both_languages() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(
            &path,
            "services:\n  api:\n    image: nginx\n    networks: [edge]\n    volumes: [data:/data]\n  worker: {}\nnetworks:\n  edge: null\nvolumes:\n  data: null\n",
        )
        .unwrap();

        for language in [Language::German, Language::English] {
            let t = |de, en| language.text(de, en);
            let context = egui::Context::default();
            let mut app = App::new(&context, Some(path.clone()));
            app.language = language;
            assert_eq!(app.selected, Some(0));
            let size = [1080.0, 720.0];
            let labels = rendered_labels(&mut app, &context, size);
            assert_labels(
                &labels,
                &[
                    "Services",
                    t("Netzwerke", "Networks"),
                    "Volumes",
                    "worker",
                    t("Konfiguration", "Configuration"),
                    t("Verknüpfungen", "Connections"),
                    t("Erweitert", "Advanced"),
                    t("Image-Info", "Image info"),
                    "Ports",
                    t("Umgebungsvariablen", "Environment variables"),
                ],
            );
            assert!(!labels.iter().any(|text| text == "Docker Hub"));

            app.service_query = "WORK".into();
            let labels = rendered_labels(&mut app, &context, size);
            assert_labels(&labels, &["worker"]);
            assert_eq!(labels.iter().filter(|text| *text == "api").count(), 1);

            app.service_tab = ServiceTab::Connections;
            let labels = rendered_labels(&mut app, &context, size);
            assert_labels(&labels, &["Labels", t("Abhängigkeiten", "Dependencies")]);
            app.service_tab = ServiceTab::Advanced;
            let labels = rendered_labels(&mut app, &context, size);
            assert_labels(&labels, &[t("Weitere Parameter", "Additional parameters")]);

            app.project_tab = ProjectTab::Resources(ResourceKind::Network);
            app.resource_focus = Some("edge".into());
            let labels = rendered_labels(&mut app, &context, size);
            assert_labels(
                &labels,
                &[
                    "edge",
                    "api",
                    t(
                        "Diese Ressource wird von Services verwendet.",
                        "This resource is used by services.",
                    ),
                ],
            );
            app.select_service(1);
            assert_eq!(app.service_tab, ServiceTab::Configuration);
            assert_eq!(app.project_tab, ProjectTab::Services);
        }
    }

    #[test]
    fn linked_environment_keeps_the_active_service_across_layouts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        std::fs::write(&path, "services:\n  app: {}\n").unwrap();
        std::fs::write(directory.path().join(".env"), "APP_PORT=8080\n").unwrap();
        let context = egui::Context::default();
        let mut app = App::new(&context, Some(path.clone()));
        app.language = Language::English;
        app.service_tab = ServiceTab::Connections;
        let desktop = [1080.0, 720.0];
        let preview = rendered_labels(&mut app, &context, desktop);
        assert_labels(&preview, &["Configuration", "APP_PORT", "8080"]);
        assert_eq!(app.active_inspector(false), None);
        app.inspector = Some(Inspector::History);
        assert_eq!(app.active_inspector(true), Some(Inspector::History));
        app.close_inspector(Inspector::History, true);
        assert_eq!(app.active_inspector(true), Some(Inspector::Environment));
        app.close_inspector(Inspector::Environment, true);
        assert_eq!(app.active_inspector(true), None);

        app.select_env("APP_PORT");
        assert_eq!(app.inspector, Some(Inspector::Environment));

        let compact = [540.0, 400.0];
        let labels = rendered_labels(&mut app, &context, compact);
        assert_labels(&labels, &["Back to editor", "APP_PORT", "Save to .env"]);
        assert!(!labels.iter().any(|text| text == "Connections"));
        let labels = rendered_labels(&mut app, &context, [1440.0, 900.0]);
        assert_labels(&labels, &["Connections", "APP_PORT"]);
        assert!(!labels.iter().any(|text| text == "Back to editor"));

        app.inspector = None;
        let labels = rendered_labels(&mut app, &context, compact);
        assert_labels(&labels, &["Project", "Connections"]);
        assert_eq!(app.selected, Some(0));
        assert_eq!(app.service_tab, ServiceTab::Connections);
        app.open(path);
        assert_eq!(app.inspector, None);
        assert!(app.env_editor.is_none());
        assert_eq!(app.active_inspector(true), Some(Inspector::Environment));
    }
}
