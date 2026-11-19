use crate::storage::{self, Language};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvSnapshot {
    File(Vec<u8>),
    Missing,
    Unavailable,
}

impl EnvSnapshot {
    pub fn read(compose_path: &Path) -> Self {
        let path = compose_path.with_file_name(".env");
        match storage::read_optional(&path) {
            Ok(Some(bytes)) => Self::File(bytes),
            Ok(None) if !path.is_symlink() => Self::Missing,
            _ => Self::Unavailable,
        }
    }

    pub fn restore(&self, compose_path: &Path, expected: &Self) -> io::Result<()> {
        if self == expected {
            return Ok(());
        }
        let path = compose_path.with_file_name(".env");
        let check = || -> io::Result<()> {
            if &Self::read(compose_path) == expected {
                Ok(())
            } else {
                Err(io::ErrorKind::AlreadyExists.into())
            }
        };
        match (self, expected) {
            (Self::File(bytes), Self::Missing) => storage::create(&path, bytes),
            (Self::File(bytes), Self::File(_)) => {
                let target = fs::canonicalize(&path)?;
                storage::replace(&target, bytes, || {
                    check()?;
                    if fs::canonicalize(&path)? != target {
                        return Err(io::ErrorKind::AlreadyExists.into());
                    }
                    Ok(())
                })
            }
            (Self::Missing, Self::File(_)) => {
                check()?;
                fs::remove_file(path)
            }
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    Initial,
    External,
    SavedAs,
    Formatted,
    ServiceAdded {
        service: String,
    },
    Image {
        service: String,
        before: Option<String>,
        after: Option<String>,
    },
    RestartPolicy {
        service: String,
        before: Option<String>,
        after: Option<String>,
    },
    Ports {
        service: String,
        before: Option<String>,
        after: Option<String>,
    },
    Variable {
        service: String,
        key: String,
        operation: Operation,
    },
    Dotenv {
        key: String,
        operation: Operation,
    },
    ComposeParameter {
        owner: String,
        key: String,
        operation: Operation,
    },
    ResourceAssigned {
        section: String,
        resource: String,
        service: String,
    },
    Restored(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Added,
    Updated,
    Removed,
}

impl Operation {
    fn title(&self, language: Language) -> &'static str {
        match self {
            Self::Added => language.text("Hinzugefügt", "Added"),
            Self::Updated => language.text("Geändert", "Changed"),
            Self::Removed => language.text("Entfernt", "Removed"),
        }
    }
}

impl Change {
    pub fn title(&self, language: Language) -> String {
        let t = |de, en| language.text(de, en);
        match self {
            Self::Initial => t("Ausgangsstand", "Initial state").into(),
            Self::External => t("Externe Änderungen", "External changes").into(),
            Self::SavedAs => t("Unter neuem Namen gespeichert", "Saved under a new name").into(),
            Self::Formatted => t("YAML formatiert", "Formatted YAML").into(),
            Self::ServiceAdded { service } => {
                format!("{service} · {}", t("Service hinzugefügt", "Service added"))
            }
            Self::Image { service, .. } => format!("{service} · Image"),
            Self::RestartPolicy { service, .. } => {
                format!("{service} · {}", t("Neustart", "Restart"))
            }
            Self::Ports { service, .. } => format!("{service} · Ports"),
            Self::Variable {
                service,
                key,
                operation,
            } => format!("{service} · {key} · {}", operation.title(language)),
            Self::Dotenv { key, operation } => {
                format!(".env · {key} · {}", operation.title(language))
            }
            Self::ComposeParameter {
                owner,
                key,
                operation,
            } => {
                let owner = match owner.as_str() {
                    "networks:" => t("Netzwerke", "Networks"),
                    "volumes:" => t("Volumes", "Volumes"),
                    _ => owner.as_str(),
                };
                format!("{owner} · {key} · {}", operation.title(language))
            }
            Self::ResourceAssigned {
                section,
                resource,
                service,
            } => {
                let section = if section == "networks" {
                    t("Netzwerke", "Networks")
                } else {
                    "Volumes"
                };
                format!(
                    "{service} · {section} · {resource} · {}",
                    t("Angelegt und zugewiesen", "Created and assigned")
                )
            }
            Self::Restored(number) => format!("{} #{number}", t("Wiederhergestellt", "Restored")),
        }
    }

    pub fn detail(&self, language: Language) -> Option<String> {
        let (before, after) = match self {
            Self::Image { before, after, .. }
            | Self::RestartPolicy { before, after, .. }
            | Self::Ports { before, after, .. } => (before, after),
            _ => return None,
        };
        let empty = language.text("leer", "empty");
        Some(format!(
            "{} → {}",
            before.as_deref().unwrap_or(empty),
            after.as_deref().unwrap_or(empty)
        ))
    }
}

#[derive(Serialize, Deserialize)]
pub struct Version {
    pub yaml: String,
    pub env: EnvSnapshot,
    pub change: Change,
    pub unix_seconds: i64,
}

pub struct Draft {
    pub yaml: String,
    pub change: Change,
}

#[derive(Clone, Copy)]
pub enum Target {
    Saved(usize),
    Draft(usize),
}

impl Version {
    fn new(yaml: &str, env: EnvSnapshot, change: Change) -> Self {
        Self {
            yaml: yaml.into(),
            env,
            change,
            unix_seconds: chrono::Utc::now().timestamp(),
        }
    }

    pub fn time(&self, language: Language) -> String {
        let Some(time) = chrono::DateTime::from_timestamp(self.unix_seconds, 0) else {
            return String::new();
        };
        time.with_timezone(&chrono::Local)
            .format(language.text("%d.%m.%Y %H:%M:%S", "%Y-%m-%d %H:%M:%S"))
            .to_string()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Notice {
    Unreadable,
    WriteFailed,
    Changed,
}

#[derive(Serialize, Deserialize)]
struct Log {
    format: u32,
    versions: Vec<Version>,
}

pub struct History {
    pub versions: Vec<Version>,
    pub notice: Option<Notice>,
    active_count: usize,
    path: PathBuf,
    original: Option<Vec<u8>>,
    readable: bool,
}

impl History {
    pub fn sidecar_path(compose: &Path) -> PathBuf {
        let mut filename = OsString::from(".");
        filename.push(compose.file_name().unwrap_or_default());
        filename.push(".docksie-history.json");
        compose.with_file_name(filename)
    }

    pub fn load(compose: &Path, yaml: Option<&str>, env: EnvSnapshot) -> Self {
        let path = Self::sidecar_path(compose);
        let loaded = storage::read_optional(&path).and_then(|bytes| {
            let versions = if let Some(bytes) = &bytes {
                let log: Log = serde_json::from_slice(bytes)?;
                if log.format != 1 || log.versions.is_empty() {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                log.versions
            } else {
                Vec::new()
            };
            Ok((bytes, versions))
        });
        let (original, versions, readable) = match loaded {
            Ok((bytes, versions)) => (bytes, versions, true),
            Err(_) => (None, Vec::new(), false),
        };
        let mut history = Self {
            path,
            original,
            active_count: versions.len(),
            versions,
            readable,
            notice: (!readable).then_some(Notice::Unreadable),
        };
        if let Some(yaml) = yaml {
            let change = if history.versions.is_empty() {
                Change::Initial
            } else {
                Change::External
            };
            history.record(yaml, env, change);
        }
        history
    }

    pub fn active_count(&self) -> usize {
        self.active_count
    }

    pub fn current(&self) -> Option<&Version> {
        self.versions[..self.active_count].last()
    }

    pub fn discard_future(&mut self) {
        self.versions.truncate(self.active_count);
    }

    pub fn select(&mut self, index: usize) {
        self.active_count = index + 1;
        self.persist();
    }

    pub fn record(&mut self, yaml: &str, env: EnvSnapshot, change: Change) -> bool {
        if self
            .current()
            .is_some_and(|version| version.yaml == yaml && version.env == env)
        {
            return false;
        }
        self.discard_future();
        self.versions.push(Version::new(yaml, env, change));
        self.active_count = self.versions.len();
        true
    }

    pub fn persist(&mut self) {
        if !self.readable || self.active_count == 0 {
            return;
        }
        self.notice = self.write().err();
    }

    fn write(&mut self) -> Result<(), Notice> {
        #[derive(Serialize)]
        struct LogRef<'a> {
            format: u32,
            versions: &'a [Version],
        }
        let bytes = serde_json::to_vec(&LogRef {
            format: 1,
            // Later versions are only available in the current session.
            versions: &self.versions[..self.active_count],
        })
        .map_err(|_| Notice::WriteFailed)?;
        if self.original.as_ref() == Some(&bytes) {
            return Ok(());
        }
        let result = if let Some(original) = &self.original {
            storage::replace(&self.path, &bytes, || {
                storage::check_contents(&self.path, Some(original))
            })
        } else {
            storage::create(&self.path, &bytes)
        };
        match result {
            Ok(()) => {
                self.original = Some(bytes);
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(Notice::Changed),
            Err(_) => Err(Notice::WriteFailed),
        }
    }
}
