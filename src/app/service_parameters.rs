use super::*;
use crate::compose::{CONDITIONS, Dependency, Mount, RowValue, SectionKind};

pub(super) struct AssignmentForm {
    pub kind: SectionKind,
    pub index: Option<usize>,
    pub value: RowValue,
    pub create_resource: bool,
}

impl AssignmentForm {
    fn new(kind: SectionKind) -> Self {
        let value = match kind {
            SectionKind::Networks => RowValue::Network(String::new()),
            SectionKind::Volumes => RowValue::Mount(Mount::default()),
            SectionKind::Labels => RowValue::Label(Variable {
                value: Some(String::new()),
                ..Variable::default()
            }),
            SectionKind::Dependencies => RowValue::Dependency(Dependency::default()),
        };
        Self {
            kind,
            index: None,
            value,
            create_resource: false,
        }
    }

    pub(super) fn fields(
        &mut self,
        ui: &mut Ui,
        language: Language,
        document: Option<&Compose>,
        selected: Option<usize>,
    ) {
        let t = |de, en| language.text(de, en);
        ui.heading(self.kind.title(language));
        match &mut self.value {
            RowValue::Network(name) => {
                resource_picker(
                    ui,
                    language,
                    document,
                    ResourceKind::Network,
                    name,
                    &mut self.create_resource,
                );
            }
            RowValue::Mount(mount) => {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_value(
                            &mut mount.bind,
                            false,
                            t("Benanntes Volume", "Named volume"),
                        )
                        .changed()
                        || ui
                            .selectable_value(&mut mount.bind, true, "Bind-Mount")
                            .changed()
                    {
                        mount.source.clear();
                        self.create_resource = false;
                    }
                });
                if mount.bind {
                    field(ui, t("Host-Pfad", "Host path"), &mut mount.source);
                } else {
                    resource_picker(
                        ui,
                        language,
                        document,
                        ResourceKind::Volume,
                        &mut mount.source,
                        &mut self.create_resource,
                    );
                }
                field(ui, t("Containerpfad", "Container path"), &mut mount.target);
                ui.checkbox(&mut mount.read_only, t("Schreibgeschützt", "Read only"));
            }
            RowValue::Label(label) => {
                field(ui, t("Schlüssel", "Key"), &mut label.key);
                let mut has_value = label.value.is_some();
                if ui
                    .checkbox(&mut has_value, t("Wert setzen", "Set a value"))
                    .changed()
                {
                    label.value = has_value.then(String::new);
                }
                if let Some(value) = &mut label.value {
                    field(ui, t("Wert", "Value"), value);
                }
            }
            RowValue::Dependency(dependency) => {
                ui.label("Service");
                choice(
                    ui,
                    "dependency-service",
                    &mut dependency.service,
                    document
                        .into_iter()
                        .flat_map(|document| document.services.iter().enumerate())
                        .filter(|(index, _)| Some(*index) != selected)
                        .map(|(_, service)| service.name.as_str()),
                    t("Service auswählen", "Select a service"),
                );
                ui.label(t("Startbedingung", "Start condition"));
                choice(
                    ui,
                    "dependency-condition",
                    &mut dependency.condition,
                    CONDITIONS,
                    "service_started",
                );
                ui.checkbox(
                    &mut dependency.restart,
                    t(
                        "Bei Compose-Neustart erneut starten",
                        "Restart on Compose restart",
                    ),
                );
                ui.checkbox(
                    &mut dependency.required,
                    t("Abhängigkeit erforderlich", "Dependency required"),
                );
            }
        }
    }

    pub(super) fn edit(&self) -> ProjectEdit {
        let mut value = self.value.clone();
        match &mut value {
            RowValue::Network(name) => *name = name.trim().into(),
            RowValue::Mount(mount) => {
                mount.source = mount.source.trim().into();
                mount.target = mount.target.trim().into();
            }
            RowValue::Label(label) => label.key = label.key.trim().into(),
            RowValue::Dependency(_) => {}
        }
        ProjectEdit::Row {
            kind: self.kind,
            index: self.index,
            value: Some(value),
            create_resource: self.create_resource,
        }
    }
}

fn resource_picker(
    ui: &mut Ui,
    language: Language,
    document: Option<&Compose>,
    kind: ResourceKind,
    key: &mut String,
    create: &mut bool,
) {
    let t = |de, en| language.text(de, en);
    let list = document.map(|document| document.resources.list(kind));
    let can_create = list.is_some_and(|list| list.can_add);
    if ui
        .add_enabled(
            can_create,
            egui::Checkbox::new(
                create,
                t(
                    "Neue Definition anlegen und zuweisen",
                    "Create and assign a definition",
                ),
            ),
        )
        .changed()
    {
        key.clear();
    }
    if *create {
        field(ui, t("Compose-Schlüssel", "Compose key"), key);
    } else {
        choice(
            ui,
            kind.key(),
            key,
            list.into_iter()
                .flat_map(|list| list.entries.iter())
                .map(|entry| entry.key.as_str()),
            t("Definition auswählen", "Select a definition"),
        );
    }
}

fn choice<'a>(
    ui: &mut Ui,
    id: &str,
    selected: &mut String,
    values: impl IntoIterator<Item = &'a str>,
    placeholder: &str,
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(if selected.is_empty() {
            placeholder
        } else {
            selected.as_str()
        })
        .width(280.0)
        .show_ui(ui, |ui| {
            for value in values {
                if ui.selectable_label(selected == value, value).clicked() {
                    *selected = value.to_owned();
                }
            }
        });
}

impl App {
    pub(super) fn service_parameter_sections(&mut self, ui: &mut Ui, service: &Service) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        for kind in SectionKind::ALL {
            let settings = service.parameters.section(kind);
            ui.push_id(kind.key(), |ui| {
                section(ui, kind.title(language), |ui| {
                    for (index, item) in settings.rows.iter().enumerate() {
                        ui.push_id(index, |ui| {
                            let ctx = ui.ctx().clone();
                            row(
                                ui,
                                &item.label,
                                settings.editable && item.value.is_some(),
                                language,
                                |edit| {
                                    if edit {
                                        if let Some(value) = &item.value {
                                            self.start_form(
                                                Form::Assignment(AssignmentForm {
                                                    kind,
                                                    index: Some(index),
                                                    value: value.clone(),
                                                    create_resource: false,
                                                }),
                                                &ctx,
                                            );
                                        }
                                    } else {
                                        self.apply_project(ProjectEdit::Row {
                                            kind,
                                            index: Some(index),
                                            value: None,
                                            create_resource: false,
                                        });
                                    }
                                },
                            );
                            if let Some(name) = &item.resource_ref {
                                let resource_kind = match kind {
                                    SectionKind::Networks => Some(ResourceKind::Network),
                                    SectionKind::Volumes => Some(ResourceKind::Volume),
                                    _ => None,
                                };
                                if let Some(resource_kind) = resource_kind
                                    && self.document.as_ref().is_some_and(|document| {
                                        document
                                            .resources
                                            .list(resource_kind)
                                            .entries
                                            .iter()
                                            .any(|entry| entry.key == *name)
                                    })
                                    && ui.link(t("Definition öffnen", "Open definition")).clicked()
                                {
                                    self.open_resource(resource_kind, name.clone());
                                }
                            }
                        });
                    }
                    if kind == SectionKind::Networks && service.parameters.network_mode {
                        ui.label(
                            RichText::new(t(
                                "Für diesen Service ist network_mode gesetzt.",
                                "This service has network_mode set.",
                            ))
                            .color(MUTED),
                        );
                    } else if !settings.editable {
                        ui.label(RichText::new(t("Schreibgeschützt", "Read only")).color(MUTED));
                    }
                    let add = match kind {
                        SectionKind::Networks => t("Netzwerk zuweisen", "Assign network"),
                        SectionKind::Volumes => t("Volume einbinden", "Add mount"),
                        SectionKind::Labels => t("Label hinzufügen", "Add label"),
                        SectionKind::Dependencies => t("Abhängigkeit hinzufügen", "Add dependency"),
                    };
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                settings.editable && settings.form_supported,
                                egui::Button::new(add),
                            )
                            .clicked()
                        {
                            self.start_form(Form::Assignment(AssignmentForm::new(kind)), ui.ctx());
                        }
                        if ui
                            .add_enabled(
                                settings.editable,
                                egui::Button::new(t("Als YAML bearbeiten", "Edit as YAML")),
                            )
                            .clicked()
                        {
                            let add = settings.yaml.is_none();
                            let yaml = settings
                                .yaml
                                .clone()
                                .unwrap_or_else(|| format!("{}: []", kind.key()));
                            self.start_form(Form::SectionYaml { kind, yaml, add }, ui.ctx());
                        }
                    });
                    if !settings.form_supported
                        && let Some(yaml) = &settings.yaml
                    {
                        ui.collapsing("YAML", |ui| {
                            ui.monospace(yaml);
                        });
                    }
                })
            });
        }
    }

    pub(super) fn open_resource(&mut self, kind: ResourceKind, name: String) {
        self.project_tab = ProjectTab::Resources(kind);
        self.resource_focus = Some(name);
        self.resource_query.clear();
        self.navigation_open = false;
        self.jump_to_resource = true;
    }
}
