use crate::storage;
use std::{collections::BTreeMap, fs, io, ops::Range, path::Path};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub text: String,
    pub unresolved: bool,
}

#[derive(Debug, Default)]
pub struct EnvFile {
    source: String,
    pub values: BTreeMap<String, Preview>,
    spans: BTreeMap<String, Range<usize>>,
}

pub enum Environment {
    Missing,
    Loaded(EnvFile),
    Invalid(usize),
    Unreadable,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveError {
    Unavailable,
    InvalidName,
    Conflict,
    Io,
}

impl Environment {
    pub fn load(compose_path: &Path) -> Self {
        match fs::read_to_string(compose_path.with_file_name(".env")) {
            Ok(source) => match parse_dotenv(&source) {
                Ok(mut file) => {
                    file.source = source;
                    Self::Loaded(file)
                }
                Err(line) => Self::Invalid(line),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::Missing,
            Err(_) => Self::Unreadable,
        }
    }

    pub fn value(&self, name: &str) -> Option<&Preview> {
        self.values()?.get(name)
    }

    fn values(&self) -> Option<&BTreeMap<String, Preview>> {
        match self {
            Self::Loaded(file) => Some(&file.values),
            _ => None,
        }
    }

    pub fn changed_key<'a>(&'a self, previous: &'a Self) -> Option<&'a str> {
        self.values()
            .into_iter()
            .chain(previous.values())
            .flat_map(BTreeMap::keys)
            .find(|key| self.value(key) != previous.value(key))
            .map(String::as_str)
    }

    pub fn preview(&self, input: &str) -> Preview {
        expand(input, &|name| self.value(name).cloned(), 0)
    }

    pub fn save_value(
        &mut self,
        compose_path: &Path,
        name: &str,
        value: &str,
    ) -> Result<(), SaveError> {
        if !valid_key(name) {
            return Err(SaveError::InvalidName);
        }
        let encoded = encode_value(value);
        let (expected, candidate) = match self {
            Self::Missing => (None, format!("{name}={encoded}\n")),
            Self::Loaded(file) => {
                let mut candidate = file.source.clone();
                if let Some(span) = file.spans.get(name) {
                    candidate.replace_range(span.clone(), &encoded);
                } else {
                    let newline = if candidate.contains("\r\n") {
                        "\r\n"
                    } else {
                        "\n"
                    };
                    if !candidate.is_empty() && !candidate.ends_with('\n') {
                        candidate.push_str(newline);
                    }
                    candidate.push_str(&format!("{name}={encoded}{newline}"));
                }
                (Some(file.source.as_str()), candidate)
            }
            _ => return Err(SaveError::Unavailable),
        };
        let mut file = parse_dotenv(&candidate).map_err(|_| SaveError::Io)?;
        let path = compose_path.with_file_name(".env");
        let target = if expected.is_some() {
            fs::canonicalize(&path).map_err(|error| match error.kind() {
                io::ErrorKind::NotFound => SaveError::Conflict,
                _ => SaveError::Io,
            })?
        } else {
            path.clone()
        };
        if expected.is_none() {
            let mut temp = tempfile::NamedTempFile::new_in(path.parent().ok_or(SaveError::Io)?)
                .map_err(|_| SaveError::Io)?;
            use std::io::Write;
            temp.write_all(candidate.as_bytes())
                .map_err(|_| SaveError::Io)?;
            temp.as_file().sync_all().map_err(|_| SaveError::Io)?;
            temp.persist_noclobber(&path)
                .map_err(|error| match error.error.kind() {
                    io::ErrorKind::AlreadyExists => SaveError::Conflict,
                    _ => SaveError::Io,
                })?;
            file.source = candidate;
            *self = Self::Loaded(file);
            return Ok(());
        }
        let mut changed = false;
        let result = storage::replace(&target, candidate.as_bytes(), || {
            let actual = match fs::read_to_string(&path) {
                Ok(source) => Some(source),
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        && fs::symlink_metadata(&path).is_err() =>
                {
                    None
                }
                Err(error) => return Err(error),
            };
            changed = actual.as_deref() != expected || fs::canonicalize(&path)? != target;
            if changed {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            Ok(())
        });
        if changed {
            return Err(SaveError::Conflict);
        }
        result.map_err(|_| SaveError::Io)?;
        file.source = candidate;
        *self = Self::Loaded(file);
        Ok(())
    }
}

fn valid_key(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| name_char(byte) || matches!(byte, b'.' | b'-' | b'[' | b']'))
}

fn encode_value(value: &str) -> String {
    let mut encoded = String::from("\"");
    for ch in value.chars() {
        encoded.push_str(match ch {
            '\\' => "\\\\",
            '"' => "\\\"",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            '$' => "$$",
            ch => {
                encoded.push(ch);
                continue;
            }
        });
    }
    encoded.push('"');
    encoded
}

type Line<'a> = (usize, (usize, &'a str));

fn parse_dotenv(source: &str) -> Result<EnvFile, usize> {
    let bom = if source.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let mut lines = source[bom..]
        .split_inclusive('\n')
        .scan(bom, |offset, line| {
            let start = *offset;
            *offset += line.len();
            let line = line.strip_suffix('\n').unwrap_or(line);
            Some((start, line.strip_suffix('\r').unwrap_or(line)))
        })
        .enumerate();
    let mut file = EnvFile::default();
    while let Some((index, (offset, original))) = lines.next() {
        let line = original.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line
            .strip_prefix("export")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .map_or(line, str::trim_start);
        let (name, raw) = line
            .split_once(['=', ':'])
            .map_or((line, None), |(name, raw)| (name, Some(raw)));
        let name = name.trim();
        if !valid_key(name) {
            return Err(index + 1);
        }
        let Some(raw) = raw else {
            file.values.remove(name);
            file.spans.remove(name);
            continue;
        };
        let trimmed = raw.trim_start();
        let start = offset + original.len() - trimmed.len();
        let (value, span) = match trimmed.chars().next() {
            Some(quote @ ('\'' | '"')) => {
                let (text, end) =
                    quoted_value(quote, &trimmed[1..], start + 1, &mut lines).ok_or(index + 1)?;
                let value = if quote == '\'' {
                    Preview {
                        text,
                        unresolved: false,
                    }
                } else {
                    expand(&text, &|name| file.values.get(name).cloned(), 0)
                };
                (value, start..end)
            }
            _ => {
                let content = raw
                    .char_indices()
                    .find_map(|(index, ch)| {
                        (ch == '#' && raw[..index].ends_with(char::is_whitespace))
                            .then_some(&raw[..index])
                    })
                    .unwrap_or(raw)
                    .trim();
                let start = if content.is_empty() {
                    offset + original.len() - raw.len()
                } else {
                    start
                };
                let value = expand(content, &|name| file.values.get(name).cloned(), 0);
                (value, start..start + content.len())
            }
        };
        file.values.insert(name.to_owned(), value);
        file.spans.insert(name.to_owned(), span);
    }
    Ok(file)
}

fn quoted_value<'a>(
    quote: char,
    mut line: &'a str,
    mut offset: usize,
    lines: &mut impl Iterator<Item = Line<'a>>,
) -> Option<(String, usize)> {
    let mut value = String::new();
    loop {
        let mut chars = line.char_indices();
        while let Some((index, ch)) = chars.next() {
            if ch == quote {
                let tail = chars.as_str().trim_start();
                return (tail.is_empty() || tail.starts_with('#'))
                    .then_some((value, offset + index + 1));
            }
            if ch == '\\'
                && let Some((_, next)) = chars.next()
            {
                match (quote, next) {
                    (_, next) if quote == next => value.push(next),
                    ('"', 'n') => value.push('\n'),
                    ('"', 'r') => value.push('\r'),
                    ('"', 't') => value.push('\t'),
                    ('"', '\\') => value.push('\\'),
                    _ => {
                        value.push('\\');
                        value.push(next);
                    }
                }
            } else {
                value.push(ch);
            }
        }
        (_, (offset, line)) = lines.next()?;
        value.push('\n');
    }
}

pub fn references(mut input: &str) -> Vec<&str> {
    let mut names = Vec::new();
    while let Some((_, tail)) = input.split_once('$') {
        if let Some(rest) = tail.strip_prefix('$') {
            input = rest;
            continue;
        }
        let tail = tail.strip_prefix('{').unwrap_or(tail);
        let name = variable_name(tail);
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
        input = &tail[name.len()..];
    }
    names
}

fn name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn name_char(byte: u8) -> bool {
    name_start(byte) || byte.is_ascii_digit()
}

fn variable_name(input: &str) -> &str {
    if !input.bytes().next().is_some_and(name_start) {
        return "";
    }
    &input[..input.bytes().take_while(|byte| name_char(*byte)).count()]
}

fn closing_brace(input: &str) -> Option<usize> {
    let mut depth = 0;
    for (index, byte) in input.bytes().enumerate() {
        match byte {
            b'{' if index > 0 && input.as_bytes()[index - 1] == b'$' => depth += 1,
            b'}' if depth == 0 => return Some(index),
            b'}' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn expand(input: &str, lookup: &impl Fn(&str) -> Option<Preview>, depth: usize) -> Preview {
    if depth == 32 {
        return Preview {
            text: input.to_owned(),
            unresolved: true,
        };
    }
    let mut result = Preview::default();
    let mut remaining = input;
    while let Some((prefix, tail)) = remaining.split_once('$') {
        result.text.push_str(prefix);
        if let Some(rest) = tail.strip_prefix('$') {
            result.text.push('$');
            remaining = rest;
            continue;
        }
        let name = variable_name(tail);
        let (length, value) = if let Some(expression) = tail.strip_prefix('{') {
            let Some(end) = closing_brace(expression) else {
                result.text.push('$');
                result.text.push_str(tail);
                result.unresolved = true;
                return result;
            };
            (end + 2, substitute(&expression[..end], lookup, depth))
        } else if !name.is_empty() {
            (name.len(), lookup(name))
        } else {
            result.text.push('$');
            remaining = tail;
            continue;
        };
        if let Some(value) = value {
            result.text.push_str(&value.text);
            result.unresolved |= value.unresolved;
        } else {
            result.text.push('$');
            result.text.push_str(&tail[..length]);
            result.unresolved = true;
        }
        remaining = &tail[length..];
    }
    result.text.push_str(remaining);
    result
}

fn substitute(
    expression: &str,
    lookup: &impl Fn(&str) -> Option<Preview>,
    depth: usize,
) -> Option<Preview> {
    let name = variable_name(expression);
    if name.is_empty() {
        return None;
    }
    let value = lookup(name);
    let rest = &expression[name.len()..];
    if rest.is_empty() {
        return value;
    }
    let (operator, check_empty) = rest
        .strip_prefix(':')
        .map_or((rest, false), |op| (op, true));
    let value = value.filter(|value| !check_empty || !value.text.is_empty());
    let argument = || expand(&operator[1..], lookup, depth + 1);
    match operator.as_bytes().first()? {
        b'-' => value.or_else(|| Some(argument())),
        b'?' => value,
        b'+' => Some(value.map_or_else(Preview::default, |_| argument())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_references_for_the_sidebar_in_source_order() {
        assert_eq!(
            references("$FIRST ${SECOND:-${THIRD}} $$SKIP $FIRST"),
            ["FIRST", "SECOND", "THIRD"]
        );
    }

    #[test]
    fn edits_only_the_selected_dotenv_value() {
        let folder = tempfile::tempdir().unwrap();
        let compose = folder.path().join("compose.yaml");
        let dotenv = folder.path().join(".env");
        let before = "\u{feff}# settings\r\nexport TAG : 'alpiné'  # release\r\nURL=${TAG}\r\n";
        fs::write(&dotenv, before).unwrap();
        let mut environment = Environment::load(&compose);
        environment
            .save_value(&compose, "TAG", "$NEW\n\"quoted\"")
            .unwrap();
        let saved = fs::read_to_string(&dotenv).unwrap();
        assert_eq!(
            saved,
            "\u{feff}# settings\r\nexport TAG : \"$$NEW\\n\\\"quoted\\\"\"  # release\r\nURL=${TAG}\r\n"
        );
        assert_eq!(environment.preview("${URL}").text, "$NEW\n\"quoted\"");
        fs::write(&dotenv, "TAG=external\n").unwrap();
        assert_eq!(
            environment.save_value(&compose, "TAG", "local"),
            Err(SaveError::Conflict)
        );
        assert_eq!(fs::read_to_string(&dotenv).unwrap(), "TAG=external\n");
    }

    #[test]
    fn creates_a_missing_dotenv_only_on_save() {
        let folder = tempfile::tempdir().unwrap();
        let compose = folder.path().join("compose.yaml");
        let dotenv = folder.path().join(".env");
        let mut environment = Environment::load(&compose);
        assert!(!dotenv.exists());
        assert_eq!(environment.preview("${PORT:-8080}").text, "8080");
        environment.save_value(&compose, "PORT", "8181").unwrap();
        assert_eq!(fs::read_to_string(&dotenv).unwrap(), "PORT=\"8181\"\n");
        assert_eq!(environment.preview("${PORT:-8080}").text, "8181");
    }

    #[test]
    fn reads_dotenv_with_quotes_comments_and_internal_references() {
        let env = parse_dotenv(concat!(
            "\u{feff}# Project\r\n",
            "export TAG=1.8.0 # Comment\r\n",
            "IMAGE: \"appwrite:${TAG}\"\r\n",
            "LITERAL='${TAG}\n # second line' # comment\n",
            "ESCAPED=\"a\\tb\\\"c\"\n",
            "HASH=a#b\n",
            "EMPTY=\n",
        ))
        .unwrap();
        assert_eq!(env.values["TAG"].text, "1.8.0");
        assert_eq!(env.values["IMAGE"].text, "appwrite:1.8.0");
        assert_eq!(env.values["LITERAL"].text, "${TAG}\n # second line");
        assert_eq!(env.values["ESCAPED"].text, "a\tb\"c");
        assert_eq!(env.values["HASH"].text, "a#b");
        assert_eq!(env.values["EMPTY"].text, "");
        assert_eq!(parse_dotenv("TAG=1\nBROKEN='").unwrap_err(), 2);
    }

    #[test]
    fn reports_unresolved_expressions_and_applies_compose_defaults() {
        let env = Environment::Loaded(parse_dotenv("SET=value\nEMPTY=\n").unwrap());
        assert_eq!(
            env.preview("$SET:${SET}:${EMPTY-fallback}:${EMPTY:-fallback}")
                .text,
            "value:value::fallback"
        );
        assert_eq!(
            env.preview("${SET:+${MISSING:-nested}}/${EMPTY:+no}/${EMPTY+yes}")
                .text,
            "nested//yes"
        );
        assert_eq!(env.preview("$$SET").text, "$SET");
        for input in [
            "${MISSING}",
            "${MISSING:?required}",
            "${EMPTY:?required}",
            "${UNFINISHED",
        ] {
            let result = env.preview(input);
            assert_eq!(result.text, input);
            assert!(result.unresolved);
        }
        assert_eq!(
            Environment::Missing.preview("${TAG:-latest}").text,
            "latest"
        );
        assert!(Environment::Missing.preview("${TAG}").unresolved);
    }
}
