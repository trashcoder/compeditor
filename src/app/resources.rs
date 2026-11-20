use super::{App, Form, ProjectEdit, section};
use crate::{
    compose::{Resource, ResourceInput, ResourceKind, ResourceSettings},
    storage::Language,
};
use eframe::egui::{self, RichText, Ui};

pub(super) struct ResourceForm {
    kind: ResourceKind,
    key: String,
    settings: ResourceSettings,
    yaml: String,
    use_yaml: bool,
    add: bool,
}

impl ResourceForm {
    pub(super) fn new(
        kind: ResourceKind,
        key: String,
        resource: Option<&Resource>,
        add: bool,
    ) -> Self {
        let settings = resource
            .and_then(|item| item.settings.clone())
            .unwrap_or_default();
        let use_yaml = resource.is_some_and(|item| item.settings.is_none());
        let yaml = resource.map_or_else(String::new, |item| item.yaml.clone());
        Self {
            kind,
            key,
            settings,
            yaml,
            use_yaml,
            add,
        }
    }

    pub(super) fn fields(&mut self, ui: &mut Ui, language: Language) {
        let t = |de, en| language.text(de, en);
        ui.heading(if self.add {
            t("Ressource hinzufügen", "Add resource")
        } else {
            t("Ressource bearbeiten", "Edit resource")
        });
        ui.label(self.kind.title(language));
        ui.add_enabled_ui(self.add, |ui| {
            ui.horizontal(|ui| {
                ui.label(t("Compose-Schlüssel", "Compose key"));
                ui.text_edit_singleline(&mut self.key);
            });
        });
        if !self.add {
            ui.label(
                RichText::new(t(
                    "Der Compose-Schlüssel bleibt unverändert.",
                    "The Compose key is immutable.",
                ))
                .weak(),
            );
        }
        ui.separator();
        ui.checkbox(
            &mut self.use_yaml,
            t("YAML direkt bearbeiten", "Edit YAML directly"),
        );
        if self.use_yaml {
            if self.add && self.yaml.is_empty() && !self.key.trim().is_empty() {
                self.yaml = format!("{}: null", self.key.trim());
            }
            ui.label(t("Definition als YAML", "Definition as YAML"));
            ui.add(
                egui::TextEdit::multiline(&mut self.yaml)
                    .desired_rows(9)
                    .desired_width(f32::INFINITY)
                    .code_editor(),
            );
        } else {
            ui.horizontal(|ui| {
                ui.label(t("Docker-Name (optional)", "Docker name (optional)"));
                ui.text_edit_singleline(&mut self.settings.name);
            });
            if !self.settings.external {
                ui.horizontal(|ui| {
                    ui.label(t("Treiber (optional)", "Driver (optional)"));
                    ui.text_edit_singleline(&mut self.settings.driver);
                });
            }
            if ui
                .checkbox(
                    &mut self.settings.external,
                    t("Extern verwaltet", "External"),
                )
                .changed()
                && self.settings.external
            {
                self.settings.driver.clear();
            }
            if self.settings.external {
                ui.label(
                    RichText::new(t(
                        "Externe Ressourcen können nicht von Compose erstellt werden.",
                        "External resources cannot be created by Compose.",
                    ))
                    .weak(),
                );
            }
        }
    }

    pub(super) fn edit(&self) -> ProjectEdit {
        ProjectEdit::Resource {
            kind: self.kind,
            key: self.key.trim().to_owned(),
            add: self.add,
            value: Some(if self.use_yaml {
                ResourceInput::Yaml(self.yaml.clone())
            } else {
                ResourceInput::Settings(self.settings.clone())
            }),
        }
    }
}

impl App {
    pub(super) fn selected_resource(&self, kind: ResourceKind) -> Option<&Resource> {
        let entries = &self.document.as_ref()?.resources.list(kind).entries;
        self.resource_focus
            .as_deref()
            .and_then(|key| entries.iter().find(|resource| resource.key == key))
            .or_else(|| entries.first())
    }

    pub(super) fn resource_view(&mut self, ui: &mut Ui, kind: ResourceKind) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        let Some(resource) = self.selected_resource(kind).cloned() else {
            let title = match kind {
                ResourceKind::Network => t("Keine Netzwerke", "No networks"),
                ResourceKind::Volume => t("Keine Volumes", "No volumes"),
            };
            section(ui, title, |ui| {
                ui.label(t(
                    "Füge eine Definition in der Projektleiste hinzu.",
                    "Add a definition in the project sidebar.",
                ));
            });
            return;
        };
        let unknown_usage = self
            .document
            .as_ref()
            .is_some_and(|document| document.resources.list(kind).unknown_usage);
        let heading =
            ui.add(egui::Label::new(RichText::new(&resource.key).size(32.0).strong()).truncate());
        heading.on_hover_text(&resource.key);
        ui.separator();
        section(ui, t("Definition", "Definition"), |ui| {
            if let Some(settings) = &resource.settings {
                if !settings.name.is_empty() {
                    ui.label(format!("Docker: {}", settings.name));
                }
                if !settings.driver.is_empty() {
                    ui.label(format!("{}: {}", t("Treiber", "Driver"), settings.driver));
                }
                if settings.external {
                    ui.label(t("Extern", "External"));
                }
            }
            let reason = if !resource.used_by.is_empty() {
                Some(t(
                    "Diese Ressource wird von Services verwendet.",
                    "This resource is used by services.",
                ))
            } else if unknown_usage {
                Some(t(
                    "Die Verwendung konnte nicht sicher ermittelt werden.",
                    "Resource usage could not be determined safely.",
                ))
            } else if !resource.editable {
                Some(t(
                    "Diese Definition ist schreibgeschützt.",
                    "This definition is read-only.",
                ))
            } else {
                None
            };
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        resource.editable,
                        egui::Button::new(t("Bearbeiten", "Edit")),
                    )
                    .clicked()
                {
                    self.start_form(
                        Form::Resource(ResourceForm::new(
                            kind,
                            resource.key.clone(),
                            Some(&resource),
                            false,
                        )),
                        ui.ctx(),
                    );
                }
                let remove = ui.add_enabled(
                    reason.is_none(),
                    egui::Button::new(t("Entfernen", "Remove")),
                );
                if let Some(reason) = reason {
                    remove.on_disabled_hover_text(reason);
                } else if remove.clicked() {
                    self.apply_project(ProjectEdit::Resource {
                        kind,
                        key: resource.key.clone(),
                        add: false,
                        value: None,
                    });
                }
            });
            if let Some(reason) = reason {
                ui.label(RichText::new(reason).weak());
            }
            ui.collapsing("YAML", |ui| ui.code(&resource.yaml));
        });
        if !resource.used_by.is_empty() {
            section(ui, t("Verwendet von", "Used by"), |ui| {
                ui.horizontal_wrapped(|ui| {
                    for service in &resource.used_by {
                        if ui.link(service).clicked()
                            && let Some(index) = self.document.as_ref().and_then(|document| {
                                document
                                    .services
                                    .iter()
                                    .position(|item| item.name == *service)
                            })
                        {
                            self.service_tab = super::ServiceTab::Configuration;
                            self.select_service(index);
                        }
                    }
                });
            });
        }
    }
}
