mod parameters;

pub use parameters::*;

use crate::{
    environment::{self, Environment, SaveError},
    history::{Change, Draft, EnvSnapshot, History, Operation, Target},
    storage::{self, Language},
    yaml_format,
};
use std::{
    cell::OnceCell,
    collections::HashSet,
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};
use yaml_edit::{
    AnchorRegistry, AsYaml, Mapping, MergedMapping, ScalarValue, Sequence, SyntaxKind, YamlFile,
    YamlNode,
};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Empty,
    Yaml,
    Services,
    Duplicate(String),
    Protected,
    ResourceInUse(String),
    UsageUncertain,
    Input(&'static str, &'static str),
    Conflict,
    ProjectExists(String),
    RestoreConflict,
    RestorePartial,
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Error {
    pub fn message(&self, language: Language) -> String {
        let t = |de, en| language.text(de, en);
        match self {
            Self::Io(error) => format!("{} ({:?})", t("Dateizugriff fehlgeschlagen", "Could not access the file"), error.kind()),
            Self::Empty => t("Die Datei ist leer.", "The file is empty.").into(),
            Self::Yaml => t("Die Datei muss genau ein gültiges YAML-Dokument enthalten.", "The file must contain exactly one valid YAML document.").into(),
            Self::Services => t("Unter „services“ wird eine Zuordnung mit Servicenamen erwartet.", "Expected a mapping of service names under ‘services’.").into(),
            Self::Duplicate(name) => format!("{} {name}", t("Doppelter YAML-Schlüssel:", "Duplicate YAML key:")),
            Self::ResourceInUse(services) => format!("{} {services}", t("Verwendet von", "Used by")),
            Self::UsageUncertain => t("Die Nutzung lässt sich wegen externer oder dynamischer Verweise nicht sicher prüfen.", "Usage cannot be verified because of external or dynamic references.").into(),
            Self::Protected => t("Dieses Feld ist schreibgeschützt.", "This field is read-only.").into(),
            Self::Input(de, en) => t(de, en).into(),
            Self::Conflict => t("Die Datei wurde außerhalb von CompEditor geändert. Sichere deine Fassung über „Speichern unter“.", "The file changed outside CompEditor. Use Save As to keep your version.").into(),
            Self::ProjectExists(files) => format!("{} {files}", t("Im ausgewählten Ordner sind bereits Dateien vorhanden:", "The selected folder already contains:")),
            Self::RestoreConflict => t("Die YAML- oder .env-Datei wurde extern geändert. Öffne die Compose-Datei erneut, bevor du einen Stand wiederherstellst.", "The YAML or .env file changed externally. Reopen the Compose file before restoring a version.").into(),
            Self::RestorePartial => t("Die Wiederherstellung wurde unterbrochen. Öffne die Compose-Datei erneut, um den Stand auf der Festplatte zu sehen.", "Restoration was interrupted. Reopen the Compose file to see the state on disk.").into(),
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Port {
    Short(String),
    Long(LongPort),
    Other(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LongPort {
    pub target: String,
    pub published: String,
    pub host_ip: String,
    pub protocol: String,
}

impl Port {
    pub fn summary(&self) -> String {
        match self {
            Self::Short(value) | Self::Other(value) => value.clone(),
            Self::Long(port) => {
                let mut result = String::new();
                if !port.host_ip.is_empty() {
                    result.push_str(&port.host_ip);
                    result.push(':');
                }
                if !port.published.is_empty() {
                    result.push_str(&port.published);
                    result.push_str(" → ");
                }
                result.push_str(&port.target);
                if !port.protocol.is_empty() {
                    result.push('/');
                    result.push_str(&port.protocol);
                }
                result
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Variable {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Service {
    pub name: String,
    pub image: Option<String>,
    pub restart: Option<String>,
    pub build_context: Option<String>,
    pub ports: Vec<Port>,
    pub variables: Vec<Variable>,
    pub parameters: ServiceParameters,
    pub image_editable: bool,
    pub restart_editable: bool,
    pub ports_editable: bool,
    pub variables_editable: bool,
}

pub enum Edit {
    Image(Option<String>),
    Restart(Option<String>),
    Port(Option<usize>, Port),
    RemovePort(usize),
    Variable(Option<usize>, Variable),
    RemoveVariable(usize),
}

#[derive(Default)]
struct EnvironmentUsage {
    names: HashSet<String>,
    env_files: Vec<String>,
}

// These Compose settings can be read from .env without a YAML reference.
const COMPOSE_ENV_SETTINGS: &[&str] = &[
    "COMPOSE_ANSI",
    "COMPOSE_CONVERT_WINDOWS_PATHS",
    "COMPOSE_DISABLE_ENV_FILE",
    "COMPOSE_ENV_FILES",
    "COMPOSE_EXPERIMENTAL",
    "COMPOSE_FILE",
    "COMPOSE_IGNORE_ORPHANS",
    "COMPOSE_MENU",
    "COMPOSE_PARALLEL_LIMIT",
    "COMPOSE_PATH_SEPARATOR",
    "COMPOSE_PROFILES",
    "COMPOSE_PROGRESS",
    "COMPOSE_PROJECT_NAME",
    "COMPOSE_REMOVE_ORPHANS",
    "COMPOSE_STATUS_STDOUT",
];

pub struct Compose {
    pub path: PathBuf,
    original: String,
    source: String,
    needs_formatting: bool,
    environment_usage: OnceCell<EnvironmentUsage>,
    pub services: Vec<Service>,
    pub has_include: bool,
    pub resources: Resources,
    pub environment: Environment,
    pub history: History,
    pub drafts: Vec<Draft>,
}

impl Compose {
    pub fn create_in(directory: &Path) -> Result<Self> {
        let directory = fs::canonicalize(directory)?;
        if !directory.is_dir() {
            return Err(Error::Input(
                "Wähle einen vorhandenen Ordner.",
                "Choose an existing folder.",
            ));
        }
        let path = directory.join("docker-compose.yml");
        let env_path = directory.join(".env");
        let history_path = History::sidecar_path(&path);
        let mut existing = Vec::new();
        for target in [&path, &env_path, &history_path] {
            match fs::symlink_metadata(target) {
                Ok(_) => existing.push(target.file_name().unwrap_or_default().to_string_lossy()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        if !existing.is_empty() {
            return Err(Error::ProjectExists(existing.join(", ")));
        }

        // An empty block mapping has no standalone YAML representation. Keep
        // the document as `services:` until the first service is added.
        let yaml = b"services:\n";
        let env = b"";
        let mut created = Vec::new();
        for (target, contents) in [(&path, yaml.as_slice()), (&env_path, env.as_slice())] {
            let mut file = match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
            {
                Ok(file) => file,
                Err(error) => {
                    remove_created(&created);
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        let name = target.file_name().unwrap_or_default().to_string_lossy();
                        return Err(Error::ProjectExists(name.into_owned()));
                    }
                    return Err(error.into());
                }
            };
            created.push((target.to_path_buf(), contents.to_vec()));
            if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
                drop(file);
                remove_created(&created);
                return Err(error.into());
            }
        }
        let mut document = match Self::open(&path) {
            Ok(document) => document,
            Err(error) => {
                remove_created(&created);
                return Err(error);
            }
        };
        document.history.persist();
        if document.history.notice.is_some() {
            remove_created(&created);
            return Err(Error::Io(io::Error::other(
                "could not create initial history file",
            )));
        }
        Ok(document)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path)?;
        let source = fs::read_to_string(&path)?;
        let file = parse(&source)?;
        let (services, has_include, resources) = inspect(&file)?;
        let env = EnvSnapshot::read(&path);
        Ok(Self {
            environment: Environment::load(&path),
            history: History::load(&path, Some(&source), env),
            drafts: Vec::new(),
            path,
            original: source.clone(),
            source,
            needs_formatting: needs_formatting(&file),
            environment_usage: OnceCell::new(),
            services,
            has_include,
            resources,
        })
    }

    pub fn unused_environment_variables(&self) -> HashSet<&str> {
        let Environment::Loaded(dotenv) = &self.environment else {
            return HashSet::new();
        };
        let usage = self.environment_usage.get_or_init(|| {
            let mut usage = EnvironmentUsage::default();
            if let Ok(file) = parse(&self.source)
                && let Some(document) = file.document()
                && let Some(root) = document.as_mapping()
            {
                let registry = AnchorRegistry::from_document(&document);
                collect_environment_usage(&YamlNode::Mapping(root), &registry, &mut usage);
            }
            usage
        });
        let dotenv_path = self.path.with_file_name(".env");
        if usage.env_files.iter().any(|path| {
            let preview = self.environment.preview(path);
            let path = Path::new(&preview.text);
            !preview.unresolved
                && (path == dotenv_path || path == Path::new(".env") || path == Path::new("./.env"))
        }) {
            return HashSet::new();
        }
        let mut used: HashSet<_> = usage.names.iter().map(String::as_str).collect();
        used.extend(COMPOSE_ENV_SETTINGS.iter().copied());
        used.extend(self.services.iter().flat_map(|service| {
            service
                .variables
                .iter()
                .filter(|variable| variable.value.is_none())
                .map(|variable| variable.key.as_str())
        }));
        dotenv.unused_variables(used)
    }

    pub fn dirty(&self) -> bool {
        self.source != self.original
    }

    pub fn can_save(&self) -> bool {
        self.dirty() || self.needs_formatting
    }

    pub fn add_service(&mut self, name: &str, image: Option<&str>) -> Result<usize> {
        let name = name.trim();
        if name.is_empty()
            || !name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            })
            || !name
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
        {
            return Err(Error::Input(
                "Der Servicename darf nur Buchstaben, Zahlen, Punkt, Bindestrich und Unterstrich enthalten und muss mit einem Buchstaben oder einer Zahl beginnen.",
                "Service names may contain letters, numbers, dots, hyphens, and underscores, and must start with a letter or number.",
            ));
        }
        if self.services.iter().any(|service| service.name == name) {
            return Err(Error::Input(
                "Dieser Servicename ist bereits vorhanden.",
                "This service name already exists.",
            ));
        }
        let image = image.map(str::trim).filter(|image| !image.is_empty());
        if image.is_some_and(|image| image.chars().any(char::is_control)) {
            return Err(Error::Input(
                "Gib ein gültiges Image ein.",
                "Enter a valid image.",
            ));
        }
        let mut file = parse(&self.source)?;
        let document = file.document().ok_or(Error::Yaml)?;
        let root = document.as_mapping().ok_or(Error::Services)?;
        let services_node = root.get("services");
        let services_alias = services_node.as_ref().is_some_and(YamlNode::is_alias);
        let services_shared = services_node
            .as_ref()
            .is_some_and(|node| shared(node, true));
        let services = root.get_mapping("services");
        if services_alias || services_shared || services.as_ref().is_some_and(has_merge) {
            return Err(Error::Protected);
        }
        let root = if let Some(node) = services.as_ref().and_then(AsYaml::as_node)
            && yaml_format::needs_formatting(node)
        {
            file = parse(&yaml_format::block_style(&self.source, node))?;
            file.document()
                .and_then(|doc| doc.as_mapping())
                .ok_or(Error::Services)?
        } else {
            root
        };
        if services
            .as_ref()
            .is_some_and(|map| map.iter().next().is_none())
            || (root.contains_key("services") && empty_yaml_value(services_node.as_ref()))
        {
            root.set("services", Mapping::new_pending_block());
        }
        let services = root.get_mapping("services").ok_or(Error::Services)?;
        unique_keys(&services)?;
        let service = Mapping::new_pending_block();
        if let Some(image) = image {
            service.set("image", string(image.to_owned()));
        }
        services.set(name, service);
        let candidate = keep_line_endings(&self.source, file.to_string());
        let (views, has_include, resources) = inspect(&parse(&candidate)?)?;
        let index = views
            .iter()
            .position(|service| service.name == name)
            .ok_or(Error::Services)?;
        self.history.discard_future();
        self.drafts.push(Draft {
            yaml: candidate.clone(),
            change: Change::ServiceAdded {
                service: name.to_owned(),
            },
        });
        self.source = candidate;
        self.environment_usage.take();
        self.services = views;
        self.has_include = has_include;
        self.resources = resources;
        Ok(index)
    }

    pub fn reload_environment(&mut self) {
        self.environment = Environment::load(&self.path);
        if self.history.record(
            &self.original,
            EnvSnapshot::read(&self.path),
            Change::External,
        ) {
            self.history.persist();
        }
    }

    pub fn save_env_value(&mut self, key: &str, value: &str) -> std::result::Result<(), SaveError> {
        let operation = if self.environment.value(key).is_some() {
            Operation::Updated
        } else {
            Operation::Added
        };
        self.environment.save_value(&self.path, key, value)?;
        self.history.record(
            &self.original,
            EnvSnapshot::read(&self.path),
            Change::Dotenv {
                key: key.into(),
                operation,
            },
        );
        self.history.persist();
        Ok(())
    }

    pub fn save(&mut self, path: PathBuf) -> Result<()> {
        let file = parse(&self.source)?;
        let document = file.document().ok_or(Error::Yaml)?;
        let node = document.as_node().ok_or(Error::Yaml)?;
        let source = keep_line_endings(&self.source, yaml_format::block_style(&self.source, node));
        let (services, has_include, resources) = inspect(&parse(&source)?)?;
        let path = match fs::canonicalize(&path) {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = path
                    .file_name()
                    .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
                let parent = path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                fs::canonicalize(parent)?.join(name)
            }
            Err(error) => return Err(error.into()),
        };
        let expected = if path == self.path {
            Some(self.original.as_bytes().to_vec())
        } else {
            storage::read_optional(&path)?
        };
        let mut conflict = false;
        let result = storage::replace(&path, source.as_bytes(), || {
            conflict = storage::read_optional(&path)? != expected;
            if conflict {
                Err(io::ErrorKind::AlreadyExists.into())
            } else {
                Ok(())
            }
        });
        if conflict {
            return Err(Error::Conflict);
        }
        result?;
        let env = EnvSnapshot::read(&path);
        if self.path != path {
            let previous_yaml = expected
                .as_deref()
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .filter(|yaml| parse(yaml).and_then(|file| inspect(&file)).is_ok());
            self.history = History::load(&path, previous_yaml, env.clone());
            self.history.record(&source, env, Change::SavedAs);
            self.environment = Environment::load(&path);
            self.drafts.clear();
        } else {
            self.history
                .record(&self.original, env.clone(), Change::External);
            if let Some(draft) = self.drafts.last_mut() {
                draft.yaml.clone_from(&source);
            }
            for draft in self.drafts.drain(..) {
                self.history.record(&draft.yaml, env.clone(), draft.change);
            }
            self.history.record(&source, env, Change::Formatted);
        }
        self.history.persist();
        self.path = path;
        self.original.clone_from(&source);
        self.source = source;
        self.environment_usage.take();
        self.services = services;
        self.has_include = has_include;
        self.resources = resources;
        self.needs_formatting = false;
        Ok(())
    }

    pub fn edit(&mut self, service_index: usize, edit: Edit) -> Result<()> {
        let view = self.services.get(service_index).ok_or(Error::Protected)?;
        let change = match &edit {
            Edit::Image(image) => Change::Image {
                service: view.name.clone(),
                before: view.image.clone(),
                after: image.clone(),
            },
            Edit::Restart(restart) => Change::RestartPolicy {
                service: view.name.clone(),
                before: view.restart.clone(),
                after: restart.clone(),
            },
            Edit::Port(index, port) => Change::Ports {
                service: view.name.clone(),
                before: index
                    .and_then(|index| view.ports.get(index))
                    .map(Port::summary),
                after: Some(port.summary()),
            },
            Edit::RemovePort(index) => Change::Ports {
                service: view.name.clone(),
                before: view.ports.get(*index).map(Port::summary),
                after: None,
            },
            Edit::Variable(index, variable) => Change::Variable {
                service: view.name.clone(),
                key: variable.key.clone(),
                operation: if index.is_some() {
                    Operation::Updated
                } else {
                    Operation::Added
                },
            },
            Edit::RemoveVariable(index) => Change::Variable {
                service: view.name.clone(),
                key: view
                    .variables
                    .get(*index)
                    .ok_or(Error::Protected)?
                    .key
                    .clone(),
                operation: Operation::Removed,
            },
        };
        let mut source = self.source.as_str();
        let mut file = parse(source)?;
        let mapping = service_map(&file, &view.name)?;
        let formatted;
        let mapping = if view.variables_editable
            && matches!(&edit, Edit::Variable(..) | Edit::RemoveVariable(_))
            && let Some(environment) = mapping.get_mapping("environment")
            && environment.is_flow_style()
        {
            if environment.iter().next().is_none() {
                mapping.set("environment", Mapping::new_pending_block());
                mapping
            } else {
                let node = environment.as_node().ok_or(Error::Yaml)?;
                formatted = keep_line_endings(source, yaml_format::block_style(source, node));
                source = &formatted;
                file = parse(source)?;
                service_map(&file, &view.name)?
            }
        } else {
            mapping
        };
        let replacement = match edit {
            Edit::Image(image) if view.image_editable => {
                if image == view.image {
                    return Ok(());
                }
                if let Some(image) = image {
                    if image.trim().is_empty() || image.chars().any(char::is_control) {
                        return Err(Error::Input(
                            "Gib ein gültiges Image ein.",
                            "Enter a valid image.",
                        ));
                    }
                    mapping.set("image", string(image));
                } else {
                    mapping.remove("image");
                }
                None
            }
            Edit::Restart(restart) if view.restart_editable => {
                if restart == view.restart {
                    return Ok(());
                }
                if let Some(restart) = restart {
                    if !valid_restart_policy(&restart) {
                        return Err(Error::Input(
                            "Wähle eine gültige Neustart-Richtlinie.",
                            "Choose a valid restart policy.",
                        ));
                    }
                    mapping.set("restart", string(restart));
                } else {
                    mapping.remove("restart");
                }
                None
            }
            Edit::Port(index, port) if view.ports_editable => {
                set_port(source, &mapping, index, port)?
            }
            Edit::RemovePort(index) if view.ports_editable => {
                let ports = mapping.get_sequence("ports").ok_or(Error::Protected)?;
                if ports.len() == 1 {
                    mapping.remove("ports");
                } else {
                    ports.remove(index).ok_or(Error::Protected)?;
                }
                None
            }
            Edit::Variable(index, variable) if view.variables_editable => {
                set_variable(source, &mapping, view, index, variable)?
            }
            Edit::RemoveVariable(index) if view.variables_editable => {
                let variable = view.variables.get(index).ok_or(Error::Protected)?;
                if let Some(list) = mapping.get_sequence("environment") {
                    if list.len() == 1 {
                        mapping.remove("environment");
                    } else {
                        list.remove(index).ok_or(Error::Protected)?;
                    }
                } else if let Some(env) = mapping.get_mapping("environment") {
                    if env.iter().count() == 1 {
                        mapping.remove("environment");
                    } else {
                        env.remove(variable.key.as_str()).ok_or(Error::Protected)?;
                    }
                }
                None
            }
            _ => return Err(Error::Protected),
        };
        let candidate = replacement.unwrap_or_else(|| file.to_string());
        let candidate = keep_line_endings(&self.source, candidate);
        if self.source == candidate {
            return Ok(());
        }
        let (views, has_include, resources) = inspect(&parse(&candidate)?)?;
        self.history.discard_future();
        self.drafts.push(Draft {
            yaml: candidate.clone(),
            change,
        });
        self.source = candidate;
        self.environment_usage.take();
        self.services = views;
        self.has_include = has_include;
        self.resources = resources;
        Ok(())
    }

    pub fn restore(&mut self, target: Target) -> Result<Option<String>> {
        let yaml = match target {
            Target::Draft(index) => self.drafts.get(index).map(|draft| draft.yaml.clone()),
            Target::Saved(index) => self
                .history
                .versions
                .get(index)
                .map(|version| version.yaml.clone()),
        }
        .ok_or(Error::Protected)?;
        let file = parse(&yaml)?;
        let (services, has_include, resources) = inspect(&file)?;
        let focus = match target {
            Target::Draft(index) => {
                self.drafts.truncate(index + 1);
                None
            }
            Target::Saved(index) => {
                let env = self.history.versions[index].env.clone();
                let current_env = self.history.current().ok_or(Error::Protected)?.env.clone();
                self.restore_files(&yaml, &env, &current_env)?;
                self.original.clone_from(&yaml);
                self.needs_formatting = needs_formatting(&file);
                let previous =
                    std::mem::replace(&mut self.environment, Environment::load(&self.path));
                let focus = self.restore_env_focus(index, &previous);
                self.drafts.clear();
                self.history.select(index);
                focus
            }
        };
        self.source = yaml;
        self.environment_usage.take();
        self.services = services;
        self.has_include = has_include;
        self.resources = resources;
        Ok(focus)
    }

    fn restore_env_focus(&self, index: usize, previous: &Environment) -> Option<String> {
        if let Change::Dotenv { key, .. } = &self.history.versions[index].change {
            return Some(key.clone());
        }
        let current = self.history.active_count() - 1;
        let from = current.min(index) + 1;
        let to = current.max(index) + 1;
        let changed = self.history.versions[from..to]
            .iter()
            .rev()
            .find_map(|version| {
                if let Change::Dotenv { key, .. } = &version.change
                    && self.environment.value(key) != previous.value(key)
                {
                    return Some(key.as_str());
                }
                None
            });
        changed
            .or_else(|| self.environment.changed_key(previous))
            .map(str::to_owned)
    }

    fn restore_files(
        &self,
        yaml: &str,
        env: &EnvSnapshot,
        current_env: &EnvSnapshot,
    ) -> Result<()> {
        if env != current_env
            && (matches!(env, EnvSnapshot::Unavailable)
                || matches!(current_env, EnvSnapshot::Unavailable))
        {
            return Err(Error::Input(
                "Die .env kann für diesen Stand nicht wiederhergestellt werden.",
                "The .env cannot be restored for this version.",
            ));
        }
        let check_env = || -> io::Result<()> {
            if &EnvSnapshot::read(&self.path) == current_env {
                Ok(())
            } else {
                Err(io::ErrorKind::AlreadyExists.into())
            }
        };
        let check = || {
            check_env()?;
            storage::check_contents(&self.path, Some(self.original.as_bytes()))
        };
        check().map_err(restore_error)?;
        let replace_yaml = self.original != yaml;
        if replace_yaml {
            storage::replace(&self.path, yaml.as_bytes(), check).map_err(restore_error)?;
        }
        if let Err(error) = env.restore(&self.path, current_env) {
            if replace_yaml
                && storage::replace(&self.path, self.original.as_bytes(), || {
                    storage::check_contents(&self.path, Some(yaml.as_bytes()))
                })
                .is_err()
            {
                return Err(Error::RestorePartial);
            }
            return Err(restore_error(error));
        }
        Ok(())
    }
}

fn remove_created(files: &[(PathBuf, Vec<u8>)]) {
    for (path, expected) in files.iter().rev() {
        if fs::read(path).ok().as_deref() == Some(expected.as_slice()) {
            let _ = fs::remove_file(path);
        }
    }
}

fn restore_error(error: io::Error) -> Error {
    match error.kind() {
        io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound => Error::RestoreConflict,
        _ => error.into(),
    }
}

fn parse(source: &str) -> Result<YamlFile> {
    if source.trim_start_matches('\u{feff}').trim().is_empty() {
        return Err(Error::Empty);
    }
    let file = YamlFile::parse(source)
        .to_result()
        .map_err(|_| Error::Yaml)?;
    match file.documents().count() {
        0 => Err(Error::Empty),
        1 => Ok(file),
        _ => Err(Error::Yaml),
    }
}

fn needs_formatting(file: &YamlFile) -> bool {
    file.document()
        .as_ref()
        .and_then(AsYaml::as_node)
        .is_some_and(yaml_format::needs_formatting)
}

fn unique_keys(mapping: &Mapping) -> Result<()> {
    let mut seen = HashSet::new();
    for (key, _) in mapping.iter() {
        if let Some(scalar) = key.as_scalar() {
            let name = scalar.as_string();
            if !seen.insert(name.clone()) {
                return Err(Error::Duplicate(name));
            }
        }
    }
    Ok(())
}

fn valid_restart_policy(value: &str) -> bool {
    matches!(value, "no" | "always" | "unless-stopped" | "on-failure")
        || value
            .strip_prefix("on-failure:")
            .is_some_and(|retries| retries.parse::<u32>().is_ok_and(|count| count > 0))
}

fn service_map(file: &YamlFile, name: &str) -> Result<Mapping> {
    file.document()
        .and_then(|doc| doc.as_mapping())
        .and_then(|root| root.get_mapping("services"))
        .and_then(|services| services.get_mapping(name))
        .ok_or(Error::Protected)
}

fn empty_yaml_value(node: Option<&YamlNode>) -> bool {
    node.is_none_or(|node| {
        node.as_scalar().is_some_and(|value| {
            matches!(value.value().as_str(), "" | "~" | "null" | "Null" | "NULL")
        })
    })
}

fn inspect(file: &YamlFile) -> Result<(Vec<Service>, bool, Resources)> {
    let document = file.document().ok_or(Error::Yaml)?;
    let root = document.as_mapping().ok_or(Error::Services)?;
    unique_keys(&root)?;
    let registry = AnchorRegistry::from_document(&document);
    let source = file.to_string();
    let resources = Resources::inspect(&root, &registry, &source);
    let node = root.get("services");
    if root.contains_key("services") && empty_yaml_value(node.as_ref()) {
        return Ok((
            Vec::new(),
            root.contains_key("include"),
            resources.with_usage(&[], root.contains_key("include") || has_merge(&root)),
        ));
    }
    let node = node.ok_or(Error::Services)?;
    let shared_services = node.is_alias();
    let resolved = resolve(&registry, node);
    let services = resolved.as_mapping().ok_or(Error::Services)?;
    unique_keys(services)?;
    let mut views = Vec::new();
    for entry in services.entries() {
        let name = entry
            .key_node()
            .and_then(|key| key.as_scalar().map(|key| key.as_string()))
            .filter(|name| !name.is_empty())
            .ok_or(Error::Services)?;
        let node = entry.value_node().ok_or(Error::Services)?;
        let read_only = shared_services || node.is_alias() || shared(&node, false);
        let resolved = resolve(&registry, node);
        let mapping = resolved.as_mapping().ok_or(Error::Services)?;
        unique_keys(mapping)?;
        let merged = MergedMapping::new(mapping.clone(), &registry).ok();
        let field = |key: &str| {
            mapping
                .get(key)
                .or_else(|| merged.as_ref()?.get(key))
                .map(|node| resolve(&registry, node))
        };
        let edit = |key: &str| {
            !read_only
                && !mapping.contains_key("extends")
                && !has_merge(mapping)
                && mapping.get(key).is_none_or(|node| !shared(&node, true))
        };
        let image = field("image");
        let restart = field("restart");
        let build_context = field("build").and_then(|node| {
            if let Some(map) = node.as_mapping() {
                Some(
                    map.get("context")
                        .map_or_else(|| ".".into(), |context| detail(&context)),
                )
            } else {
                node.is_scalar().then(|| detail(&node))
            }
        });
        let ports = field("ports");
        let variables = field("environment");
        let port_rows = ports
            .as_ref()
            .and_then(YamlNode::as_sequence)
            .map(|ports| {
                ports
                    .values()
                    .map(|node| port_view(resolve(&registry, node)))
                    .collect()
            })
            .unwrap_or_default();
        let env_rows = variables.as_ref().map(variable_view).unwrap_or_default();
        let distinct = env_rows
            .iter()
            .map(|var| &var.key)
            .collect::<HashSet<_>>()
            .len()
            == env_rows.len();
        let parameters =
            ServiceParameters::inspect(&source, mapping, &registry, &field, &edit, read_only);
        views.push(Service {
            name,
            image: image.as_ref().map(detail),
            restart: restart.as_ref().map(detail),
            build_context,
            image_editable: edit("image") && image.as_ref().is_none_or(YamlNode::is_scalar),
            restart_editable: edit("restart") && restart.as_ref().is_none_or(YamlNode::is_scalar),
            ports_editable: edit("ports")
                && ports.as_ref().is_none_or(|node| {
                    node.as_sequence().is_some_and(|seq| {
                        seq.values().all(|node| {
                            node.is_scalar()
                                || node.as_mapping().is_some_and(|map| {
                                    !has_merge(map)
                                        && unique_keys(map).is_ok()
                                        && ["target", "published", "host_ip", "protocol"]
                                            .into_iter()
                                            .all(|key| {
                                                map.get(key).is_none_or(|node| node.is_scalar())
                                            })
                                })
                        })
                    })
                }),
            variables_editable: edit("environment")
                && distinct
                && variables.as_ref().is_none_or(|node| {
                    node.as_mapping().is_some_and(|map| {
                        !has_merge(map)
                            && map
                                .iter()
                                .all(|(key, value)| key.is_scalar() && value.is_scalar())
                    }) || node
                        .as_sequence()
                        .is_some_and(|seq| seq.values().all(|node| node.is_scalar()))
                }),
            ports: port_rows,
            variables: env_rows,
            parameters,
        });
    }
    let resources = resources.with_usage(&views, root.contains_key("include") || has_merge(&root));
    Ok((views, root.contains_key("include"), resources))
}

fn resolve(registry: &AnchorRegistry, node: YamlNode) -> YamlNode {
    node.as_alias()
        .and_then(|alias| registry.resolve_alias(alias).ok())
        .unwrap_or(node)
}

fn has_merge(mapping: &Mapping) -> bool {
    mapping.as_node().is_some_and(|node| {
        node.children().any(|entry| {
            entry.children().any(|child| {
                child.kind() == SyntaxKind::KEY
                    && child
                        .descendants_with_tokens()
                        .any(|part| part.kind() == SyntaxKind::MERGE_KEY)
            })
        })
    })
}

fn shared(node: &YamlNode, descendants: bool) -> bool {
    let Some(syntax) = node.as_node() else {
        return true;
    };
    node.is_alias()
        || node.is_tagged()
        || syntax.ancestors().any(|parent| {
            parent
                .children_with_tokens()
                .any(|part| part.kind() == SyntaxKind::ANCHOR)
        })
        || (descendants
            && syntax.descendants_with_tokens().any(|part| {
                matches!(
                    part.kind(),
                    SyntaxKind::ANCHOR | SyntaxKind::ALIAS | SyntaxKind::TAGGED_NODE
                )
            }))
}

fn detail(node: &YamlNode) -> String {
    node.as_scalar().map_or_else(
        || node.to_string().trim().to_owned(),
        |scalar| scalar.as_string(),
    )
}

fn port_view(node: YamlNode) -> Port {
    if let Some(mapping) = node.as_mapping() {
        let read = |key| {
            mapping
                .get(key)
                .map(|value| detail(&value))
                .unwrap_or_default()
        };
        Port::Long(LongPort {
            target: read("target"),
            published: read("published"),
            host_ip: read("host_ip"),
            protocol: read("protocol"),
        })
    } else if node.is_scalar() {
        Port::Short(detail(&node))
    } else {
        Port::Other(detail(&node))
    }
}

fn collect_environment_usage(
    node: &YamlNode,
    registry: &AnchorRegistry,
    usage: &mut EnvironmentUsage,
) {
    if let Some(scalar) = node.as_scalar() {
        usage.names.extend(
            environment::references(&scalar.as_string())
                .into_iter()
                .map(str::to_owned),
        );
    } else if let Some(sequence) = node.as_sequence() {
        for value in sequence.values() {
            collect_environment_usage(&value, registry, usage);
        }
    } else if let Some(mapping) = node.as_mapping() {
        for entry in mapping.entries() {
            let Some(value) = entry.value_node() else {
                continue;
            };
            let key = entry
                .key_node()
                .and_then(|node| node.as_scalar().map(|key| key.as_string()));
            match key.as_deref() {
                Some("env_file") => collect_env_file_paths(
                    resolve(registry, value.clone()),
                    registry,
                    &mut usage.env_files,
                ),
                Some("environment") => {
                    collect_forwarded_variables(&resolve(registry, value.clone()), usage);
                }
                Some("build") => {
                    if let Some(args) = resolve(registry, value.clone())
                        .as_mapping()
                        .and_then(|build| build.get("args"))
                    {
                        collect_forwarded_variables(&resolve(registry, args), usage);
                    }
                }
                _ => {}
            }
            collect_environment_usage(&value, registry, usage);
        }
    }
}

fn collect_forwarded_variables(node: &YamlNode, usage: &mut EnvironmentUsage) {
    usage.names.extend(
        variable_view(node)
            .into_iter()
            .filter(|variable| variable.value.is_none())
            .map(|variable| variable.key),
    );
}

fn collect_env_file_paths(node: YamlNode, registry: &AnchorRegistry, paths: &mut Vec<String>) {
    let mut add_path = |node: YamlNode| {
        let path = node
            .as_mapping()
            .and_then(|mapping| mapping.get("path"))
            .map_or(node, |path| resolve(registry, path));
        if let Some(scalar) = path.as_scalar() {
            paths.push(scalar.as_string());
        }
    };
    if let Some(sequence) = node.as_sequence() {
        for value in sequence.values() {
            add_path(resolve(registry, value));
        }
    } else {
        add_path(node);
    }
}

fn variable_view(node: &YamlNode) -> Vec<Variable> {
    if let Some(map) = node.as_mapping() {
        map.entries()
            .filter_map(|entry| {
                let key = detail(&entry.key_node()?);
                let value = entry.value_node().and_then(|value| {
                    if value.yaml_eq(&ScalarValue::null()) {
                        None
                    } else {
                        Some(detail(&value))
                    }
                });
                Some(Variable { key, value })
            })
            .collect()
    } else if let Some(seq) = node.as_sequence() {
        seq.values()
            .map(|node| {
                let text = detail(&node);
                match text.split_once('=') {
                    Some((key, value)) => Variable {
                        key: key.into(),
                        value: Some(value.into()),
                    },
                    None => Variable {
                        key: text,
                        value: None,
                    },
                }
            })
            .collect()
    } else {
        vec![Variable {
            key: detail(node),
            value: None,
        }]
    }
}

fn string(value: String) -> ScalarValue {
    // Explicit quoting keeps Compose values such as "on", "" and "8080" strings.
    ScalarValue::double_quoted(value)
}

fn scalar_patch(source: &str, node: &YamlNode, replacement: &str) -> String {
    let mut source = source.to_owned();
    let range = node.as_node().expect("CST node").text_range();
    source.replace_range(
        usize::from(range.start())..usize::from(range.end()),
        replacement,
    );
    source
}

fn keep_line_endings(original: &str, candidate: String) -> String {
    if original.contains("\r\n") && original.replace("\r\n", "").find('\n').is_none() {
        candidate.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        candidate
    }
}

fn set_port(
    source: &str,
    service: &Mapping,
    index: Option<usize>,
    port: Port,
) -> Result<Option<String>> {
    match port {
        Port::Short(value) => {
            let value = value.trim();
            if value.is_empty() || value.chars().any(char::is_control) {
                return Err(Error::Input(
                    "Gib eine Port-Zuordnung ein.",
                    "Enter a port mapping.",
                ));
            }
            let value = string(value.into());
            if let Some(index) = index {
                let node = service
                    .get_sequence("ports")
                    .and_then(|seq| seq.get(index))
                    .ok_or(Error::Protected)?;
                if !node.is_scalar() {
                    return Err(Error::Protected);
                }
                if detail(&node) == value.value() {
                    return Ok(None);
                }
                return Ok(Some(scalar_patch(source, &node, &value.to_yaml_string())));
            }
            if let Some(sequence) = service.get_sequence("ports") {
                sequence.push(value);
            } else {
                let sequence = Sequence::new_pending_block();
                sequence.push(value);
                service.set("ports", sequence);
            }
        }
        Port::Long(port) => {
            let target = port.target.trim();
            let number = target.parse::<u16>().ok().filter(|value| *value > 0);
            if number.is_none() && !target.contains('$') {
                return Err(Error::Input(
                    "Der Container-Port muss zwischen 1 und 65535 liegen oder einen Compose-Ausdruck enthalten.",
                    "The container port must be between 1 and 65535 or contain a Compose expression.",
                ));
            }
            let existing = index
                .map(|index| {
                    service
                        .get_sequence("ports")
                        .and_then(|seq| seq.get(index))
                        .and_then(|node| node.as_mapping().cloned())
                        .ok_or(Error::Protected)
                })
                .transpose()?;
            let map = existing.clone().unwrap_or_else(Mapping::new_pending_block);
            if map.get("target").is_none_or(|node| detail(&node) != target) {
                if let Some(number) = number {
                    map.set("target", i64::from(number));
                } else {
                    map.set("target", string(target.into()));
                }
            }
            for (key, value) in [
                ("published", port.published),
                ("host_ip", port.host_ip),
                ("protocol", port.protocol),
            ] {
                let value = value.trim();
                if value.is_empty() {
                    if map.get(key).is_some_and(|node| !detail(&node).is_empty()) {
                        map.remove(key);
                    }
                } else if map.get(key).is_none_or(|node| detail(&node) != value) {
                    map.set(key, string(value.into()));
                }
            }
            if existing.is_none() {
                if let Some(sequence) = service.get_sequence("ports") {
                    sequence.push(map);
                } else {
                    let sequence = Sequence::new_pending_block();
                    sequence.push(map);
                    service.set("ports", sequence);
                }
            }
        }
        Port::Other(_) => return Err(Error::Protected),
    }
    Ok(None)
}

fn set_variable(
    source: &str,
    service: &Mapping,
    view: &Service,
    index: Option<usize>,
    variable: Variable,
) -> Result<Option<String>> {
    let rows = &view.variables;
    let old = index
        .map(|index| rows.get(index).ok_or(Error::Protected))
        .transpose()?;
    if old == Some(&variable) {
        return Ok(None);
    }
    let key = variable.key.as_str();
    if key.trim().is_empty() || key.contains('=') || key.chars().any(char::is_control) {
        return Err(Error::Input(
            "Gib einen Variablennamen ohne Gleichheitszeichen ein.",
            "Enter a variable name without an equals sign.",
        ));
    }
    if rows
        .iter()
        .enumerate()
        .any(|(row, other)| Some(row) != index && other.key == key)
    {
        return Err(Error::Input(
            "Diese Variable ist bereits vorhanden.",
            "This variable already exists.",
        ));
    }
    if let Some(sequence) = service.get_sequence("environment") {
        let content = match variable.value {
            Some(value) => format!("{key}={value}"),
            None => key.into(),
        };
        let value = string(content);
        if let Some(index) = index {
            let node = sequence.get(index).ok_or(Error::Protected)?;
            return Ok(Some(scalar_patch(source, &node, &value.to_yaml_string())));
        }
        sequence.push(value);
        return Ok(None);
    }
    let value = variable.value.map_or_else(ScalarValue::null, string);
    if let Some(map) = service.get_mapping("environment") {
        if let Some(old) = old.filter(|old| old.key != key) {
            let key_node = map
                .entries()
                .find(|entry| entry.key_matches(old.key.as_str()))
                .and_then(|entry| entry.key_node())
                .ok_or(Error::Protected)?;
            // Rename only the key so the row keeps its original position and comment.
            let new_source = scalar_patch(source, &key_node, &string(key.into()).to_yaml_string());
            if old.value == variable_view_value(&value) {
                return Ok(Some(new_source));
            }
            let file = parse(&new_source)?;
            service_map(&file, &view.name)?
                .get_mapping("environment")
                .ok_or(Error::Protected)?
                .set(key, value);
            return Ok(Some(file.to_string()));
        }
        map.set(key, value);
    } else {
        let map = Mapping::new_pending_block();
        map.set(key, value);
        service.set("environment", map);
    }
    Ok(None)
}

fn variable_view_value(value: &ScalarValue) -> Option<String> {
    (value.scalar_type() != yaml_edit::ScalarType::Null).then(|| value.value().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../examples/compose.yaml");

    fn fixture(source: &str) -> (tempfile::TempDir, Compose) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compose.yaml");
        fs::write(&path, source).unwrap();
        let document = Compose::open(&path).unwrap();
        (dir, document)
    }

    fn variable(key: &str, value: Option<&str>) -> Variable {
        Variable {
            key: key.into(),
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn finds_unused_dotenv_entries_across_compose_and_dotenv_references() {
        let (folder, mut document) = fixture(concat!(
            "services:\n",
            "  app:\n",
            "    image: 'nginx:$TAG' # $COMMENT\n",
            "    command: '$$ESCAPED'\n",
            "    environment: [INHERITED, FIXED=value, 'FROM_ENV=$ALIAS']\n",
            "    build:\n",
            "      context: .\n",
            "      args: {BUILD_ARG: null}\n",
            "    labels: {'$KEY_ONLY': value}\n",
        ));
        let path = folder.path().join(".env");
        fs::write(
            &path,
            concat!(
                "TAG=alpine\nINHERITED=x\nFIXED=x\n",
                "BASE=x\nALIAS=$BASE\nUNUSED_REF=$ORPHAN\nORPHAN=x\n",
                "COMMENT=x\nESCAPED=x\nKEY_ONLY=x\nBUILD_ARG=x\nCOMPOSE_PROFILES=dev\n",
            ),
        )
        .unwrap();
        document.reload_environment();
        assert_eq!(
            document.unused_environment_variables(),
            HashSet::from([
                "COMMENT",
                "ESCAPED",
                "FIXED",
                "KEY_ONLY",
                "ORPHAN",
                "UNUSED_REF"
            ]),
        );

        document
            .edit(0, Edit::Image(Some("nginx:$UNUSED_REF".into())))
            .unwrap();
        let unused = document.unused_environment_variables();
        assert!(unused.contains("TAG"));
        assert!(!unused.contains("ORPHAN"));

        let (folder, mut document) =
            fixture("services:\n  app:\n    env_file:\n      - path: ${ENV_PATH:-./.env}\n");
        fs::write(folder.path().join(".env"), "IMPORTED=demo\n").unwrap();
        document.reload_environment();
        assert!(document.unused_environment_variables().is_empty());
    }

    #[test]
    fn reads_cards_and_environment_forms_in_source_order() {
        let (_temp, doc) = fixture(EXAMPLE);
        assert_eq!(
            doc.services
                .iter()
                .map(|service| service.name.as_str())
                .collect::<Vec<_>>(),
            ["gateway", "cache", "worker"]
        );
        assert_eq!(
            doc.services[0].variables,
            [
                variable("LOG_LEVEL", Some("${LOG_LEVEL:-info}")),
                variable("EMPTY", Some("")),
                variable("PROXY_MODE", None),
            ]
        );
        assert_eq!(
            doc.services[1].variables,
            [
                variable("CACHE_MODE", Some("local")),
                variable("CACHE_TAG", Some("")),
                variable("CACHE_TRACE", None),
            ]
        );
        assert!(doc.services[0].variables_editable);
        assert!(doc.services[1].variables_editable);
        assert!(matches!(doc.services[0].ports[1], Port::Long(_)));
        assert!(
            doc.services[2]
                .parameters
                .additional
                .iter()
                .any(|field| field.key == "build")
        );
    }

    #[test]
    fn updates_image_and_each_port_form_while_preserving_unrelated_yaml() {
        let (_temp, mut doc) = fixture(EXAMPLE);
        doc.edit(0, Edit::Image(Some("nginx:${TAG:-alpine}".into())))
            .unwrap();
        doc.edit(
            0,
            Edit::Port(Some(0), Port::Short("127.0.0.1:8181:80".into())),
        )
        .unwrap();
        let Port::Long(mut port) = doc.services[0].ports[1].clone() else {
            panic!("long port expected")
        };
        port.published = "9180".into();
        doc.edit(0, Edit::Port(Some(1), Port::Long(port))).unwrap();
        assert_eq!(
            &doc.source[doc.source.find("  cache:\n").unwrap()..],
            &EXAMPLE[EXAMPLE.find("  cache:\n").unwrap()..]
        );
        assert!(doc.source.contains("# reverse proxy"));
        assert!(doc.source.contains("# local development"));
        assert!(doc.source.contains("name: metrics"));
        assert!(doc.source.contains("${LOG_LEVEL:-info}"));
        doc.save(doc.path.clone()).unwrap();
        assert!(!doc.dirty());
        let reopened = Compose::open(&doc.path).unwrap();
        assert_eq!(
            reopened.services[0].image.as_deref(),
            Some("nginx:${TAG:-alpine}")
        );
        assert_eq!(
            reopened.services[0].ports[0],
            Port::Short("127.0.0.1:8181:80".into())
        );
    }

    #[test]
    fn adds_edits_and_removes_environment_rows_in_both_styles() {
        let (_temp, mut doc) = fixture(EXAMPLE);
        doc.edit(
            0,
            Edit::Variable(Some(0), variable("LEVEL", Some("hi # there"))),
        )
        .unwrap();
        doc.edit(0, Edit::Variable(Some(1), variable("EMPTY", None)))
            .unwrap();
        doc.edit(0, Edit::Variable(None, variable("EXTRA", Some("on"))))
            .unwrap();
        doc.edit(0, Edit::RemoveVariable(2)).unwrap();
        assert_eq!(
            doc.services[0].variables,
            [
                variable("LEVEL", Some("hi # there")),
                variable("EMPTY", None),
                variable("EXTRA", Some("on"))
            ]
        );
        doc.edit(
            1,
            Edit::Variable(Some(0), variable("CACHE_MODE", Some("test=one\nline two"))),
        )
        .unwrap();
        doc.edit(1, Edit::Variable(None, variable("EXTRA", Some(""))))
            .unwrap();
        doc.edit(1, Edit::RemoveVariable(2)).unwrap();
        assert_eq!(
            doc.services[1].variables,
            [
                variable("CACHE_MODE", Some("test=one\nline two")),
                variable("CACHE_TAG", Some("")),
                variable("EXTRA", Some("")),
            ]
        );
        assert!(doc.source.contains("- \"EXTRA=\""));
        assert!(doc.source.contains("EXTRA: \"on\""));
    }

    #[test]
    fn edits_empty_sections_and_removes_last_entries() {
        let (_temp, mut doc) = fixture(
            "services:\n  app:\n    image: nginx\n  worker: {image: redis, environment: {}, ports: []}\n",
        );
        for service in 0..2 {
            doc.edit(
                service,
                Edit::Port(
                    None,
                    Port::Long(LongPort {
                        target: "80".into(),
                        host_ip: "127.0.0.1".into(),
                        ..Default::default()
                    }),
                ),
            )
            .unwrap();
            assert_eq!(doc.services[service].ports[0].summary(), "127.0.0.1:80");
            doc.edit(service, Edit::RemovePort(0)).unwrap();
            assert!(doc.services[service].ports.is_empty());

            doc.edit(service, Edit::Variable(None, variable("TERM", None)))
                .unwrap();
            assert_eq!(doc.services[service].variables, [variable("TERM", None)]);
            doc.edit(service, Edit::RemoveVariable(0)).unwrap();
            assert!(doc.services[service].variables.is_empty());
        }
    }

    #[test]
    fn resolves_merges_and_aliases_and_protects_shared_values() {
        let (_temp, mut doc) = fixture(concat!(
            "x-base: &base\n  image: nginx\n",
            "services:\n",
            "  first: {<<: *base}\n",
            "  second: *base\n",
            "  third:\n    image: &shared redis\n    environment: [X=1]\n",
            "  fourth:\n    image: *shared\n    ports: [\"80\"]\n",
            "  fifth: &service\n    image: alpine\n",
            "  sixth: *service\n",
        ));
        for service in 0..6 {
            assert!(!doc.services[service].image_editable, "service {service}");
            assert!(matches!(
                doc.edit(service, Edit::Image(Some("other".into()))),
                Err(Error::Protected)
            ));
        }
        assert_eq!(doc.services[0].image.as_deref(), Some("nginx"));
        assert_eq!(doc.services[3].image.as_deref(), Some("redis"));
        assert!(doc.services[2].variables_editable);
        assert!(doc.services[3].ports_editable);
        assert!(!doc.dirty());
    }

    #[test]
    fn preserves_crlf_and_conflicting_external_changes() {
        let source = EXAMPLE.replace('\n', "\r\n");
        let (dir, mut doc) = fixture(&source);
        doc.edit(0, Edit::Image(Some("nginx:alpine".into())))
            .unwrap();
        assert!(!doc.source.replace("\r\n", "").contains('\n'));
        fs::write(&doc.path, "changed externally").unwrap();
        assert!(matches!(doc.save(doc.path.clone()), Err(Error::Conflict)));
        assert_eq!(fs::read_to_string(&doc.path).unwrap(), "changed externally");
        assert!(doc.dirty());
        let copy = dir.path().join("copy.yaml");
        doc.save(copy.clone()).unwrap();
        assert!(!doc.dirty());
        assert!(fs::read_to_string(copy).unwrap().contains("nginx:alpine"));
    }

    #[test]
    fn saves_mixed_collections_as_indented_yaml() {
        let source = concat!(
            "# Compose example\n",
            "x-defaults: &defaults {image: nginx, init: true}\n",
            "services:\n",
            "  app: {<<: *defaults, environment: {MODE: '${MODE:-dev}'}, # note\n",
            "    ports: [{target: 80, published: '8080'}]} # app\n",
            "  worker:\n",
            "    image: redis\n",
            "    command: [redis-server, --appendonly, 'yes']\n",
            "volumes: {data: {}}\n",
        )
        .replace('\n', "\r\n");
        let (_dir, mut doc) = fixture(&source);
        assert!(doc.can_save());
        assert!(!doc.dirty());
        doc.save(doc.path.clone()).unwrap();
        let saved = fs::read_to_string(&doc.path).unwrap();
        assert!(!saved.replace("\r\n", "").contains('\n'));
        for expected in [
            "# Compose example",
            "x-defaults: &defaults\r\n  image: nginx",
            "  app:\r\n    <<: *defaults",
            "    environment:\r\n      MODE: '${MODE:-dev}'",
            "    # note",
            "        target: 80\r\n        published: '8080'",
            "# app",
            "    command:\r\n      - redis-server\r\n      - --appendonly\r\n      - 'yes'",
            "volumes:\r\n  data: {}",
        ] {
            assert!(
                saved.contains(expected),
                "Missing {expected:?} in {saved:?}"
            );
        }
        assert!(!doc.can_save());
        assert_eq!(doc.history.current().unwrap().yaml, saved);
        let reopened = Compose::open(&doc.path).unwrap();
        assert!(!reopened.can_save());
        assert_eq!(reopened.services[0].image.as_deref(), Some("nginx"));
        assert_eq!(
            reopened.services[0].variables,
            [variable("MODE", Some("${MODE:-dev}"))]
        );
        assert_eq!(reopened.services[0].ports[0].summary(), "8080 → 80");
    }

    #[test]
    fn saves_a_flow_document_as_a_block_copy_and_records_the_file_contents() {
        let source = "--- {services: {app: {image: nginx, environment: {MODE: dev}}}}\n";
        let (dir, mut doc) = fixture(source);
        let original = doc.path.clone();
        doc.edit(0, Edit::Variable(Some(0), variable("MODE", Some("prod"))))
            .unwrap();
        let copy = dir.path().join("copy.yml");
        doc.save(copy.clone()).unwrap();
        let saved = fs::read_to_string(copy).unwrap();
        assert_eq!(fs::read_to_string(original).unwrap(), source);
        assert_eq!(
            saved,
            "---\nservices:\n  app:\n    image: nginx\n    environment:\n      MODE: \"prod\"\n"
        );
        assert_eq!(doc.history.current().unwrap().yaml, saved);
        assert!(!doc.can_save());
    }

    #[test]
    fn rejects_bad_documents() {
        for source in ["", " \t\r\n", "\u{feff}"] {
            assert!(matches!(parse(source), Err(Error::Empty)));
        }
        for source in [
            "services: [broken",
            "services: []",
            "services: {}\n---\nservices: {}\n",
        ] {
            let result = parse(source).and_then(|file| inspect(&file));
            assert!(result.is_err(), "{source}");
        }
        let file = parse("services:\n  same: {image: nginx}\n  'same': {image: redis}\n").unwrap();
        assert!(matches!(inspect(&file), Err(Error::Duplicate(name)) if name == "same"));
    }

    #[test]
    fn saves_each_step_and_restores_both_files_across_sessions() {
        let (dir, mut document) = fixture(EXAMPLE);
        let path = document.path.clone();
        let dotenv = dir.path().join(".env");
        let sidecar = dir.path().join(".compose.yaml.compeditor-history.json");
        assert!(!sidecar.exists());
        document.save_env_value("LOG_LEVEL", "debug").unwrap();
        document
            .edit(0, Edit::Image(Some("nginx:latest".into())))
            .unwrap();
        document.save(path.clone()).unwrap();
        document.save_env_value("LOG_LEVEL", "warn").unwrap();
        document
            .edit(
                0,
                Edit::Port(Some(0), Port::Short("127.0.0.1:9090:80".into())),
            )
            .unwrap();
        document
            .edit(1, Edit::Image(Some("redis:latest".into())))
            .unwrap();
        document.save(path.clone()).unwrap();
        assert_eq!(document.history.versions.len(), 6);
        assert!(sidecar.is_file());

        let mut reopened = Compose::open(&path).unwrap();
        assert_eq!(reopened.history.versions.len(), 6);
        assert_eq!(
            reopened.restore(Target::Saved(1)).unwrap().as_deref(),
            Some("LOG_LEVEL")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), EXAMPLE);
        assert_eq!(
            fs::read_to_string(&dotenv).unwrap(),
            "LOG_LEVEL=\"debug\"\n"
        );
        assert_eq!(reopened.history.active_count(), 2);
        assert_eq!(reopened.history.versions.len(), 6);
        assert_eq!(Compose::open(&path).unwrap().history.versions.len(), 2);

        reopened.restore(Target::Saved(5)).unwrap();
        assert_eq!(reopened.history.active_count(), 6);
        assert_eq!(Compose::open(&path).unwrap().history.versions.len(), 6);
        assert_eq!(
            reopened.restore(Target::Saved(0)).unwrap().as_deref(),
            Some("LOG_LEVEL")
        );
        assert!(!dotenv.exists());
        assert!(matches!(reopened.environment, Environment::Missing));
        assert!(!reopened.dirty());
        assert_eq!(Compose::open(&path).unwrap().history.versions.len(), 1);
    }

    #[test]
    fn editing_an_older_version_replaces_the_later_history() {
        let (_dir, mut document) = fixture(EXAMPLE);
        let path = document.path.clone();
        for step in 1..=5 {
            document
                .edit(0, Edit::Image(Some(format!("nginx:step{step}"))))
                .unwrap();
            document.save(path.clone()).unwrap();
        }
        document.restore(Target::Saved(2)).unwrap();
        assert_eq!(document.history.versions.len(), 6);
        document
            .edit(0, Edit::Image(Some("nginx:step2".into())))
            .unwrap();
        assert_eq!(document.history.versions.len(), 6);
        document
            .edit(0, Edit::Image(Some("nginx:new".into())))
            .unwrap();
        assert_eq!(document.history.versions.len(), 3);
        assert_eq!(document.drafts.len(), 1);
        document.save(path.clone()).unwrap();
        let reopened = Compose::open(&path).unwrap();
        assert_eq!(reopened.history.versions.len(), 4);
        assert!(
            reopened
                .history
                .current()
                .unwrap()
                .yaml
                .contains("nginx:new")
        );
        assert!(
            reopened
                .history
                .versions
                .iter()
                .all(|version| !version.yaml.contains("nginx:step3"))
        );
    }

    #[test]
    fn saving_a_dotenv_value_also_replaces_the_later_history() {
        let (_dir, mut document) = fixture(EXAMPLE);
        let path = document.path.clone();
        for value in ["first", "second", "third"] {
            document.save_env_value("LOG_LEVEL", value).unwrap();
        }
        document.restore(Target::Saved(1)).unwrap();
        document.save_env_value("LOG_LEVEL", "new").unwrap();
        assert_eq!(document.history.versions.len(), 3);
        let reopened = Compose::open(&path).unwrap();
        assert_eq!(reopened.history.versions.len(), 3);
        assert_eq!(reopened.environment.value("LOG_LEVEL").unwrap().text, "new");
    }

    #[test]
    fn restores_a_draft_without_writing_or_committing_later_drafts() {
        let (_dir, mut document) = fixture(EXAMPLE);
        document
            .edit(0, Edit::Image(Some("nginx:one".into())))
            .unwrap();
        document
            .edit(0, Edit::Image(Some("nginx:two".into())))
            .unwrap();
        assert!(document.restore(Target::Draft(0)).unwrap().is_none());
        assert!(document.dirty());
        assert_eq!(document.drafts.len(), 1);
        assert_eq!(document.services[0].image.as_deref(), Some("nginx:one"));
        assert_eq!(fs::read_to_string(&document.path).unwrap(), EXAMPLE);
        document.save(document.path.clone()).unwrap();
        assert_eq!(document.history.versions.len(), 2);
        assert!(
            document
                .history
                .versions
                .last()
                .unwrap()
                .yaml
                .contains("nginx:one")
        );
    }

    #[test]
    fn restore_blocks_external_edits_to_either_file() {
        let (dir, mut document) = fixture(EXAMPLE);
        let dotenv = dir.path().join(".env");
        document.save_env_value("LOG_LEVEL", "info").unwrap();
        document
            .edit(0, Edit::Image(Some("nginx:other".into())))
            .unwrap();
        document.save(document.path.clone()).unwrap();
        let current = fs::read_to_string(&document.path).unwrap();

        fs::write(&dotenv, "LOG_LEVEL=external\n").unwrap();
        assert!(matches!(
            document.restore(Target::Saved(0)),
            Err(Error::RestoreConflict)
        ));
        assert_eq!(fs::read_to_string(&document.path).unwrap(), current);
        document.reload_environment();
        fs::write(&document.path, EXAMPLE).unwrap();
        assert!(matches!(
            document.restore(Target::Saved(0)),
            Err(Error::RestoreConflict)
        ));
        assert_eq!(fs::read_to_string(&dotenv).unwrap(), "LOG_LEVEL=external\n");
    }

    #[test]
    fn keeps_unreadable_history_file_when_saving_yaml() {
        let (dir, document) = fixture(EXAMPLE);
        let sidecar = dir.path().join(".compose.yaml.compeditor-history.json");
        fs::write(&sidecar, "unreadable history").unwrap();
        let mut reopened = Compose::open(&document.path).unwrap();
        reopened
            .edit(0, Edit::Image(Some("nginx:other".into())))
            .unwrap();
        reopened.save(reopened.path.clone()).unwrap();
        assert_eq!(
            reopened.history.notice,
            Some(crate::history::Notice::Unreadable)
        );
        assert_eq!(fs::read_to_string(sidecar).unwrap(), "unreadable history");
    }

    #[test]
    fn creates_new_compose_env_and_initial_history_in_selected_folder() {
        let directory = tempfile::tempdir().unwrap();
        let document = Compose::create_in(directory.path()).unwrap();
        assert_eq!(document.path.file_name().unwrap(), "docker-compose.yml");
        assert_eq!(fs::read_to_string(&document.path).unwrap(), "services:\n");
        assert_eq!(fs::read(directory.path().join(".env")).unwrap(), b"");
        assert!(History::sidecar_path(&document.path).exists());
        assert!(document.services.is_empty());
    }

    #[test]
    fn refuses_new_compose_when_any_reserved_file_exists() {
        let reserved = [
            "docker-compose.yml",
            ".env",
            ".docker-compose.yml.compeditor-history.json",
        ];
        for existing in reserved {
            let directory = tempfile::tempdir().unwrap();
            fs::write(directory.path().join(existing), b"existing").unwrap();
            assert!(matches!(
                Compose::create_in(directory.path()),
                Err(Error::ProjectExists(_))
            ));
            for filename in reserved {
                assert_eq!(
                    directory.path().join(filename).exists(),
                    filename == existing
                );
            }
        }
    }

    #[test]
    fn adds_service_to_empty_and_existing_service_mappings() {
        let (_empty_dir, mut empty) = fixture("services: {}\n");
        let index = empty.add_service("api", Some("nginx:1.27")).unwrap();
        assert_eq!(index, 0);
        assert_eq!(empty.services[index].name, "api");
        assert_eq!(empty.services[index].image.as_deref(), Some("nginx:1.27"));
        assert!(empty.dirty());
        assert!(
            empty.source.starts_with("services:\n  api:\n    image:")
                && empty.source.contains("nginx:1.27")
        );
        assert!(matches!(
            empty.drafts.last().map(|draft| &draft.change),
            Some(Change::ServiceAdded { service }) if service == "api"
        ));

        let (_existing_dir, mut existing) = fixture("services:\n  web:\n    image: nginx\n");
        existing.add_service("cache", None).unwrap();
        assert_eq!(existing.services.len(), 2);
        assert_eq!(existing.services[1].name, "cache");
        assert!(existing.services[1].image.is_none());
        assert!(existing.source.contains("image: nginx"));

        let (_inline_dir, mut inline) =
            fixture("services:\n  mariadb: {image: \"mariadb:latest\"}\n");
        inline.add_service("cache", Some("redis:alpine")).unwrap();
        assert!(inline.source.contains("services:\n  mariadb:\n    image:"));
        assert!(inline.source.contains("  cache:\n    image:"));
        assert!(!inline.source.contains('{'));
    }

    #[test]
    fn writes_new_image_parameters_as_indented_yaml() {
        let (_temp, mut doc) = fixture("services:\n");
        let index = doc
            .add_service("app", Some("example/image:latest"))
            .unwrap();
        doc.edit(
            index,
            Edit::Variable(None, variable("APP_MODE", Some("production"))),
        )
        .unwrap();
        assert!(doc.source.contains("services:\n  app:\n    image:"));
        assert!(
            doc.source
                .contains("    environment:\n      APP_MODE: \"production\""),
            "generated YAML: {:?}",
            doc.source
        );
        assert!(!doc.source.contains("{APP_MODE:"));

        let (_temp, mut flow) = fixture(
            "services:\n  app:\n    image: example/image\n    environment: {APP_MODE: development, DEBUG: 'false'}\n",
        );
        flow.edit(
            0,
            Edit::Variable(Some(0), variable("APP_MODE", Some("production"))),
        )
        .unwrap();
        assert!(
            flow.source
                .contains("environment:\n      APP_MODE: \"production\"\n      DEBUG: 'false'")
        );
    }

    #[test]
    fn rejects_duplicate_or_invalid_service_names() {
        let (_dir, mut document) = fixture("services:\n  web:\n    image: nginx\n");
        assert!(matches!(
            document.add_service("web", None),
            Err(Error::Input(_, _))
        ));
        assert!(matches!(
            document.add_service("not valid", None),
            Err(Error::Input(_, _))
        ));
        assert_eq!(document.services.len(), 1);
    }
}
