use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Network,
    Volume,
}

impl ResourceKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Network => "networks",
            Self::Volume => "volumes",
        }
    }

    pub fn title(self, language: Language) -> &'static str {
        match self {
            Self::Network => language.text("Netzwerke", "Networks"),
            Self::Volume => "Volumes",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceSettings {
    pub name: String,
    pub driver: String,
    pub external: bool,
}

#[derive(Clone, Debug)]
pub struct Resource {
    pub key: String,
    pub yaml: String,
    pub settings: Option<ResourceSettings>,
    pub editable: bool,
    pub used_by: Vec<String>,
}

#[derive(Default)]
pub struct Resources {
    pub networks: ResourceList,
    pub volumes: ResourceList,
}

#[derive(Default)]
pub struct ResourceList {
    pub entries: Vec<Resource>,
    pub can_add: bool,
    pub unknown_usage: bool,
}

impl Resources {
    pub fn list(&self, kind: ResourceKind) -> &ResourceList {
        match kind {
            ResourceKind::Network => &self.networks,
            ResourceKind::Volume => &self.volumes,
        }
    }

    pub(super) fn inspect(root: &Mapping, registry: &AnchorRegistry, source: &str) -> Self {
        Self {
            networks: ResourceList::inspect(root, registry, source, ResourceKind::Network),
            volumes: ResourceList::inspect(root, registry, source, ResourceKind::Volume),
        }
    }

    pub(super) fn with_usage(mut self, services: &[Service], external: bool) -> Self {
        self.networks
            .find_usage(services, ResourceKind::Network, external);
        self.volumes
            .find_usage(services, ResourceKind::Volume, external);
        self
    }
}

impl ResourceList {
    fn inspect(
        root: &Mapping,
        registry: &AnchorRegistry,
        source: &str,
        kind: ResourceKind,
    ) -> Self {
        let node = root.get(kind.key());
        let local = node.as_ref().and_then(YamlNode::as_mapping);
        let resolved = node.clone().map(|node| resolve(registry, node));
        let mapping = resolved.as_ref().and_then(YamlNode::as_mapping);
        let ordinary = node.as_ref().is_none_or(|node| !shared(node, false));
        let can_add = !has_merge(root)
            && ordinary
            && mapping.is_none_or(|map| !has_merge(map) && scalar_keys(map))
            && (local.is_some() || empty_yaml_value(node.as_ref()));
        let entries = mapping
            .into_iter()
            .flat_map(Mapping::entries)
            .filter_map(|entry| {
                let key = entry.key_node()?.as_scalar()?.as_string();
                if key == "<<" {
                    return None;
                }
                let node = entry.value_node()?;
                let editable = can_add && !shared(&node, true);
                let settings = resource_settings(&node);
                Some(Resource {
                    yaml: entry_yaml(source, mapping?, &key, &node),
                    key,
                    settings,
                    editable,
                    used_by: Vec::new(),
                })
            })
            .collect();
        Self {
            entries,
            can_add,
            unknown_usage: false,
        }
    }

    fn find_usage(&mut self, services: &[Service], kind: ResourceKind, external: bool) {
        self.unknown_usage = external;
        for service in services {
            let usage = service_usage(service, services, kind, &mut HashSet::new());
            self.unknown_usage |= usage.unknown;
            let uses_default = kind == ResourceKind::Network
                && !usage.unknown
                && usage.network_mode.is_none()
                && usage.references.is_empty();
            for entry in &mut self.entries {
                if (uses_default && entry.key == "default")
                    || usage
                        .references
                        .iter()
                        .any(|reference| reference.key == Some(entry.key.as_str()))
                {
                    entry.used_by.push(service.name.clone());
                }
            }
        }
    }
}

struct ResourceReference<'a> {
    key: Option<&'a str>,
    mount_target: Option<&'a str>,
}

#[derive(Default)]
struct ServiceUsage<'a> {
    references: Vec<ResourceReference<'a>>,
    network_mode: Option<&'a str>,
    unknown: bool,
}

fn service_usage<'a>(
    service: &'a Service,
    services: &'a [Service],
    kind: ResourceKind,
    visiting: &mut HashSet<&'a str>,
) -> ServiceUsage<'a> {
    if !visiting.insert(&service.name) {
        return ServiceUsage {
            unknown: true,
            ..ServiceUsage::default()
        };
    }
    let parameters = &service.parameters;
    let section = parameters.section(match kind {
        ResourceKind::Network => SectionKind::Networks,
        ResourceKind::Volume => SectionKind::Volumes,
    });
    let mut usage = parameters
        .extends
        .as_deref()
        .map_or_else(ServiceUsage::default, |name| {
            linked_usage(name, services, kind, visiting)
        });
    usage.unknown |= parameters.uncertain_mapping
        || !section.form_supported
        || section.rows.iter().any(|row| row.reference_unknown);
    if kind == ResourceKind::Volume {
        if let Some(sources) = &parameters.volumes_from {
            for source in sources {
                let name = source.split(':').next().unwrap_or_default();
                let inherited = linked_usage(name, services, kind, visiting);
                usage.unknown |= inherited.unknown;
                usage.references.extend(inherited.references);
            }
        }
        usage.unknown |= parameters.unparsed_volumes_from;
    }
    for row in &section.rows {
        if kind == ResourceKind::Volume
            && let Some(target) = row.mount_target.as_deref()
        {
            usage
                .references
                .retain(|reference| reference.mount_target != Some(target));
        }
        usage.references.push(ResourceReference {
            key: row.resource_ref.as_deref(),
            mount_target: row.mount_target.as_deref(),
        });
    }
    if kind == ResourceKind::Network {
        if let Some(mode) = parameters.network_mode_value.as_deref() {
            usage.network_mode = Some(mode);
        }
        if let Some(mode) = usage.network_mode {
            usage.unknown |= mode.contains('$');
            if let Some(name) = mode.strip_prefix("service:") {
                let linked = linked_usage(name, services, kind, visiting);
                usage.unknown |= linked.unknown;
                usage.network_mode = linked.network_mode;
                usage.references = linked.references;
            } else {
                usage.references.clear();
            }
        }
    }
    visiting.remove(service.name.as_str());
    usage
}

fn linked_usage<'a>(
    name: &str,
    services: &'a [Service],
    kind: ResourceKind,
    visiting: &mut HashSet<&'a str>,
) -> ServiceUsage<'a> {
    services
        .iter()
        .find(|service| service.name == name)
        .map_or_else(
            || ServiceUsage {
                unknown: true,
                ..ServiceUsage::default()
            },
            |service| service_usage(service, services, kind, visiting),
        )
}

fn resource_settings(node: &YamlNode) -> Option<ResourceSettings> {
    if empty_yaml_value(Some(node)) {
        return Some(ResourceSettings::default());
    }
    let map = node.as_mapping()?;
    if !scalar_keys(map) || has_merge(map) {
        return None;
    }
    let external = bool_field(map, "external", false)?;
    if external
        && map
            .iter()
            .any(|(key, _)| !matches!(detail(&key).as_str(), "external" | "name"))
    {
        return None;
    }
    Some(ResourceSettings {
        name: optional_scalar(map, "name")?.unwrap_or_default(),
        driver: optional_scalar(map, "driver")?.unwrap_or_default(),
        external,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SectionKind {
    Networks,
    Volumes,
    Labels,
    Dependencies,
}

impl SectionKind {
    pub const ALL: [Self; 4] = [
        Self::Networks,
        Self::Volumes,
        Self::Labels,
        Self::Dependencies,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Networks => "networks",
            Self::Volumes => "volumes",
            Self::Labels => "labels",
            Self::Dependencies => "depends_on",
        }
    }

    pub fn title(self, language: Language) -> &'static str {
        match self {
            Self::Networks => ResourceKind::Network.title(language),
            Self::Volumes => ResourceKind::Volume.title(language),
            Self::Labels => "Labels",
            Self::Dependencies => language.text("Abhängigkeiten", "Dependencies"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Parameter {
    pub key: String,
    pub yaml: String,
    pub editable: bool,
}

#[derive(Clone, Debug)]
pub struct ServiceParameters {
    pub sections: [ServiceSection; 4],
    pub additional: Vec<Parameter>,
    pub can_add: bool,
    pub network_mode: bool,
    network_mode_value: Option<String>,
    uncertain_mapping: bool,
    extends: Option<String>,
    volumes_from: Option<Vec<String>>,
    unparsed_volumes_from: bool,
}

#[derive(Clone, Debug)]
pub struct ServiceSection {
    pub rows: Vec<SectionRow>,
    pub yaml: Option<String>,
    pub editable: bool,
    pub form_supported: bool,
}

#[derive(Clone, Debug)]
pub struct SectionRow {
    pub label: String,
    pub value: Option<RowValue>,
    pub resource_ref: Option<String>,
    reference_unknown: bool,
    mount_target: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowValue {
    Network(String),
    Mount(Mount),
    Label(Variable),
    Dependency(Dependency),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mount {
    pub source: String,
    pub target: String,
    pub bind: bool,
    pub read_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub service: String,
    pub condition: String,
    pub restart: bool,
    pub required: bool,
}

impl Default for Dependency {
    fn default() -> Self {
        Self {
            service: String::new(),
            condition: "service_started".into(),
            restart: false,
            required: true,
        }
    }
}

impl RowValue {
    fn kind(&self) -> SectionKind {
        match self {
            Self::Network(_) => SectionKind::Networks,
            Self::Mount(_) => SectionKind::Volumes,
            Self::Label(_) => SectionKind::Labels,
            Self::Dependency(_) => SectionKind::Dependencies,
        }
    }

    fn identity(&self) -> &str {
        match self {
            Self::Network(key) => key,
            Self::Mount(mount) => &mount.target,
            Self::Label(label) => &label.key,
            Self::Dependency(dependency) => &dependency.service,
        }
    }
}

impl ServiceParameters {
    pub fn section(&self, kind: SectionKind) -> &ServiceSection {
        &self.sections[kind as usize]
    }

    pub(super) fn inspect(
        source: &str,
        mapping: &Mapping,
        registry: &AnchorRegistry,
        field: &impl Fn(&str) -> Option<YamlNode>,
        edit: &impl Fn(&str) -> bool,
        read_only: bool,
    ) -> Self {
        let network_mode_node = field("network_mode").filter(|node| !empty_yaml_value(Some(node)));
        let network_mode = network_mode_node.is_some();
        let network_mode_value = network_mode_node.map(|node| {
            node.as_scalar()
                .map_or_else(|| "$".into(), |scalar| scalar.as_string())
        });
        let sections = SectionKind::ALL.map(|kind| {
            let key = kind.key();
            let value = field(key);
            let raw = mapping.get(key);
            let editable = edit(key) && !(kind == SectionKind::Networks && network_mode);
            ServiceSection::inspect(
                kind,
                value.as_ref(),
                raw.as_ref()
                    .or(value.as_ref())
                    .map(|node| entry_yaml(source, mapping, key, node)),
                editable,
            )
        });
        let merged = MergedMapping::new(mapping.clone(), registry).ok();
        let mut seen = HashSet::new();
        let additional = mapping
            .entries()
            .filter_map(|entry| entry.key_node()?.as_scalar().map(|key| key.as_string()))
            .chain(
                merged
                    .iter()
                    .flat_map(|map| map.keys())
                    .filter_map(|key| key.as_scalar().map(|scalar| scalar.as_string())),
            )
            .filter(|key| additional_key(key) && seen.insert(key.clone()))
            .filter_map(|key| {
                let local = mapping.get(&key);
                let node = local.or_else(|| field(&key))?;
                Some(Parameter {
                    yaml: entry_yaml(source, mapping, &key, &node),
                    editable: key != "extends" && edit(&key),
                    key,
                })
            })
            .collect();
        let extends = field("extends").map(|node| {
            node.as_mapping()
                .filter(|map| !map.contains_key("file"))
                .and_then(|map| optional_scalar(map, "service").flatten())
                .unwrap_or_default()
        });
        let volumes_from_node = field("volumes_from");
        let volumes_from = volumes_from_node
            .as_ref()
            .and_then(YamlNode::as_sequence)
            .and_then(|list| {
                list.values()
                    .map(|node| Some(node.as_scalar()?.as_string()))
                    .collect()
            });
        let unparsed_volumes_from = volumes_from_node.is_some() && volumes_from.is_none();
        Self {
            sections,
            additional,
            can_add: edit(""),
            network_mode,
            network_mode_value,
            uncertain_mapping: read_only || has_merge(mapping),
            extends,
            volumes_from,
            unparsed_volumes_from,
        }
    }
}

pub fn additional_key(key: &str) -> bool {
    !matches!(key, "image" | "restart" | "ports" | "environment" | "<<")
        && !SectionKind::ALL.iter().any(|kind| kind.key() == key)
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-' | '.'))
}

fn scalar_keys(mapping: &Mapping) -> bool {
    unique_keys(mapping).is_ok() && mapping.iter().all(|(key, _)| key.is_scalar())
}

fn optional_scalar(map: &Mapping, key: &str) -> Option<Option<String>> {
    match map.get(key) {
        Some(node) if !empty_yaml_value(Some(&node)) => Some(Some(node.as_scalar()?.as_string())),
        _ => Some(None),
    }
}

fn bool_field(map: &Mapping, key: &str, default: bool) -> Option<bool> {
    match map.get(key) {
        Some(node) => node.as_scalar()?.as_bool(),
        None => Some(default),
    }
}

fn entry_yaml(source: &str, map: &Mapping, key: &str, node: &YamlNode) -> String {
    let fallback = || format!("{}: {}", string(key.to_owned()), node.to_string().trim());
    if map.is_flow_style() {
        return fallback();
    }
    let Some(entry) = map.entries().find_map(|entry| {
        let name = entry.key_node()?;
        (name.as_scalar()?.as_string() == key)
            .then(|| {
                name.as_node()?
                    .ancestors()
                    .find(|part| part.kind() == SyntaxKind::MAPPING_ENTRY)
            })
            .flatten()
    }) else {
        return fallback();
    };
    let range = entry.text_range();
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    let indent = source[..start]
        .rsplit_once('\n')
        .map_or(start, |(_, line)| line.len());
    let prefix = " ".repeat(indent);
    source[start..end]
        .lines()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                line
            } else {
                line.strip_prefix(&prefix).unwrap_or(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl ServiceSection {
    fn inspect(
        kind: SectionKind,
        node: Option<&YamlNode>,
        yaml: Option<String>,
        editable: bool,
    ) -> Self {
        let (rows, form_supported) = match node {
            None => (Vec::new(), true),
            Some(node) if empty_yaml_value(Some(node)) => (Vec::new(), true),
            Some(node) if node.as_sequence().is_some() => {
                let sequence = node.as_sequence().unwrap();
                (
                    sequence
                        .values()
                        .map(|node| sequence_row(kind, &node))
                        .collect(),
                    true,
                )
            }
            Some(node)
                if kind != SectionKind::Volumes
                    && node
                        .as_mapping()
                        .is_some_and(|map| scalar_keys(map) && !has_merge(map)) =>
            {
                let mapping = node.as_mapping().unwrap();
                (
                    mapping
                        .iter()
                        .map(|(key, value)| mapping_row(kind, detail(&key), &value))
                        .collect(),
                    true,
                )
            }
            _ => (Vec::new(), false),
        };
        Self {
            rows,
            yaml,
            editable,
            form_supported,
        }
    }
}

fn sequence_row(kind: SectionKind, node: &YamlNode) -> SectionRow {
    let text = node.as_scalar().map(|scalar| scalar.as_string());
    let (value, resource_ref, reference_unknown) = match kind {
        SectionKind::Networks => (
            text.clone()
                .filter(|key| !key.contains('$'))
                .map(RowValue::Network),
            text.clone(),
            text.as_ref().is_none_or(|key| key.contains('$')),
        ),
        SectionKind::Volumes => {
            let (mount, resource, unknown) = mount_row(node, text.as_deref());
            (mount.map(RowValue::Mount), resource, unknown)
        }
        SectionKind::Labels => (
            text.as_deref().map(|text| {
                let (key, value) = text
                    .split_once('=')
                    .map_or((text, None), |(key, value)| (key, Some(value.to_owned())));
                RowValue::Label(Variable {
                    key: key.into(),
                    value,
                })
            }),
            None,
            false,
        ),
        SectionKind::Dependencies => (
            text.clone()
                .filter(|key| !key.contains('$'))
                .map(|service| {
                    RowValue::Dependency(Dependency {
                        service,
                        ..Dependency::default()
                    })
                }),
            None,
            false,
        ),
    };
    let label = value.as_ref().map_or_else(|| detail(node), row_label);
    let mount_target = if kind == SectionKind::Volumes {
        if let Some(text) = &text {
            let parts = mount_parts(text);
            parts
                .get(1)
                .or(parts.first())
                .map(|target| (*target).to_owned())
        } else {
            node.as_mapping()
                .and_then(|map| optional_scalar(map, "target").flatten())
        }
    } else {
        None
    };
    SectionRow {
        label,
        value,
        resource_ref,
        reference_unknown,
        mount_target,
    }
}

fn mapping_row(kind: SectionKind, key: String, node: &YamlNode) -> SectionRow {
    let value = match kind {
        SectionKind::Networks => (!key.contains('$')).then(|| RowValue::Network(key.clone())),
        SectionKind::Labels => {
            if empty_yaml_value(Some(node)) {
                Some(RowValue::Label(Variable {
                    key: key.clone(),
                    value: None,
                }))
            } else {
                node.as_scalar().map(|scalar| {
                    RowValue::Label(Variable {
                        key: key.clone(),
                        value: Some(scalar.as_string()),
                    })
                })
            }
        }
        SectionKind::Dependencies => node.as_mapping().and_then(|map| {
            if !scalar_keys(map) || has_merge(map) || key.contains('$') {
                return None;
            }
            let condition =
                optional_scalar(map, "condition")?.unwrap_or_else(|| "service_started".into());
            CONDITIONS
                .contains(&condition.as_str())
                .then_some(RowValue::Dependency(Dependency {
                    service: key.clone(),
                    condition,
                    restart: bool_field(map, "restart", false)?,
                    required: bool_field(map, "required", true)?,
                }))
        }),
        SectionKind::Volumes => None,
    };
    let label = value.as_ref().map_or_else(|| key.clone(), row_label);
    SectionRow {
        label,
        value,
        resource_ref: (kind == SectionKind::Networks).then(|| key.clone()),
        reference_unknown: kind == SectionKind::Networks && key.contains('$'),
        mount_target: None,
    }
}

fn row_label(value: &RowValue) -> String {
    match value {
        RowValue::Network(key) => key.clone(),
        RowValue::Mount(mount) => {
            let mode = if mount.read_only { " · ro" } else { "" };
            format!("{} → {}{mode}", mount.source, mount.target)
        }
        RowValue::Label(label) => match label.value.as_deref() {
            Some("") => format!("{} = \"\"", label.key),
            Some(value) => format!("{} = {value}", label.key),
            None => label.key.clone(),
        },
        RowValue::Dependency(dependency) => {
            format!("{} · {}", dependency.service, dependency.condition)
        }
    }
}

pub const CONDITIONS: [&str; 3] = [
    "service_started",
    "service_healthy",
    "service_completed_successfully",
];

fn mount_row(node: &YamlNode, text: Option<&str>) -> (Option<Mount>, Option<String>, bool) {
    if let Some(text) = text {
        let parts = mount_parts(text);
        let [source, target, options @ ..] = parts.as_slice() else {
            return (None, None, text.contains('$'));
        };
        let dynamic = source.contains('$');
        let bind = bind_path(source);
        let reference = (!bind).then(|| (*source).to_owned());
        let mount = (!dynamic
            && options.len() <= 1
            && options
                .first()
                .is_none_or(|mode| matches!(*mode, "ro" | "rw")))
        .then(|| Mount {
            source: (*source).to_owned(),
            target: (*target).to_owned(),
            bind,
            read_only: options.first() == Some(&"ro"),
        });
        return (mount, reference, dynamic);
    }
    let parsed = (|| {
        let map = node.as_mapping()?;
        let mount_type = optional_scalar(map, "type")??;
        let source = optional_scalar(map, "source")?;
        Some((map, mount_type, source))
    })();
    let Some((map, mount_type, source)) = parsed else {
        return (None, None, true);
    };
    if !matches!(mount_type.as_str(), "volume" | "bind") {
        return (None, None, mount_type.contains('$'));
    }
    let reference = if mount_type == "volume" {
        source.clone()
    } else {
        None
    };
    let dynamic = reference.as_ref().is_some_and(|name| name.contains('$'));
    let mount = (|| {
        if dynamic || has_merge(map) || !scalar_keys(map) {
            return None;
        }
        Some(Mount {
            source: source?,
            target: optional_scalar(map, "target")??,
            bind: mount_type == "bind",
            read_only: bool_field(map, "read_only", false)?,
        })
    })();
    (mount, reference, dynamic || has_merge(map))
}

fn bind_path(source: &str) -> bool {
    source.starts_with(['.', '/', '~', '\\']) || source.as_bytes().get(1) == Some(&b':')
}

fn mount_parts(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut braces: usize = 0;
    for (index, value) in text.char_indices() {
        match value {
            '{' => braces += 1,
            '}' => braces = braces.saturating_sub(1),
            ':' if braces == 0 => {
                let drive = index == start + 1
                    && text.as_bytes()[start].is_ascii_alphabetic()
                    && matches!(text.as_bytes().get(index + 1), Some(b'/' | b'\\'));
                if !drive {
                    parts.push(&text[start..index]);
                    start = index + 1;
                }
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

#[derive(Clone, Debug)]
pub enum ResourceInput {
    Settings(ResourceSettings),
    Yaml(String),
}

#[derive(Clone, Debug)]
pub enum ProjectEdit {
    Resource {
        kind: ResourceKind,
        key: String,
        add: bool,
        value: Option<ResourceInput>,
    },
    Parameter {
        key: String,
        add: bool,
        yaml: Option<String>,
    },
    Row {
        kind: SectionKind,
        index: Option<usize>,
        value: Option<RowValue>,
        create_resource: bool,
    },
}

impl Compose {
    pub fn edit_project(&mut self, service: Option<usize>, edit: ProjectEdit) -> Result<()> {
        let file = parse(&self.source)?;
        let root = file
            .document()
            .and_then(|document| document.as_mapping())
            .ok_or(Error::Yaml)?;
        let change = match edit {
            ProjectEdit::Resource {
                kind,
                key,
                add,
                value,
            } => {
                edit_resource(
                    &root,
                    self.resources.list(kind),
                    kind,
                    &key,
                    add,
                    value.as_ref(),
                )?;
                parameter_change(format!("{}:", kind.key()), key, add, value.is_none())
            }
            ProjectEdit::Parameter { key, add, yaml } => {
                let service = self
                    .services
                    .get(service.ok_or(Error::Protected)?)
                    .ok_or(Error::Protected)?;
                let map = service_map(&file, &service.name)?;
                edit_parameter(service, &map, &key, add, yaml.as_deref())?;
                parameter_change(service.name.clone(), key, add, yaml.is_none())
            }
            ProjectEdit::Row {
                kind,
                index,
                value,
                create_resource,
            } => {
                let service = self
                    .services
                    .get(service.ok_or(Error::Protected)?)
                    .ok_or(Error::Protected)?;
                let resource = if let Some(value) = &value {
                    self.validate_row(service, value, create_resource)?
                } else if create_resource {
                    return Err(Error::Protected);
                } else {
                    None
                };
                if let Some((resource_kind, key)) = resource {
                    edit_resource(
                        &root,
                        self.resources.list(resource_kind),
                        resource_kind,
                        key,
                        true,
                        Some(&ResourceInput::Settings(ResourceSettings::default())),
                    )?;
                }
                let map = service_map(&file, &service.name)?;
                edit_row(service, &map, kind, index, value.as_ref())?;
                if let Some((resource_kind, key)) = resource {
                    Change::ResourceAssigned {
                        section: resource_kind.key().into(),
                        resource: key.into(),
                        service: service.name.clone(),
                    }
                } else {
                    parameter_change(
                        service.name.clone(),
                        kind.key().into(),
                        index.is_none(),
                        value.is_none(),
                    )
                }
            }
        };
        let candidate = keep_line_endings(&self.source, file.to_string());
        if self.source == candidate {
            return Ok(());
        }
        let (services, has_include, resources) = inspect(&parse(&candidate)?)?;
        self.history.discard_future();
        self.drafts.push(Draft {
            yaml: candidate.clone(),
            change,
        });
        self.source = candidate;
        self.environment_usage.take();
        self.services = services;
        self.has_include = has_include;
        self.resources = resources;
        Ok(())
    }

    fn validate_row<'a>(
        &self,
        service: &Service,
        value: &'a RowValue,
        create: bool,
    ) -> Result<Option<(ResourceKind, &'a str)>> {
        let invalid = || Error::Input("Gib einen gültigen Wert ein.", "Enter a valid value.");
        let resource = match value {
            RowValue::Network(key) => Some((ResourceKind::Network, key.as_str())),
            RowValue::Mount(mount) => {
                if mount.source.trim().is_empty()
                    || mount.target.trim().is_empty()
                    || mount.source.chars().any(char::is_control)
                    || mount.target.chars().any(char::is_control)
                    || (mount.bind && !bind_path(&mount.source))
                {
                    return Err(invalid());
                }
                (!mount.bind).then_some((ResourceKind::Volume, mount.source.as_str()))
            }
            RowValue::Label(label) => {
                if label.key.trim().is_empty()
                    || label.key.contains('=')
                    || label.key.chars().any(char::is_whitespace)
                {
                    return Err(invalid());
                }
                None
            }
            RowValue::Dependency(dependency) => {
                if service.name == dependency.service
                    || !self
                        .services
                        .iter()
                        .any(|service| service.name == dependency.service)
                    || !CONDITIONS.contains(&dependency.condition.as_str())
                {
                    return Err(invalid());
                }
                None
            }
        };
        if let Some((kind, key)) = resource {
            let list = self.resources.list(kind);
            if create {
                return Ok(resource);
            }
            if list.entries.iter().any(|entry| entry.key == key) {
                return Ok(None);
            }
            return Err(Error::Input(
                "Wähle eine vorhandene Definition aus.",
                "Select an existing definition.",
            ));
        }
        if create {
            return Err(Error::Protected);
        }
        Ok(None)
    }
}

fn parameter_change(owner: String, key: String, add: bool, remove: bool) -> Change {
    let operation = if remove {
        Operation::Removed
    } else if add {
        Operation::Added
    } else {
        Operation::Updated
    };
    Change::ComposeParameter {
        owner,
        key,
        operation,
    }
}

fn edit_resource(
    root: &Mapping,
    list: &ResourceList,
    kind: ResourceKind,
    key: &str,
    add: bool,
    value: Option<&ResourceInput>,
) -> Result<()> {
    let existing = list.entries.iter().find(|entry| entry.key == key);
    if add && existing.is_some() {
        return Err(Error::Duplicate(key.into()));
    }
    if add {
        if !valid_key(key) {
            return Err(Error::Input(
                "Der Compose-Schlüssel darf Buchstaben, Zahlen, Punkt, Bindestrich und Unterstrich enthalten.",
                "The Compose key may contain letters, digits, dots, hyphens, and underscores.",
            ));
        }
        if !list.can_add || value.is_none() {
            return Err(Error::Protected);
        }
    } else if !existing.is_some_and(|entry| entry.editable) {
        return Err(Error::Protected);
    }
    if value.is_none() {
        let entry = existing.ok_or(Error::Protected)?;
        if !entry.used_by.is_empty() {
            return Err(Error::ResourceInUse(entry.used_by.join(", ")));
        }
        if list.unknown_usage {
            return Err(Error::UsageUncertain);
        }
    }

    let map = get_or_create_mapping(root, kind.key())?;
    match value {
        None => {
            map.remove(key);
        }
        Some(ResourceInput::Yaml(yaml)) => {
            if existing.is_some_and(|entry| entry.yaml == *yaml) {
                return Ok(());
            }
            let node = parse_entry(key, yaml)?;
            if !node.is_mapping() && !empty_yaml_value(Some(&node)) {
                return Err(Error::Input(
                    "Erwartet wird eine Zuordnung oder null.",
                    "Expected a mapping or null.",
                ));
            }
            if let Some(inner) = node.as_mapping()
                && bool_field(inner, "external", false) == Some(true)
                && inner
                    .iter()
                    .any(|(key, _)| !matches!(detail(&key).as_str(), "external" | "name"))
            {
                return Err(external_options());
            }
            map.set(string(key.into()), node);
        }
        Some(ResourceInput::Settings(settings)) => {
            if add && settings == &ResourceSettings::default() {
                map.set(string(key.into()), ScalarValue::null());
            } else {
                let previous = existing
                    .map(|entry| entry.settings.as_ref().ok_or(Error::Protected))
                    .transpose()?;
                if previous == Some(settings) {
                    return Ok(());
                }
                update_resource_settings(&map, key, settings, previous)?;
            }
        }
    }
    Ok(())
}

fn external_options() -> Error {
    Error::Input(
        "Externe Definitionen erlauben neben „external“ nur „name“. Entferne weitere Optionen im YAML-Editor.",
        "External definitions allow only ‘name’ alongside ‘external’. Remove other options in the YAML editor.",
    )
}

fn update_resource_settings(
    map: &Mapping,
    key: &str,
    settings: &ResourceSettings,
    previous: Option<&ResourceSettings>,
) -> Result<()> {
    let existing = map.get_mapping(key);
    let inner = get_or_create_mapping(map, key)?;
    if settings.external
        && ((!settings.driver.is_empty() && previous.is_none())
            || inner
                .iter()
                .any(|(key, _)| !matches!(detail(&key).as_str(), "external" | "name" | "driver")))
    {
        return Err(external_options());
    }
    if previous.is_none_or(|previous| previous.name != settings.name) {
        update_text(&inner, "name", &settings.name);
    }
    if settings.external {
        inner.remove("driver");
    } else if previous.is_none_or(|previous| previous.driver != settings.driver) {
        update_text(&inner, "driver", &settings.driver);
    }
    if previous.is_none_or(|previous| previous.external != settings.external) {
        if settings.external {
            inner.set("external", true);
        } else {
            inner.remove("external");
        }
    }
    if existing.is_none() && inner.entries().next().is_none() {
        map.set(string(key.into()), ScalarValue::null());
    }
    Ok(())
}

fn update_text(map: &Mapping, key: &str, value: &str) {
    if value.is_empty() {
        map.remove(key);
    } else {
        map.set(key, string(value.into()));
    }
}

fn get_or_create_mapping(parent: &Mapping, key: &str) -> Result<Mapping> {
    if let Some(mapping) = parent.get_mapping(key) {
        if !mapping.is_flow_style() || mapping.entries().next().is_some() {
            return Ok(mapping);
        }
    } else if !empty_yaml_value(parent.get(key).as_ref()) {
        return Err(Error::Protected);
    }
    parent.set(string(key.into()), Mapping::new_pending_block());
    parent.get_mapping(key).ok_or(Error::Protected)
}

fn edit_parameter(
    service: &Service,
    map: &Mapping,
    key: &str,
    add: bool,
    yaml: Option<&str>,
) -> Result<()> {
    if add && map.contains_key(key) {
        return Err(Error::Duplicate(key.into()));
    }
    if let Some(kind) = SectionKind::ALL.into_iter().find(|kind| kind.key() == key) {
        if !service.parameters.section(kind).editable {
            return Err(Error::Protected);
        }
    } else if !additional_key(key) || key == "extends" {
        return Err(Error::Protected);
    } else if add {
        if !valid_key(key) || !service.parameters.can_add {
            return Err(Error::Input(
                "Gib einen gültigen Compose-Schlüssel ein.",
                "Enter a valid Compose key.",
            ));
        }
    } else if !service
        .parameters
        .additional
        .iter()
        .any(|parameter| parameter.key == key && parameter.editable)
    {
        return Err(Error::Protected);
    }
    match yaml {
        Some(yaml) => {
            let current_yaml = SectionKind::ALL
                .into_iter()
                .find(|kind| kind.key() == key)
                .and_then(|kind| service.parameters.section(kind).yaml.as_deref())
                .or_else(|| {
                    service
                        .parameters
                        .additional
                        .iter()
                        .find(|parameter| parameter.key == key)
                        .map(|parameter| parameter.yaml.as_str())
                });
            if current_yaml != Some(yaml) {
                map.set(string(key.into()), parse_entry(key, yaml)?);
            }
        }
        None if !add => {
            map.remove(key);
        }
        None => return Err(Error::Protected),
    }
    Ok(())
}

fn parse_entry(key: &str, yaml: &str) -> Result<YamlNode> {
    let file = parse(yaml)?;
    let map = file
        .document()
        .and_then(|document| document.as_mapping())
        .ok_or(Error::Yaml)?;
    unique_keys(&map)?;
    if map.entries().count() != 1 {
        return Err(Error::Input(
            "Gib genau den ausgewählten YAML-Eintrag ein.",
            "Enter exactly the selected YAML entry.",
        ));
    }
    let node = map.get(key).ok_or(Error::Input(
        "Der YAML-Schlüssel muss mit dem ausgewählten Parameter übereinstimmen.",
        "The YAML key must match the selected parameter.",
    ))?;
    if shared(&node, true) {
        return Err(Error::Protected);
    }
    check_nested_keys(&node)?;
    Ok(node)
}

fn check_nested_keys(node: &YamlNode) -> Result<()> {
    if let Some(map) = node.as_mapping() {
        unique_keys(map)?;
        for (_, value) in map.iter() {
            check_nested_keys(&value)?;
        }
    }
    if let Some(sequence) = node.as_sequence() {
        for item in sequence.values() {
            check_nested_keys(&item)?;
        }
    }
    Ok(())
}

fn edit_row(
    service: &Service,
    map: &Mapping,
    kind: SectionKind,
    index: Option<usize>,
    value: Option<&RowValue>,
) -> Result<()> {
    let section = service.parameters.section(kind);
    if !section.editable
        || !section.form_supported
        || value.is_some_and(|value| value.kind() != kind)
    {
        return Err(Error::Protected);
    }
    let previous = match index {
        Some(index) => Some(
            section
                .rows
                .get(index)
                .and_then(|row| row.value.as_ref())
                .ok_or(Error::Protected)?,
        ),
        None => None,
    };
    if previous == value {
        return Ok(());
    }
    let key = kind.key();
    if let Some(value) = value {
        if section.rows.iter().enumerate().any(|(i, row)| {
            Some(i) != index
                && row
                    .value
                    .as_ref()
                    .is_some_and(|other| other.identity() == value.identity())
        }) {
            return Err(Error::Duplicate(value.identity().into()));
        }
        if let Some(entries) = map.get_mapping(key) {
            set_mapping_row(&entries, previous, value)?;
        } else if let RowValue::Dependency(dependency) = value
            && needs_long_dependency(dependency)
        {
            convert_dependencies(map, section, index, dependency)?;
        } else {
            let sequence = get_or_create_sequence(map, key)?;
            set_sequence_row(&sequence, index, value)?;
        }
    } else if section.rows.len() == 1 {
        map.remove(key);
    } else if let Some(entries) = map.get_mapping(key) {
        entries
            .remove(previous.ok_or(Error::Protected)?.identity())
            .ok_or(Error::Protected)?;
    } else {
        map.get_sequence(key)
            .ok_or(Error::Protected)?
            .remove(index.ok_or(Error::Protected)?)
            .ok_or(Error::Protected)?;
    }
    Ok(())
}

fn get_or_create_sequence(map: &Mapping, key: &str) -> Result<Sequence> {
    if let Some(sequence) = map.get_sequence(key) {
        if !sequence.is_flow_style() || !sequence.is_empty() {
            return Ok(sequence);
        }
    } else if !empty_yaml_value(map.get(key).as_ref()) {
        return Err(Error::Protected);
    }
    map.set(key, Sequence::new_pending_block());
    map.get_sequence(key).ok_or(Error::Protected)
}

fn set_mapping_row(map: &Mapping, previous: Option<&RowValue>, value: &RowValue) -> Result<()> {
    let key = value.identity();
    let previous_key = previous.map(RowValue::identity);
    if previous_key != Some(key) && map.contains_key(key) {
        return Err(Error::Duplicate(key.into()));
    }
    if let Some(old) = previous_key.filter(|old| *old != key)
        && !map.rename_key(old, string(key.into()))
    {
        return Err(Error::Protected);
    }
    match value {
        RowValue::Label(label) => {
            if let Some(value) = &label.value {
                map.set(string(key.into()), string(value.clone()));
            } else {
                map.set(string(key.into()), ScalarValue::null());
            }
        }
        RowValue::Network(_) => {
            if previous.is_none() {
                map.set(string(key.into()), ScalarValue::null());
            }
        }
        RowValue::Dependency(dependency) => {
            let inner = map.get_mapping(key);
            if let Some(inner) = inner {
                let previous = previous
                    .and_then(|value| match value {
                        RowValue::Dependency(value) => Some(value),
                        _ => None,
                    })
                    .ok_or(Error::Protected)?;
                if dependency.condition != previous.condition {
                    inner.set("condition", dependency.condition.as_str());
                }
                if dependency.restart != previous.restart {
                    inner.set("restart", dependency.restart);
                }
                if dependency.required != previous.required {
                    inner.set("required", dependency.required);
                }
            } else {
                map.set(string(key.into()), dependency_mapping(dependency));
            }
        }
        RowValue::Mount(_) => return Err(Error::Protected),
    }
    Ok(())
}

fn set_sequence_row(sequence: &Sequence, index: Option<usize>, value: &RowValue) -> Result<()> {
    let text = match value {
        RowValue::Network(key) => key.clone(),
        RowValue::Label(label) => label.value.as_ref().map_or_else(
            || label.key.clone(),
            |value| format!("{}={value}", label.key),
        ),
        RowValue::Dependency(dependency) => dependency.service.clone(),
        RowValue::Mount(mount) => {
            let existing = index
                .and_then(|index| sequence.get(index))
                .and_then(|node| node.as_mapping().cloned());
            let mode = if mount.read_only { ":ro" } else { "" };
            let short = format!("{}:{}{mode}", mount.source, mount.target);
            if let Some(existing) = existing {
                let old_type = optional_scalar(&existing, "type")
                    .flatten()
                    .unwrap_or_default();
                let kind = if mount.bind { "bind" } else { "volume" };
                let incompatible = existing.contains_key(&old_type) && kind != old_type;
                if incompatible {
                    return Err(Error::Input(
                        "Ändere den Mount-Typ und seine erweiterten Optionen gemeinsam im YAML-Editor.",
                        "Change the mount type and its advanced options together in the YAML editor.",
                    ));
                }
                if old_type != kind {
                    existing.set("type", kind);
                }
                if optional_scalar(&existing, "source").flatten().as_deref() != Some(&mount.source)
                {
                    existing.set("source", string(mount.source.clone()));
                }
                if optional_scalar(&existing, "target").flatten().as_deref() != Some(&mount.target)
                {
                    existing.set("target", string(mount.target.clone()));
                }
                if bool_field(&existing, "read_only", false) != Some(mount.read_only) {
                    existing.set("read_only", mount.read_only);
                }
                return Ok(());
            }
            let parts = mount_parts(&short);
            if parts.get(..2) != Some(&[mount.source.as_str(), mount.target.as_str()]) {
                let inner = Mapping::new_pending_block();
                inner.set("type", if mount.bind { "bind" } else { "volume" });
                inner.set("source", string(mount.source.clone()));
                inner.set("target", string(mount.target.clone()));
                if mount.read_only {
                    inner.set("read_only", true);
                }
                if let Some(index) = index {
                    sequence.set(index, inner);
                } else {
                    sequence.push(inner);
                }
                return Ok(());
            }
            short
        }
    };
    if let Some(index) = index {
        sequence.set(index, string(text));
    } else {
        sequence.push(string(text));
    }
    Ok(())
}

fn needs_long_dependency(dependency: &Dependency) -> bool {
    dependency.condition != "service_started" || dependency.restart || !dependency.required
}

fn dependency_mapping(dependency: &Dependency) -> Mapping {
    let mapping = Mapping::new_pending_block();
    mapping.set("condition", dependency.condition.as_str());
    if dependency.restart {
        mapping.set("restart", true);
    }
    if !dependency.required {
        mapping.set("required", false);
    }
    mapping
}

fn convert_dependencies(
    service: &Mapping,
    section: &ServiceSection,
    index: Option<usize>,
    dependency: &Dependency,
) -> Result<()> {
    let mapping = Mapping::new_pending_block();
    for (i, row) in section.rows.iter().enumerate() {
        let previous = if Some(i) == index {
            dependency
        } else if let Some(RowValue::Dependency(previous)) = &row.value {
            previous
        } else {
            return Err(Error::Input(
                "Bearbeite Abhängigkeiten mit Platzhaltern im YAML-Editor.",
                "Use the YAML editor for dependencies with placeholders.",
            ));
        };
        if mapping.contains_key(previous.service.as_str()) {
            return Err(Error::Duplicate(previous.service.clone()));
        }
        mapping.set(
            string(previous.service.clone()),
            dependency_mapping(previous),
        );
    }
    if index.is_none() {
        mapping.set(
            string(dependency.service.clone()),
            dependency_mapping(dependency),
        );
    }
    service.set("depends_on", mapping);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(source: &str) -> (tempfile::TempDir, Compose) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compose.yaml");
        fs::write(&path, source).unwrap();
        (directory, Compose::open(&path).unwrap())
    }

    fn resource(
        kind: ResourceKind,
        key: &str,
        add: bool,
        value: Option<ResourceInput>,
    ) -> ProjectEdit {
        ProjectEdit::Resource {
            kind,
            key: key.into(),
            add,
            value,
        }
    }

    fn row(
        kind: SectionKind,
        index: Option<usize>,
        value: RowValue,
        create_resource: bool,
    ) -> ProjectEdit {
        ProjectEdit::Row {
            kind,
            index,
            value: Some(value),
            create_resource,
        }
    }

    fn parameter(key: &str, add: bool, yaml: &str) -> ProjectEdit {
        ProjectEdit::Parameter {
            key: key.into(),
            add,
            yaml: Some(yaml.into()),
        }
    }

    #[test]
    fn edits_appwrite_resources_and_service_settings_and_restores_the_saved_snapshot() {
        const APPWRITE: &str = include_str!("../../examples/appwrite.yml");
        let (_directory, mut document) = fixture(APPWRITE);
        let service = |document: &Compose, name: &str| {
            document
                .services
                .iter()
                .position(|service| service.name == name)
                .unwrap()
        };
        assert_eq!(
            document
                .resources
                .networks
                .entries
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>(),
            ["gateway", "appwrite", "runtimes", "browser"]
        );
        assert_eq!(document.resources.volumes.entries.len(), 18);
        let config = document
            .resources
            .volumes
            .entries
            .iter()
            .find(|entry| entry.key == "appwrite-config")
            .unwrap();
        assert!(config.used_by.iter().any(|name| name == "traefik"));
        assert!(config.used_by.iter().any(|name| name == "appwrite"));
        assert!(
            config
                .settings
                .as_ref()
                .is_some_and(|settings| settings.name.is_empty())
        );
        assert!(!document.resources.networks.unknown_usage);
        assert!(!document.resources.volumes.unknown_usage);

        let mongo = &document.services[service(&document, "mongodb")];
        let healthcheck = mongo
            .parameters
            .additional
            .iter()
            .find(|parameter| parameter.key == "healthcheck")
            .unwrap();
        let node = parse_entry("healthcheck", &healthcheck.yaml).unwrap();
        let original = node
            .as_mapping()
            .unwrap()
            .get("test")
            .unwrap()
            .as_scalar()
            .unwrap()
            .as_string();
        assert!(original.contains("${_APP_DB_ROOT_PASS}"));
        let mongo_index = service(&document, "mongodb");
        document
            .edit_project(
                Some(mongo_index),
                parameter(
                    "healthcheck",
                    false,
                    &healthcheck.yaml.replace("timeout: 10s", "timeout: 11s"),
                ),
            )
            .unwrap();

        let traefik = service(&document, "traefik");
        document
            .edit_project(
                Some(traefik),
                row(
                    SectionKind::Networks,
                    None,
                    RowValue::Network("extra".into()),
                    true,
                ),
            )
            .unwrap();
        document
            .edit_project(
                Some(traefik),
                row(
                    SectionKind::Volumes,
                    None,
                    RowValue::Mount(Mount {
                        source: "trace-data".into(),
                        target: "/traces".into(),
                        read_only: true,
                        bind: false,
                    }),
                    true,
                ),
            )
            .unwrap();
        document
            .edit_project(
                Some(traefik),
                row(
                    SectionKind::Dependencies,
                    Some(0),
                    RowValue::Dependency(Dependency {
                        service: "appwrite".into(),
                        condition: "service_healthy".into(),
                        ..Dependency::default()
                    }),
                    false,
                ),
            )
            .unwrap();
        let appwrite = service(&document, "appwrite");
        document
            .edit_project(
                Some(appwrite),
                row(
                    SectionKind::Labels,
                    Some(0),
                    RowValue::Label(Variable {
                        key: "traefik.enable".into(),
                        value: Some("false".into()),
                    }),
                    false,
                ),
            )
            .unwrap();
        let logging = &document.services[appwrite]
            .parameters
            .additional
            .iter()
            .find(|entry| entry.key == "logging")
            .unwrap()
            .yaml;
        document
            .edit_project(
                Some(appwrite),
                parameter(
                    "logging",
                    false,
                    &logging.replace("max-size: 10m", "max-size: 12m"),
                ),
            )
            .unwrap();
        assert_eq!(document.services.len(), 46);
        assert_eq!(document.drafts.len(), 6);
        assert!(matches!(
            document.drafts[1].change,
            Change::ResourceAssigned { .. }
        ));
        assert!(
            document
                .source
                .contains("    # The marker is APP_READINESS_MARKER")
        );
        assert!(document.source.contains("      - ${_APP_HTTP_PORT:-80}:80"));
        assert!(document.source.contains("Host: $${_APP_DOMAIN:-localhost}"));
        let mongo = &document.services[mongo_index];
        let healthcheck = mongo
            .parameters
            .additional
            .iter()
            .find(|parameter| parameter.key == "healthcheck")
            .unwrap();
        let new_node = parse_entry("healthcheck", &healthcheck.yaml).unwrap();
        let new = new_node
            .as_mapping()
            .unwrap()
            .get("test")
            .unwrap()
            .as_scalar()
            .unwrap()
            .as_string();
        assert_eq!(new, original);
        assert_group_member(&document, ResourceKind::Volume, "trace-data", "traefik");
        assert_group_member(&document, ResourceKind::Network, "extra", "traefik");

        let path = document.path.clone();
        document.save(path.clone()).unwrap();
        if std::env::var_os("COMPEDITOR_TEST_DOCKER_COMPOSE").is_some() {
            let result = std::process::Command::new("docker")
                .args(["compose", "-f"])
                .arg(&path)
                .args(["config", "--quiet", "--no-interpolate"])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let mut reopened = Compose::open(&path).unwrap();
        assert_group_member(&reopened, ResourceKind::Volume, "trace-data", "traefik");
        assert_eq!(reopened.history.versions.len(), 7);
        reopened.restore(Target::Saved(0)).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), APPWRITE);
        assert_eq!(reopened.resources.volumes.entries.len(), 18);
    }

    fn assert_group_member(document: &Compose, kind: ResourceKind, key: &str, service: &str) {
        let entry = document
            .resources
            .list(kind)
            .entries
            .iter()
            .find(|entry| entry.key == key)
            .unwrap();
        assert_eq!(entry.used_by, [service]);
    }

    #[test]
    fn retains_long_options_and_distinguishes_labels_and_mount_sources() {
        let (_directory, mut document) = fixture(
            r#"
services:
  api:
    image: nginx
    networks:
      bridge:
        aliases: [api-local]
    volumes:
      - C:\data:/windows
      - ./data:/host
      - /anonymous
      - type: volume
        source: data
        target: /data
        volume:
          nocopy: true
    labels:
      bare:
      empty: ""
    depends_on:
      setup:
        condition: service_completed_successfully
        restart: true
        x-extra: keep
  setup:
    image: busybox
networks:
  default: {}
  bridge:
    ipam:
      config:
        - subnet: 10.6.0.0/16
  other: null
volumes:
  data: null
  unused: null
"#,
        );
        assert_group_member(&document, ResourceKind::Network, "default", "setup");
        assert_group_member(&document, ResourceKind::Network, "bridge", "api");
        assert_group_member(&document, ResourceKind::Volume, "data", "api");
        assert!(!document.resources.volumes.unknown_usage);
        let values = document.services[0]
            .parameters
            .section(SectionKind::Labels)
            .rows
            .iter()
            .map(|row| row.value.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            vec![
                Some(RowValue::Label(Variable {
                    key: "bare".into(),
                    value: None
                })),
                Some(RowValue::Label(Variable {
                    key: "empty".into(),
                    value: Some(String::new())
                })),
            ]
        );
        document
            .edit_project(
                Some(0),
                row(
                    SectionKind::Networks,
                    Some(0),
                    RowValue::Network("other".into()),
                    false,
                ),
            )
            .unwrap();
        assert!(document.source.contains("aliases: [api-local]"));
        assert_group_member(&document, ResourceKind::Network, "other", "api");
        let volume = RowValue::Mount(Mount {
            source: "data".into(),
            target: "/stored".into(),
            bind: false,
            read_only: true,
        });
        document
            .edit_project(Some(0), row(SectionKind::Volumes, Some(3), volume, false))
            .unwrap();
        assert!(document.source.contains("nocopy: true"));
        assert!(document.source.contains("read_only: true"));
        document
            .edit_project(
                Some(0),
                row(
                    SectionKind::Dependencies,
                    Some(0),
                    RowValue::Dependency(Dependency {
                        service: "setup".into(),
                        condition: "service_healthy".into(),
                        restart: true,
                        required: false,
                    }),
                    false,
                ),
            )
            .unwrap();
        assert!(document.source.contains("x-extra: keep"));
        assert!(document.source.contains("required: false"));
        document
            .edit_project(
                None,
                resource(
                    ResourceKind::Network,
                    "bridge",
                    false,
                    Some(ResourceInput::Settings(ResourceSettings {
                        driver: "bridge".into(),
                        name: "bridge-name".into(),
                        external: false,
                    })),
                ),
            )
            .unwrap();
        assert!(document.source.contains("subnet: 10.6.0.0/16"));
        document
            .edit_project(None, resource(ResourceKind::Volume, "unused", false, None))
            .unwrap();
        assert_eq!(document.resources.volumes.entries.len(), 1);
        assert!(matches!(
            document.edit_project(None, resource(ResourceKind::Volume, "data", false, None)),
            Err(Error::ResourceInUse(_))
        ));
    }

    #[test]
    fn follows_local_extends_and_resolves_mount_overrides_in_usage_lists() {
        let (_directory, document) = fixture(
            r#"
services:
  base:
    image: nginx
    networks: [edge]
    volumes:
      - data:/work
      - shared:/shared
  child:
    extends: {service: base}
    volumes: [override:/work]
  joined:
    image: nginx
    network_mode: service:child
    volumes_from: [child:ro]
  implicit:
    image: nginx
    networks: []
networks: {edge: {}, default: {}}
volumes: {data: {}, shared: {}, override: {}}
"#,
        );
        let networks = &document.resources.networks.entries;
        assert_eq!(networks[0].used_by, ["base", "child", "joined"]);
        assert_eq!(networks[1].used_by, ["implicit"]);
        let volumes = &document.resources.volumes.entries;
        assert_eq!(volumes[0].used_by, ["base"]);
        assert_eq!(volumes[1].used_by, ["base", "child", "joined"]);
        assert_eq!(volumes[2].used_by, ["child", "joined"]);
    }

    #[test]
    fn protects_shared_and_dynamic_definitions_and_rolls_back_failed_combined_edits() {
        let (_directory, mut document) = fixture(
            r#"
services:
  api:
    image: nginx
    network_mode: host
    volumes:
      - ${MOUNT:-data}:/data
    labels: &shared_labels
      test: app
  worker:
    image: nginx
    labels: *shared_labels
  inherited:
    extends:
      service: api
networks:
  unused: null
volumes:
  data: null
"#,
        );
        let original = document.source.clone();
        assert!(matches!(
            document.edit_project(
                Some(0),
                row(
                    SectionKind::Networks,
                    None,
                    RowValue::Network("new".into()),
                    true
                )
            ),
            Err(Error::Protected)
        ));
        assert!(matches!(
            document.edit_project(None, resource(ResourceKind::Volume, "data", false, None)),
            Err(Error::UsageUncertain)
        ));
        assert!(matches!(
            document.edit_project(Some(1), parameter("labels", false, "labels: {}")),
            Err(Error::Protected)
        ));
        assert!(matches!(
            document.edit_project(Some(2), parameter("hostname", true, "hostname: worker")),
            Err(Error::Input(..))
        ));
        assert!(matches!(
            document.edit_project(
                Some(0),
                parameter(
                    "healthcheck",
                    true,
                    "healthcheck:\n  timeout: 5s\n  timeout: 6s"
                )
            ),
            Err(Error::Duplicate(_))
        ));
        assert!(
            document
                .edit_project(Some(0), parameter("logging", true, "logging: ["))
                .is_err()
        );
        assert!(
            document
                .edit_project(
                    Some(0),
                    parameter("logging", true, "logging: {}\nimage: wrong")
                )
                .is_err()
        );
        assert_eq!(document.source, original);
        assert!(document.drafts.is_empty());
        assert_eq!(document.resources.networks.entries.len(), 1);
    }

    #[test]
    fn adds_and_removes_parameters_and_resources_in_flow_and_empty_sections() {
        let (_directory, mut document) =
            fixture("services: {app: {image: nginx}, empty: {}}\nnetworks: {}\nvolumes:\n");
        document
            .edit_project(
                Some(0),
                row(
                    SectionKind::Networks,
                    None,
                    RowValue::Network("backend".into()),
                    true,
                ),
            )
            .unwrap();
        document
            .edit_project(
                Some(0),
                row(
                    SectionKind::Volumes,
                    None,
                    RowValue::Mount(Mount {
                        source: "data".into(),
                        target: "/data".into(),
                        ..Mount::default()
                    }),
                    true,
                ),
            )
            .unwrap();
        document
            .edit_project(
                Some(1),
                parameter("container_name", true, "container_name: \"service-name\""),
            )
            .unwrap();
        document
            .edit_project(
                Some(1),
                ProjectEdit::Parameter {
                    key: "container_name".into(),
                    add: false,
                    yaml: None,
                },
            )
            .unwrap();
        let path = document.path.clone();
        document.save(path).unwrap();
        assert_group_member(&document, ResourceKind::Network, "backend", "app");
        assert_group_member(&document, ResourceKind::Volume, "data", "app");
    }
}
