//! Agent Skills frontmatter with namespaced Sovra metadata. External YAML features
//! such as includes and environment interpolation are deliberately unavailable.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sovra_core::skills::{MAX_SKILL_BYTES, Skill, SkillError, SkillKind};

#[derive(Deserialize, Serialize)]
struct Frontmatter {
    name: String,
    description: String,
    #[serde(
        default,
        rename = "allowed-tools",
        skip_serializing_if = "String::is_empty"
    )]
    allowed_tools: String,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compatibility: Option<String>,
    /// The Agent Skills convention for "only run when asked", honoured on import.
    #[serde(
        default,
        rename = "disable-model-invocation",
        skip_serializing_if = "std::ops::Not::not"
    )]
    disable_model_invocation: bool,
}

pub(crate) fn decode(content: &str, directory_name: &str) -> Result<Skill, SkillError> {
    if content.len() > MAX_SKILL_BYTES {
        return Err(SkillError("SKILL.md exceeds 64 KB.".into()));
    }
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = content.split_inclusive('\n');
    if lines.next().is_none_or(|line| line.trim_end() != "---") {
        return Err(SkillError(
            "SKILL.md must start with YAML frontmatter delimited by --- lines.".into(),
        ));
    }
    let mut header = String::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        header.push_str(line);
    }
    if !closed {
        return Err(SkillError(
            "SKILL.md frontmatter has no closing --- line.".into(),
        ));
    }
    let options = serde_saphyr::options! {
        reject_unsupported_tags: true,
        budget: serde_saphyr::budget! {
            max_depth: 12,
            flow_nesting_limit: 12,
            max_events: 4096,
            max_nodes: 2048,
            max_aliases: 0,
            max_anchors: 0,
            max_documents: 1,
            max_total_scalar_bytes: MAX_SKILL_BYTES,
        },
    };
    let front: Frontmatter = serde_saphyr::from_str_with_options(&header, options)
        .map_err(|_| SkillError("Invalid or overly complex skill frontmatter. Check names, strings and metadata; YAML anchors and tags are not supported.".into()))?;
    if front.name != directory_name {
        return Err(SkillError(
            "The skill name must match its folder name.".into(),
        ));
    }
    let title = front
        .metadata
        .get("sovra-title")
        .cloned()
        .unwrap_or_else(|| front.name.replace('-', " "));
    let skill = Skill {
        name: front.name,
        title,
        description: front.description,
        category: front
            .metadata
            .get("sovra-category")
            .cloned()
            .unwrap_or_else(|| "Your skills".into()),
        aliases: front
            .metadata
            .get("sovra-aliases")
            .map(|value| value.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        kind: SkillKind::Procedure,
        instructions: lines.collect::<String>().trim().into(),
        tools: front
            .allowed_tools
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
        argument_hint: front
            .metadata
            .get("sovra-argument-hint")
            .cloned()
            .unwrap_or_default(),
        requires_input: boolean(&front.metadata, "sovra-requires-input", false)?,
        enabled: boolean(&front.metadata, "sovra-enabled", true)?,
        auto_apply: boolean(
            &front.metadata,
            "sovra-auto-apply",
            !front.disable_model_invocation,
        )?,
        builtin: false,
        license: front.license,
        compatibility: front.compatibility,
        metadata: front
            .metadata
            .into_iter()
            .filter(|(name, _)| {
                !matches!(
                    name.as_str(),
                    "sovra-title"
                        | "sovra-category"
                        | "sovra-aliases"
                        | "sovra-argument-hint"
                        | "sovra-requires-input"
                        | "sovra-enabled"
                        | "sovra-auto-apply"
                )
            })
            .collect(),
    };
    skill.validate()?;
    Ok(skill)
}

fn boolean(
    metadata: &BTreeMap<String, String>,
    name: &str,
    default: bool,
) -> Result<bool, SkillError> {
    match metadata.get(name).map(String::as_str) {
        None => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(_) => Err(SkillError(format!(
            "{name} must be the string true or false."
        ))),
    }
}

pub(crate) fn encode(skill: &Skill) -> Result<String, SkillError> {
    skill.validate()?;
    let mut front = Frontmatter {
        name: skill.name.clone(),
        description: skill.description.clone(),
        allowed_tools: skill.tools.join(" "),
        metadata: BTreeMap::from([
            ("sovra-title".into(), skill.title.clone()),
            ("sovra-category".into(), skill.category.clone()),
            ("sovra-aliases".into(), skill.aliases.join(" ")),
            ("sovra-argument-hint".into(), skill.argument_hint.clone()),
            (
                "sovra-requires-input".into(),
                skill.requires_input.to_string(),
            ),
            ("sovra-enabled".into(), skill.enabled.to_string()),
            ("sovra-auto-apply".into(), skill.auto_apply.to_string()),
        ]),
        license: skill.license.clone(),
        compatibility: skill.compatibility.clone(),
        disable_model_invocation: !skill.auto_apply,
    };
    for (key, value) in &skill.metadata {
        front
            .metadata
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    let header = serde_saphyr::to_string(&front)
        .map_err(|_| SkillError("Could not encode skill metadata.".into()))?;
    let result = format!("---\n{header}---\n\n{}\n", skill.instructions.trim());
    if result.len() > MAX_SKILL_BYTES {
        return Err(SkillError(
            "Skill metadata and instructions together exceed 64 KB.".into(),
        ));
    }
    Ok(result)
}
