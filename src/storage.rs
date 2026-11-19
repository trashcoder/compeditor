use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub fn replace(
    path: &Path,
    bytes: &[u8],
    before_replace: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let mut temp = staged_file(path)?;
    if let Ok(metadata) = fs::metadata(path) {
        temp.as_file().set_permissions(metadata.permissions())?;
    }
    write_staged(&mut temp, bytes)?;
    before_replace()?;
    temp.persist(path)?;
    Ok(())
}

pub fn create(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temp = staged_file(path)?;
    write_staged(&mut temp, bytes)?;
    temp.persist_noclobber(path)?;
    Ok(())
}

fn staged_file(path: &Path) -> io::Result<tempfile::NamedTempFile> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    tempfile::NamedTempFile::new_in(parent)
}

fn write_staged(temp: &mut tempfile::NamedTempFile, bytes: &[u8]) -> io::Result<()> {
    temp.write_all(bytes)?;
    temp.as_file().sync_all()
}

pub fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn check_contents(path: &Path, expected: Option<&[u8]>) -> io::Result<()> {
    if read_optional(path)?.as_deref() == expected {
        Ok(())
    } else {
        Err(io::ErrorKind::AlreadyExists.into())
    }
}

pub fn config_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    base.map(|path| path.join("docksie").join("language"))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Language {
    #[default]
    German,
    English,
}

impl Language {
    pub fn text(self, german: &'static str, english: &'static str) -> &'static str {
        match self {
            Self::German => german,
            Self::English => english,
        }
    }

    pub fn load(path: Option<&Path>) -> Self {
        match path
            .and_then(|path| fs::read_to_string(path).ok())
            .as_deref()
            .map(str::trim)
        {
            Some("en") => Self::English,
            _ => Self::German,
        }
    }

    pub fn save(self, path: Option<&Path>) -> io::Result<()> {
        let path = path.ok_or(io::ErrorKind::NotFound)?;
        fs::create_dir_all(path.parent().ok_or(io::ErrorKind::NotFound)?)?;
        replace(path, self.text("de\n", "en\n").as_bytes(), || Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_round_trip_and_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("docksie").join("language");
        assert_eq!(Language::load(Some(&path)), Language::German);
        Language::English.save(Some(&path)).unwrap();
        assert_eq!(Language::load(Some(&path)), Language::English);
        Language::German.save(Some(&path)).unwrap();
        assert_eq!(Language::load(Some(&path)), Language::German);
        fs::write(&path, "unknown").unwrap();
        assert_eq!(Language::load(Some(&path)), Language::German);
    }

    #[test]
    fn language_fails_when_configuration_parent_is_a_file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(
            Language::English
                .save(Some(&file.path().join("language")))
                .is_err()
        );
    }
}
