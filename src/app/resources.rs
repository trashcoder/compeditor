use super::{App, Form, ProjectEdit, ProjectTab};
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
    pub(super) fn resource_view(&mut self, ui: &mut Ui, kind: ResourceKind) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let list = document.resources.list(kind);
        let entries = list.entries.clone();
        let can_add = list.can_add;
        let unknown_usage = list.unknown_usage;

        ui.heading(kind.title(language));
        if document.dirty() {
            ui.label(RichText::new(t("Ungespeichert", "Unsaved")).color(super::ACCENT));
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(can_add, egui::Button::new(t("+ Hinzufügen", "+ Add"))).clicked() {
                self.start_form(Form::Resource(ResourceForm::new(kind, String::new(), None, true)), ui.ctx());
            }
            if !can_add {
                ui.label(RichText::new(t("Definitionen sind aufgrund geteilter, geerbter oder nicht unterstützter YAML-Werte schreibgeschützt.", "Definitions are read-only because the YAML uses shared, inherited, or unsupported values.")).weak());
            }
        });

        if entries.is_empty() {
            ui.label(RichText::new(t("Keine Definitionen", "No definitions")).weak());
        }
        for resource in &entries {
            let selected = self.resource_focus.as_deref() == Some(resource.key.as_str());
            let card = ui.push_id(&resource.key, |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .selectable_label(selected, RichText::new(&resource.key).strong())
                            .clicked()
                        {
                            self.resource_focus = Some(resource.key.clone());
                        }
                        if let Some(settings) = &resource.settings {
                            if !settings.name.is_empty() {
                                ui.label(format!("Docker: {}", settings.name));
                            }
                            if !settings.driver.is_empty() {
                                ui.label(format!(
                                    "{}: {}",
                                    t("Treiber", "Driver"),
                                    settings.driver
                                ));
                            }
                            if settings.external {
                                ui.label(t("Extern", "External"));
                            }
                        }
                        if !resource.editable {
                            ui.label(RichText::new(t("Schreibgeschützt", "Read-only")).weak());
                        }
                    });
                    if !resource.used_by.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(t("Verwendet von:", "Used by:"));
                            for service in &resource.used_by {
                                if ui.link(service).clicked() {
                                    self.selected = self.document.as_ref().and_then(|doc| {
                                        doc.services.iter().position(|item| item.name == *service)
                                    });
                                    self.project_tab = ProjectTab::Services;
                                    self.resource_focus = None;
                                }
                            }
                        });
                    }
                    if selected {
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
                                        Some(resource),
                                        false,
                                    )),
                                    ui.ctx(),
                                );
                            }
                            let protected =
                                !resource.used_by.is_empty() || unknown_usage || !resource.editable;
                            let remove = ui.add_enabled(
                                !protected,
                                egui::Button::new(t("Entfernen", "Remove")),
                            );
                            if protected {
                                let reason = if !resource.used_by.is_empty() {
                                    t(
                                        "Diese Ressource wird von Services verwendet.",
                                        "This resource is used by services.",
                                    )
                                } else if unknown_usage {
                                    t(
                                        "Die Verwendung konnte nicht sicher ermittelt werden.",
                                        "Resource usage could not be determined safely.",
                                    )
                                } else {
                                    t(
                                        "Diese Definition ist schreibgeschützt.",
                                        "This definition is read-only.",
                                    )
                                };
                                remove.on_hover_text(reason);
                                ui.label(RichText::new(reason).weak());
                            } else if remove.clicked() {
                                self.apply_project(ProjectEdit::Resource {
                                    kind,
                                    key: resource.key.clone(),
                                    add: false,
                                    value: None,
                                });
                            }
                        });
                        ui.collapsing(t("YAML", "YAML"), |ui| {
                            ui.code(resource.yaml.clone());
                        });
                    }
                })
            });
            if selected && self.jump_to_resource {
                card.response.scroll_to_me(Some(egui::Align::Center));
                self.jump_to_resource = false;
            }
        }
        if unknown_usage {
            ui.label(RichText::new(t("Entfernen ist deaktiviert, weil eine Referenz nicht sicher ausgewertet werden konnte.", "Removal is disabled because a reference could not be evaluated safely.")).weak());
        }
    }
}
