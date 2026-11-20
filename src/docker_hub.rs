use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct ImageReference {
    pub namespace: String,
    pub repository: String,
    pub reference: Option<String>,
}

impl ImageReference {
    pub fn parse(image: &str) -> Option<Self> {
        // Interpolated references cannot be reliably resolved before Compose runs.
        if image.contains('$') || image.contains(char::is_whitespace) {
            return None;
        }
        let (name, digest) = image
            .split_once('@')
            .map_or((image, None), |(name, digest)| {
                (name, Some(format!("@{digest}")))
            });
        let parts: Vec<_> = name.split('/').collect();
        let (namespace, repository) = match parts.as_slice() {
            [repository] => ("library", *repository),
            [first, repository]
                if first.contains('.') || first.contains(':') || *first == "localhost" =>
            {
                if *first != "docker.io" && *first != "index.docker.io" {
                    return None;
                }
                ("library", *repository)
            }
            [namespace, repository] => (*namespace, *repository),
            [registry, namespace, repository]
                if *registry == "docker.io" || *registry == "index.docker.io" =>
            {
                (*namespace, *repository)
            }
            _ => return None,
        };
        if !valid_component(namespace) || repository.is_empty() {
            return None;
        }
        let (repository, tag) = repository
            .rsplit_once(':')
            .map_or((repository, None), |(repo, tag)| (repo, Some(tag)));
        if repository.split('/').any(|part| !valid_component(part)) {
            return None;
        }
        let reference = tag.map(str::to_owned).or(digest);
        Some(Self {
            namespace: namespace.to_owned(),
            repository: repository.to_owned(),
            reference,
        })
    }

    pub fn url(&self) -> String {
        let mut url = format!(
            "https://hub.docker.com/r/{}/{}",
            self.namespace, self.repository
        );
        if let Some(tag) = &self.reference
            && !tag.starts_with('@')
        {
            url.push_str("/tags/");
            url.push_str(tag);
        }
        url
    }
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

#[derive(Clone, Debug, Deserialize)]
pub struct DockerHubInfo {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub full_description: String,
    #[serde(default)]
    pub pull_count: u64,
    #[serde(default)]
    pub star_count: u64,
    #[serde(default)]
    pub last_updated: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterRequirement {
    Required,
    Optional,
    Unspecified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentedParameter {
    pub name: String,
    pub requirement: ParameterRequirement,
    pub default_value: Option<String>,
    pub description: String,
}

pub fn documented_environment_parameters(markdown: &str) -> Vec<DocumentedParameter> {
    let mut parameters = Vec::new();
    let mut section_heading = String::new();
    let mut headers: Option<Vec<String>> = None;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            section_heading = heading.to_ascii_lowercase();
            headers = None;
            continue;
        }
        let relevant_section = ["environment", "variables", "parameters", "configuration"]
            .iter()
            .any(|word| section_heading.contains(word));
        if !relevant_section || !trimmed.starts_with('|') {
            if !trimmed.is_empty() {
                headers = None;
            }
            continue;
        }
        let cells: Vec<String> = trimmed
            .trim_matches('|')
            .split('|')
            .map(clean_markdown_cell)
            .collect();
        if headers.is_none() {
            if cells.first().is_some_and(|header| {
                let header = header.to_ascii_lowercase();
                header.contains("variable")
                    || header.contains("parameter")
                    || ((section_heading.contains("environment")
                        || section_heading.contains("variables"))
                        && (header == "name" || header == "key"))
            }) {
                headers = Some(cells);
            }
            continue;
        }
        let Some(columns) = &headers else { continue };
        let Some(name) = cells.first().filter(|name| valid_env_name(name)) else {
            continue;
        };
        let requirement_index = columns.iter().position(|header| {
            let header = header.to_ascii_lowercase();
            [
                "required",
                "mandatory",
                "optional",
                "requirement",
                "pflicht",
            ]
            .iter()
            .any(|word| header.contains(word))
        });
        let status_header = requirement_index
            .and_then(|index| columns.get(index))
            .map(String::as_str)
            .unwrap_or("");
        let marker = requirement_index
            .and_then(|index| cells.get(index))
            .map(String::as_str)
            .unwrap_or("");
        let requirement = classify_requirement(status_header, marker);
        let default_index = columns
            .iter()
            .position(|header| header.to_ascii_lowercase().contains("default"));
        let default_value = default_index
            .and_then(|index| cells.get(index))
            .filter(|value| !value.is_empty() && value.as_str() != "-")
            .cloned();
        let description = cells
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(index, _)| Some(*index) != requirement_index && Some(*index) != default_index)
            .map(|(_, cell)| cell.as_str())
            .filter(|cell| !cell.is_empty() && *cell != "-")
            .collect::<Vec<_>>()
            .join(" · ");
        if !parameters
            .iter()
            .any(|parameter: &DocumentedParameter| parameter.name == *name)
        {
            parameters.push(DocumentedParameter {
                name: name.clone(),
                requirement,
                default_value,
                description,
            });
        }
    }
    parameters
}

fn clean_markdown_cell(cell: &str) -> String {
    cell.trim()
        .trim_matches('`')
        .trim_matches('*')
        .trim()
        .to_owned()
}

fn valid_env_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn classify_requirement(header: &str, marker: &str) -> ParameterRequirement {
    let header = header.to_ascii_lowercase();
    let marker = marker.to_ascii_lowercase();
    if marker.contains("optional") || marker.contains("not required") {
        return ParameterRequirement::Optional;
    }
    if marker.contains("mandatory") || marker.contains("required") || marker.contains("pflicht") {
        return ParameterRequirement::Required;
    }
    let affirmative = marker == "yes" || marker == "true" || marker == "ja" || marker == "✓";
    let negative = marker == "no" || marker == "false" || marker == "nein" || marker == "✗";
    if header.contains("optional") {
        if affirmative {
            ParameterRequirement::Optional
        } else if negative {
            ParameterRequirement::Required
        } else {
            ParameterRequirement::Unspecified
        }
    } else if affirmative || (header.contains("required") && marker == "x") {
        ParameterRequirement::Required
    } else if negative {
        ParameterRequirement::Optional
    } else {
        ParameterRequirement::Unspecified
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct DockerHubTag {
    pub name: String,
    #[serde(default)]
    pub last_updated: Option<String>,
}

#[derive(Deserialize)]
struct TagPage {
    next: Option<String>,
    results: Vec<DockerHubTag>,
}

pub fn fetch(reference: &ImageReference) -> Result<DockerHubInfo, String> {
    let url = format!(
        "https://hub.docker.com/v2/namespaces/{}/repositories/{}",
        reference.namespace, reference.repository
    );
    let response = reqwest::blocking::Client::builder()
        .user_agent(concat!("CompEditor/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|_| "network".to_owned())?
        .get(url)
        .send()
        .map_err(|_| "network".to_owned())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("not_found".to_owned());
    }
    if !response.status().is_success() {
        return Err("unavailable".to_owned());
    }
    response.json().map_err(|_| "unavailable".to_owned())
}

pub fn fetch_tags(reference: &ImageReference) -> Result<Vec<DockerHubTag>, String> {
    // Docker Hub's public tags endpoint does not require an account. The newer
    // documented endpoint requires an organization access token.
    let mut url = format!(
        "https://hub.docker.com/v2/repositories/{}/{}/tags?page_size=100&page=1",
        reference.namespace, reference.repository
    );
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("CompEditor/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|_| "network".to_owned())?;
    let mut tags = Vec::new();
    let mut visited = std::collections::HashSet::new();
    loop {
        if !visited.insert(url.clone()) {
            break;
        }
        let response = client.get(&url).send().map_err(|_| "network".to_owned())?;
        if !response.status().is_success() {
            return Err(match response.status().as_u16() {
                401 | 403 => "access".to_owned(),
                404 => "not_found".to_owned(),
                429 => "rate_limited".to_owned(),
                _ => "unavailable".to_owned(),
            });
        }
        let page: TagPage = response.json().map_err(|_| "invalid_response".to_owned())?;
        tags.extend(page.results);
        let Some(next) = page.next else { break };
        if !next.starts_with("https://hub.docker.com/") {
            break;
        }
        url = next;
    }
    Ok(sort_tags(tags))
}

fn sort_tags(mut tags: Vec<DockerHubTag>) -> Vec<DockerHubTag> {
    tags.sort_by(|left, right| {
        right
            .last_updated
            .cmp(&left.last_updated)
            .then_with(|| left.name.cmp(&right.name))
    });
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_docker_hub_image_names() {
        let official = ImageReference::parse("nginx:1.27").unwrap();
        assert_eq!(official.namespace, "library");
        assert_eq!(official.repository, "nginx");
        assert_eq!(official.reference.as_deref(), Some("1.27"));
        let namespaced = ImageReference::parse("docker.io/appwrite/appwrite:latest").unwrap();
        assert_eq!(namespaced.namespace, "appwrite");
        assert_eq!(namespaced.repository, "appwrite");
        assert!(ImageReference::parse("ghcr.io/acme/app").is_none());
        assert!(ImageReference::parse("appwrite/appwrite:${TAG}").is_none());
    }

    #[test]
    fn sorts_tags_by_updated_time_newest_first() {
        let tags = sort_tags(vec![
            DockerHubTag {
                name: "old".into(),
                last_updated: Some("2024-01-01T00:00:00Z".into()),
            },
            DockerHubTag {
                name: "new".into(),
                last_updated: Some("2025-01-01T00:00:00Z".into()),
            },
        ]);
        assert_eq!(tags[0].name, "new");
    }
}
