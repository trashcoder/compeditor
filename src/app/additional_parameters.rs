use super::*;
use crate::compose::Parameter;
use yaml_edit::{ScalarValue, YamlFile, YamlNode};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueMode {
    Text,
    List,
    Boolean,
    Yaml,
}

const CATALOG: &[(&str, &str, &str, ValueMode, &str)] = &[
    (
        "container_name",
        "Containername",
        "Container name",
        ValueMode::Text,
        "\"\"",
    ),
    ("hostname", "Hostname", "Hostname", ValueMode::Text, "\"\""),
    ("user", "Benutzer", "User", ValueMode::Text, "\"\""),
    (
        "stop_signal",
        "Stoppsignal",
        "Stop signal",
        ValueMode::Text,
        "SIGTERM",
    ),
    (
        "stop_grace_period",
        "Wartezeit beim Stoppen",
        "Stop grace period",
        ValueMode::Text,
        "10s",
    ),
    (
        "shm_size",
        "Größe des gemeinsamen Speichers",
        "Shared memory size",
        ValueMode::Text,
        "64m",
    ),
    (
        "init",
        "Init-Prozess",
        "Init process",
        ValueMode::Boolean,
        "true",
    ),
    ("profiles", "Profile", "Profiles", ValueMode::List, "[]"),
    ("command", "Befehl", "Command", ValueMode::Text, "\"\""),
    (
        "entrypoint",
        "Einstiegspunkt",
        "Entrypoint",
        ValueMode::Text,
        "\"\"",
    ),
    (
        "build",
        "Build-Konfiguration",
        "Build configuration",
        ValueMode::Yaml,
        ".",
    ),
    (
        "healthcheck",
        "Gesundheitsprüfung",
        "Health check",
        ValueMode::Yaml,
        "\n  test: [CMD, echo, ok]\n  interval: 30s",
    ),
    (
        "logging",
        "Protokollierung",
        "Logging",
        ValueMode::Yaml,
        "\n  driver: json-file\n  options:\n    max-size: 10m",
    ),
    (
        "ulimits",
        "Ressourcenlimits",
        "Resource limits",
        ValueMode::Yaml,
        "\n  nofile:\n    soft: 1024\n    hard: 2048",
    ),
    ("dns", "DNS-Server", "DNS servers", ValueMode::Yaml, "[]"),
    (
        "extra_hosts",
        "Zusätzliche Hosts",
        "Extra hosts",
        ValueMode::Yaml,
        "{}",
    ),
    (
        "deploy",
        "Bereitstellung",
        "Deployment",
        ValueMode::Yaml,
        "{}",
    ),
    (
        "cap_add",
        "Zusätzliche Linux-Capabilities",
        "Additional Linux capabilities",
        ValueMode::Yaml,
        "[]",
    ),
    ("devices", "Geräte", "Devices", ValueMode::Yaml, "[]"),
    (
        "security_opt",
        "Sicherheitsoptionen",
        "Security options",
        ValueMode::Yaml,
        "[]",
    ),
    (
        "working_dir",
        "Arbeitsverzeichnis",
        "Working directory",
        ValueMode::Yaml,
        "\"\"",
    ),
];

pub(super) struct ParameterForm {
    key: String,
    original: String,
    yaml: String,
    adding: bool,
    custom: bool,
    mode: ValueMode,
    text: String,
    list: Vec<String>,
    boolean: bool,
    initial_form_yaml: String,
}

impl ParameterForm {
    fn new(key: String, entry: Option<&Parameter>) -> Self {
        let catalog = CATALOG.iter().find(|item| item.0 == key);
        let yaml = entry.map_or_else(
            || catalog.map_or_else(String::new, |item| yaml_entry(&key, item.4)),
            |entry| entry.yaml.clone(),
        );
        let value = parsed_value(&yaml, &key);
        let text = value
            .as_ref()
            .and_then(YamlNode::as_scalar)
            .map(|scalar| scalar.as_string());
        let list: Option<Vec<String>> =
            value
                .as_ref()
                .and_then(YamlNode::as_sequence)
                .and_then(|sequence| {
                    sequence
                        .values()
                        .map(|node| Some(node.as_scalar()?.as_string()))
                        .collect()
                });
        let boolean = value
            .as_ref()
            .and_then(YamlNode::as_scalar)
            .and_then(|scalar| scalar.as_bool());
        let suggested = catalog.map_or(ValueMode::Yaml, |item| item.3);
        let mode = match suggested {
            ValueMode::Text
                if list.is_some() && matches!(key.as_str(), "command" | "entrypoint") =>
            {
                ValueMode::List
            }
            ValueMode::Text if text.is_some() => ValueMode::Text,
            ValueMode::List if list.is_some() => ValueMode::List,
            ValueMode::Boolean if boolean.is_some() => ValueMode::Boolean,
            _ => ValueMode::Yaml,
        };
        let custom = key.is_empty();
        let mut form = Self {
            key,
            original: yaml.clone(),
            yaml,
            adding: entry.is_none(),
            custom,
            mode,
            text: text.unwrap_or_default(),
            list: list.unwrap_or_default(),
            boolean: boolean.unwrap_or(false),
            initial_form_yaml: String::new(),
        };
        form.initial_form_yaml = form.structured_yaml();
        form
    }

    fn structured_yaml(&self) -> String {
        let quote = |value: &str| ScalarValue::double_quoted(value.to_owned()).to_string();
        let value = match self.mode {
            ValueMode::Text => quote(&self.text),
            ValueMode::Boolean => self.boolean.to_string(),
            ValueMode::List if self.list.is_empty() => "[]".into(),
            ValueMode::List => self
                .list
                .iter()
                .map(|item| format!("\n  - {}", quote(item)))
                .collect(),
            ValueMode::Yaml => return self.yaml.clone(),
        };
        yaml_entry(&self.key, &value)
    }

    fn current_structured_yaml(&self) -> String {
        let generated = self.structured_yaml();
        if generated == self.initial_form_yaml {
            self.original.clone()
        } else {
            generated
        }
    }

    pub(super) fn fields(&mut self, ui: &mut Ui, language: Language) {
        let t = |de, en| language.text(de, en);
        ui.heading(if self.adding {
            t("Parameter hinzufügen", "Add parameter")
        } else {
            t("Parameter bearbeiten", "Edit parameter")
        });
        if self.custom {
            let previous = self.key.clone();
            field(ui, t("Compose-Schlüssel", "Compose key"), &mut self.key);
            if self.yaml.is_empty() || self.yaml == yaml_entry(&previous, "null") {
                self.yaml = yaml_entry(self.key.trim(), "null");
            }
        } else {
            ui.monospace(&self.key);
        }
        if self.mode != ValueMode::Yaml {
            if matches!(self.key.as_str(), "command" | "entrypoint") {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.mode, ValueMode::Text, t("Text", "Text"));
                    ui.selectable_value(
                        &mut self.mode,
                        ValueMode::List,
                        t("Argumentliste", "Argument list"),
                    );
                });
            }
            if ui
                .button(t("Als YAML bearbeiten", "Edit as YAML"))
                .clicked()
            {
                self.yaml = self.current_structured_yaml();
                self.mode = ValueMode::Yaml;
            }
        }
        match self.mode {
            ValueMode::Text => field(ui, t("Wert", "Value"), &mut self.text),
            ValueMode::Boolean => {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.boolean, true, t("Ja", "Yes"));
                    ui.selectable_value(&mut self.boolean, false, t("Nein", "No"));
                });
            }
            ValueMode::List => {
                let mut remove = None;
                for (index, item) in self.list.iter_mut().enumerate() {
                    ui.push_id(index, |ui| {
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(item);
                            if ui.small_button(t("Entfernen", "Remove")).clicked() {
                                remove = Some(index);
                            }
                        });
                    });
                }
                if let Some(index) = remove {
                    self.list.remove(index);
                }
                if ui
                    .small_button(t("Eintrag hinzufügen", "Add item"))
                    .clicked()
                {
                    self.list.push(String::new());
                }
            }
            ValueMode::Yaml => {
                ui.add(
                    egui::TextEdit::multiline(&mut self.yaml)
                        .code_editor()
                        .desired_rows(9)
                        .desired_width(f32::INFINITY),
                );
            }
        }
    }

    pub(super) fn edit(&self) -> ProjectEdit {
        ProjectEdit::Parameter {
            key: self.key.trim().into(),
            add: self.adding,
            yaml: Some(if self.mode == ValueMode::Yaml {
                self.yaml.clone()
            } else {
                self.current_structured_yaml()
            }),
        }
    }
}

fn yaml_entry(key: &str, value: &str) -> String {
    if value.starts_with('\n') {
        format!("{key}:{value}")
    } else {
        format!("{key}: {value}")
    }
}

fn parsed_value(source: &str, key: &str) -> Option<YamlNode> {
    let file = YamlFile::parse(source).to_result().ok()?;
    file.document()?.as_mapping()?.get(key)
}

impl App {
    pub(super) fn additional_parameters(&mut self, ui: &mut Ui, service: &Service) {
        let language = self.language;
        let t = |de, en| language.text(de, en);
        let search_id = Id::new(("parameter-search", self.document_view, &service.name));
        let mut search = ui
            .ctx()
            .data(|data| data.get_temp::<String>(search_id).unwrap_or_default());
        section(ui, t("Weitere Parameter", "Additional parameters"), |ui| {
            if service.parameters.can_add {
                egui::CollapsingHeader::new(t("Parameter hinzufügen", "Add parameter"))
                    .id_salt(search_id)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut search)
                                    .hint_text(t("Parameter suchen …", "Search parameters …")),
                            );
                            if ui.button(t("Eigener Schlüssel", "Custom key")).clicked() {
                                self.start_form(
                                    Form::Parameter(ParameterForm::new(String::new(), None)),
                                    ui.ctx(),
                                );
                            }
                        });
                        let query = search.to_lowercase();
                        ui.horizontal_wrapped(|ui| {
                            for &(key, de, en, ..) in CATALOG {
                                let matches = key.contains(&query)
                                    || de.to_lowercase().contains(&query)
                                    || en.to_lowercase().contains(&query);
                                if matches
                                    && !service
                                        .parameters
                                        .additional
                                        .iter()
                                        .any(|parameter| parameter.key == key)
                                    && ui.small_button(format!("{key} · {}", t(de, en))).clicked()
                                {
                                    self.start_form(
                                        Form::Parameter(ParameterForm::new(key.into(), None)),
                                        ui.ctx(),
                                    );
                                }
                            }
                        });
                    });
            }
            for parameter in &service.parameters.additional {
                ui.push_id(&parameter.key, |ui| {
                    let ctx = ui.ctx().clone();
                    let line = parameter.yaml.lines().next().unwrap_or(&parameter.key);
                    let preview: String = line.chars().take(95).collect();
                    row(ui, &preview, parameter.editable, language, |edit| {
                        if edit {
                            self.start_form(
                                Form::Parameter(ParameterForm::new(
                                    parameter.key.clone(),
                                    Some(parameter),
                                )),
                                &ctx,
                            );
                        } else {
                            self.apply_project(ProjectEdit::Parameter {
                                key: parameter.key.clone(),
                                add: false,
                                yaml: None,
                            });
                        }
                    });
                    if parameter.yaml.lines().count() > 1 || preview != line {
                        ui.collapsing("YAML", |ui| {
                            ui.monospace(&parameter.yaml);
                        });
                    }
                });
            }
        });
        ui.ctx()
            .data_mut(|data| data.insert_temp(search_id, search));
    }
}
