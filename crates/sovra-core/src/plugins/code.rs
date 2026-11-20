//! Plugins that bring their own code. The code is one WebAssembly module that runs
//! in a sealed box: it sees nothing on the computer, and it can reach only the
//! internet addresses its plugin lists, after the person turns the plugin on.
//!
//! This module holds the publisher contract (`sovra-plugin.json` beside
//! `plugin.wasm` at a repository's root), its validation, and the rules for what an
//! update must ask again. Running the module is the `sovra-wasm` adapter's job.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{PluginPackage, error, sha256_hex, short_text};
use crate::mcp::{McpToolInfo, ToolOffer};
use crate::skills::SkillError;

/// The file that lists a plugin's tools and the addresses its code may contact.
pub const CODE_MANIFEST_PATH: &str = "sovra-plugin.json";
/// The plugin's code, a WebAssembly module, at the repository root.
pub const CODE_MODULE_PATH: &str = "plugin.wasm";
/// The `api_version` of a package that carries code: older hosts refuse it.
pub const CODE_API_VERSION: u32 = 2;
/// Room for a useful tool written in Rust or another compiled language.
pub const MAX_MODULE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CODE_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_TOOLS: usize = 16;
pub const MAX_NETWORK_HOSTS: usize = 8;
const MAX_TOOL_NAME_BYTES: usize = 48;
const MAX_SCHEMA_BYTES: usize = 16 * 1024;
/// Tool calls for code plugins are routed by this start of their service id.
pub const PLUGIN_SERVICE_PREFIX: &str = "plugin:";
const WASM_MAGIC: &[u8; 4] = b"\0asm";

/// One tool the plugin's code answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginTool {
    /// What the module receives as the tool name: lowercase letters, digits and `_`.
    pub name: String,
    /// Plain words for people, shown under **What's included**.
    pub title: String,
    /// When the helper should use the tool; the model reads it.
    pub description: String,
    /// A JSON Schema object describing the arguments.
    pub input_schema: Value,
}

/// `sovra-plugin.json` as a publisher writes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeManifest {
    /// What the plugin does, in one or two sentences, when no
    /// `.claude-plugin/plugin.json` describes it.
    #[serde(default)]
    pub description: Option<String>,
    pub tools: Vec<PluginTool>,
    /// Host names the code may contact over HTTPS, like `api.example.com`.
    #[serde(default)]
    pub network: Vec<String>,
}

/// The code a package carries, with what it may do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCode {
    /// The module bytes in standard base64, so the package stays one JSON document.
    pub module: String,
    pub module_sha256: String,
    pub tools: Vec<PluginTool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network: Vec<String>,
}

impl CodeManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, SkillError> {
        if bytes.len() > MAX_CODE_MANIFEST_BYTES {
            return Err(error("sovra-plugin.json is larger than 64 KB."));
        }
        let manifest: Self = serde_json::from_slice(bytes).map_err(|problem| {
            SkillError(format!(
                "sovra-plugin.json could not be read: {problem}. Check it against the plugin guide."
            ))
        })?;
        check_tools(&manifest.tools)?;
        check_network(&manifest.network)?;
        Ok(manifest)
    }

    /// Joins the manifest with the module it describes.
    pub fn with_module(self, module: &[u8]) -> Result<PluginCode, SkillError> {
        check_module(module)?;
        check_interface(module, !self.network.is_empty())?;
        Ok(PluginCode {
            module: encode_base64(module),
            module_sha256: sha256_hex(module),
            tools: self.tools,
            network: self.network,
        })
    }
}

impl PluginCode {
    pub fn validate(&self) -> Result<(), SkillError> {
        check_tools(&self.tools)?;
        check_network(&self.network)?;
        let module = self.module_bytes()?;
        if sha256_hex(&module) != self.module_sha256 {
            return Err(error(
                "The plugin's code does not match its checksum. Nothing of it was used.",
            ));
        }
        Ok(())
    }

    /// The module bytes, checked to be a WebAssembly module within the size limit.
    pub fn module_bytes(&self) -> Result<Vec<u8>, SkillError> {
        let module = decode_base64(&self.module)
            .ok_or_else(|| error("The plugin's code is not stored correctly."))?;
        check_module(&module)?;
        Ok(module)
    }

    /// Plain sentences for the turn-on question and **What's included**.
    pub fn access_summary(&self) -> String {
        if self.network.is_empty() {
            "It runs in a sealed box. It cannot see your files or programs and cannot use the internet.".into()
        } else {
            format!(
                "It runs in a sealed box. It cannot see your files or programs. It can contact only: {}.",
                self.network.join(", ")
            )
        }
    }
}

/// The service id a code plugin's tools are offered under.
pub fn code_service_id(plugin_id: &str) -> String {
    format!("{PLUGIN_SERVICE_PREFIX}{plugin_id}")
}

impl PluginPackage {
    /// The code plugin's tools as chat tools; empty for a plugin without code.
    pub fn tool_offers(&self) -> Vec<ToolOffer> {
        let Some(code) = &self.code else {
            return Vec::new();
        };
        code.tools
            .iter()
            .map(|tool| ToolOffer {
                service_id: code_service_id(&self.id),
                service_name: self.title.clone(),
                tool: McpToolInfo {
                    name: tool.name.clone(),
                    title: tool.title.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.input_schema.clone(),
                },
            })
            .collect()
    }
}

/// What changed in a plugin's code between two packages, in plain words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeChanges {
    /// The newer package brings code where the installed one had none.
    pub code_added: bool,
    pub code_removed: bool,
    /// Same tools and access, different module.
    pub code_changed: bool,
    pub tools_added: Vec<String>,
    pub tools_removed: Vec<String>,
    pub network_added: Vec<String>,
    pub network_removed: Vec<String>,
}

impl CodeChanges {
    pub fn between(old: Option<&PluginCode>, new: Option<&PluginCode>) -> Self {
        let empty = Vec::new();
        let old_tools = old.map_or(&empty, |code| &code.tools);
        let new_tools = new.map_or(&empty, |code| &code.tools);
        let empty_hosts = Vec::new();
        let old_hosts = old.map_or(&empty_hosts, |code| &code.network);
        let new_hosts = new.map_or(&empty_hosts, |code| &code.network);
        Self {
            code_added: old.is_none() && new.is_some(),
            code_removed: old.is_some() && new.is_none(),
            code_changed: matches!((old, new), (Some(old), Some(new)) if old.module_sha256 != new.module_sha256),
            tools_added: new_tools
                .iter()
                .filter(|tool| !old_tools.iter().any(|known| known.name == tool.name))
                .map(|tool| tool.title.clone())
                .collect(),
            tools_removed: old_tools
                .iter()
                .filter(|tool| !new_tools.iter().any(|next| next.name == tool.name))
                .map(|tool| tool.title.clone())
                .collect(),
            network_added: new_hosts
                .iter()
                .filter(|host| !old_hosts.contains(host))
                .cloned()
                .collect(),
            network_removed: old_hosts
                .iter()
                .filter(|host| !new_hosts.contains(host))
                .cloned()
                .collect(),
        }
    }

    /// An update that lets code run for the first time, or reach somewhere new, is
    /// installed switched off so the person approves the new access themselves.
    pub fn needs_approval(&self) -> bool {
        self.code_added || !self.network_added.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

fn check_module(module: &[u8]) -> Result<(), SkillError> {
    if module.len() > MAX_MODULE_BYTES {
        return Err(error("plugin.wasm is larger than 2 MB."));
    }
    if !module.starts_with(WASM_MAGIC) {
        return Err(error(
            "plugin.wasm is not a WebAssembly module. Build it for wasm32-unknown-unknown.",
        ));
    }
    Ok(())
}

/// Functions Sovra links into plugin code, from the `sovra` import module.
const IMPORT_MODULE: &str = "sovra";
const NETWORK_IMPORTS: [&str; 2] = ["fetch", "take_reply"];
const REQUIRED_EXPORTS: [(&str, u8); 3] = [("memory", 2), ("sovra_alloc", 0), ("sovra_call", 0)];

/// Checks what the module asks for and offers, so a module the sandbox would
/// refuse is refused when the plugin is added, in the same words. The sandbox
/// still validates the whole module before running it.
fn check_interface(module: &[u8], network: bool) -> Result<(), SkillError> {
    let refused = |reason: String| SkillError(format!("The plugin's code cannot run: {reason}."));
    let interface = read_interface(module)
        .ok_or_else(|| refused("plugin.wasm is not a valid WebAssembly module".into()))?;
    for (module_name, name, is_function) in &interface.imports {
        let allowed = module_name == IMPORT_MODULE
            && *is_function
            && (name == "log" || (network && NETWORK_IMPORTS.contains(&name.as_str())));
        if !allowed {
            return Err(refused(
                if module_name == IMPORT_MODULE
                    && NETWORK_IMPORTS.contains(&name.as_str())
                    && !network
                {
                    "it asks for the internet but its sovra-plugin.json lists no addresses".into()
                } else {
                    format!(
                        "it asks for \"{module_name}.{name}\", which Sovra does not give plugin code"
                    )
                },
            ));
        }
    }
    for (name, kind) in REQUIRED_EXPORTS {
        if !interface
            .exports
            .iter()
            .any(|(known, k)| known == name && *k == kind)
        {
            return Err(refused(format!("it does not export \"{name}\"")));
        }
    }
    Ok(())
}

#[derive(Default)]
struct Interface {
    /// `(module, name, is a function)`.
    imports: Vec<(String, String, bool)>,
    /// `(name, kind)`: 0 function, 1 table, 2 memory, 3 global, 4 tag.
    exports: Vec<(String, u8)>,
}

/// Reads only the import and export sections of a WebAssembly binary.
fn read_interface(module: &[u8]) -> Option<Interface> {
    let mut reader = Reader {
        bytes: module.get(8..)?,
    };
    let mut interface = Interface::default();
    while !reader.bytes.is_empty() {
        let id = reader.byte()?;
        let size = usize::try_from(reader.leb()?).ok()?;
        let mut section = Reader {
            bytes: reader.take(size)?,
        };
        match id {
            2 => {
                for _ in 0..section.leb()? {
                    let module_name = section.name()?;
                    let name = section.name()?;
                    let kind = section.byte()?;
                    interface.imports.push((module_name, name, kind == 0));
                    if kind != 0 {
                        // Any other import is refused, so it need not be skipped.
                        return Some(interface);
                    }
                    section.leb()?;
                }
            }
            7 => {
                for _ in 0..section.leb()? {
                    let name = section.name()?;
                    let kind = section.byte()?;
                    section.leb()?;
                    interface.exports.push((name, kind));
                }
            }
            _ => {}
        }
    }
    Some(interface)
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.bytes.split_at_checked(count)?;
        self.bytes = rest;
        Some(taken)
    }

    fn byte(&mut self) -> Option<u8> {
        self.take(1).map(|bytes| bytes[0])
    }

    /// An unsigned LEB128 number of at most 64 bits.
    fn leb(&mut self) -> Option<u64> {
        let mut value = 0_u64;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn name(&mut self) -> Option<String> {
        let length = usize::try_from(self.leb()?).ok()?;
        String::from_utf8(self.take(length)?.to_vec()).ok()
    }
}

fn check_tools(tools: &[PluginTool]) -> Result<(), SkillError> {
    if tools.is_empty() || tools.len() > MAX_TOOLS {
        return Err(error("sovra-plugin.json must list between 1 and 16 tools."));
    }
    for (index, tool) in tools.iter().enumerate() {
        if !valid_tool_name(&tool.name)
            || tools[..index].iter().any(|known| known.name == tool.name)
        {
            return Err(SkillError(format!(
                "The tool \"{}\" needs a unique name of lowercase letters, digits and underscores (48 at most), starting with a letter.",
                tool.name
            )));
        }
        if !short_text(&tool.title, 120) || !short_text(&tool.description, 1000) {
            return Err(SkillError(format!(
                "The tool \"{}\" needs a short title and a description of what it does.",
                tool.name
            )));
        }
        let schema_size = serde_json::to_vec(&tool.input_schema).map_or(usize::MAX, |b| b.len());
        if tool.input_schema.get("type").and_then(Value::as_str) != Some("object")
            || schema_size > MAX_SCHEMA_BYTES
        {
            return Err(SkillError(format!(
                "The tool \"{}\" needs an input_schema of \"type\": \"object\", at most 16 KB.",
                tool.name
            )));
        }
    }
    Ok(())
}

fn valid_tool_name(name: &str) -> bool {
    name.len() <= MAX_TOOL_NAME_BYTES
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn check_network(hosts: &[String]) -> Result<(), SkillError> {
    if hosts.len() > MAX_NETWORK_HOSTS {
        return Err(error(
            "sovra-plugin.json can list at most 8 internet addresses.",
        ));
    }
    for (index, host) in hosts.iter().enumerate() {
        if !valid_host(host) || hosts[..index].contains(host) {
            return Err(SkillError(format!(
                "\"{host}\" is not a host name Sovra can allow. List each host once, like api.example.com, without https:// or a path."
            )));
        }
    }
    Ok(())
}

/// A public DNS name: lowercase labels joined by dots, no IP addresses, ports,
/// wildcards or this computer's own names.
pub fn valid_host(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    host.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        // A last label of digits only is an IP address, not a name.
        && labels
            .last()
            .is_some_and(|label| !label.bytes().all(|b| b.is_ascii_digit()))
        && !matches!(
            labels.last().copied(),
            Some("localhost" | "local" | "internal" | "lan" | "home" | "arpa")
        )
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = chunk.iter().enumerate().fold(0_u32, |acc, (index, byte)| {
            acc | (u32::from(*byte) << (16 - 8 * index))
        });
        for index in 0..4 {
            if index <= chunk.len() {
                let sextet = (triple >> (18 - 6 * index)) & 0x3f;
                text.push(char::from(BASE64[sextet as usize]));
            } else {
                text.push('=');
            }
        }
    }
    text
}

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (position, chunk) in bytes.chunks(4).enumerate() {
        let last = position + 1 == bytes.len() / 4;
        let padding = chunk.iter().rev().take_while(|b| **b == b'=').count();
        if padding > 2 || (padding > 0 && !last) {
            return None;
        }
        let mut triple = 0_u32;
        for (index, byte) in chunk.iter().enumerate() {
            let value = if index >= 4 - padding {
                0
            } else {
                u32::try_from(BASE64.iter().position(|known| known == byte)?).ok()?
            };
            triple |= value << (18 - 6 * index);
        }
        let [_, first, second, third] = triple.to_be_bytes();
        out.push(first);
        if padding < 2 {
            out.push(second);
        }
        if padding < 1 {
            out.push(third);
        }
    }
    // Only the one canonical spelling of each module is accepted.
    (encode_base64(&out) == text).then_some(out)
}

/// Largest request body plugin code may send, and largest reply it is given.
pub const MAX_FETCH_BODY_BYTES: usize = 256 * 1024;
pub const MAX_FETCH_REPLY_BYTES: usize = 1024 * 1024;
/// Requests one tool call may make, so a loop cannot hammer a service.
pub const MAX_FETCHES_PER_CALL: usize = 8;
/// Headers the computer sets itself; code may not choose them.
const RESERVED_HEADERS: [&str; 8] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
    "cookie",
    "proxy-authorization",
    "te",
];

/// What plugin code asks to send to one of its allowed addresses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeFetch {
    pub url: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
}

fn default_method() -> String {
    "GET".into()
}

/// The service's answer, as plugin code receives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeFetchReply {
    pub status: u16,
    pub body: String,
}

/// Checks a request against the plugin's allowed hosts before anything is sent.
/// Only HTTPS on the default port to a listed host, with no credentials in the
/// address, a plain method, bounded headers and body.
pub fn check_fetch(request: &CodeFetch, allowed: &[String]) -> Result<url::Url, String> {
    let url = url::Url::parse(&request.url)
        .map_err(|_| "That is not a web address this plugin may use.".to_owned())?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !allowed.iter().any(|known| known == host)
    {
        return Err(format!(
            "This plugin may only contact {} over https.",
            if allowed.is_empty() {
                "nothing".to_owned()
            } else {
                allowed.join(", ")
            }
        ));
    }
    if !matches!(
        request.method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
    ) {
        return Err("Plugin code can use GET, POST, PUT, PATCH or DELETE.".into());
    }
    if request.body.len() > MAX_FETCH_BODY_BYTES || request.headers.len() > 32 {
        return Err("The plugin's request is too large.".into());
    }
    for (name, value) in &request.headers {
        let lower = name.to_ascii_lowercase();
        if name.is_empty()
            || name.len() > 64
            || value.len() > 4096
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || value.chars().any(char::is_control)
            || RESERVED_HEADERS.contains(&lower.as_str())
        {
            return Err(format!("Plugin code cannot set the header \"{name}\"."));
        }
    }
    Ok(url)
}

/// Whether plugin code may connect to `address` once its host name is looked up.
/// A listed name that points at this computer or the local network is refused,
/// so a plugin cannot reach a router, printer or another program on this machine.
pub fn public_address(address: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match address {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                // Shared carrier space, protocol assignments, 6to4 relays,
                // benchmarking and reserved ranges.
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && v4.octets()[2] == 0)
                || (a == 192 && b == 88 && v4.octets()[2] == 99)
                || (a == 198 && (b == 18 || b == 19))
                || a == 0
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_address(IpAddr::V4(v4));
            }
            let segments = v6.segments();
            let first = segments[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique local fc00::/7, link-local fe80::/10, old site-local
                // fec0::/10, documentation 2001:db8::/32.
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first & 0xffc0) == 0xfec0
                || (first == 0x2001 && segments[1] == 0x0db8)
                // Forms that carry an IPv4 address a gateway may deliver to the
                // local network: IPv4-compatible ::/96, NAT64 64:ff9b::/96 and
                // 64:ff9b:1::/48, and 6to4 2002::/16. None is needed to reach a
                // public service by name.
                || segments[..6] == [0; 6]
                || (first == 0x0064 && segments[1] == 0xff9b)
                || first == 0x2002)
        }
    }
}

/// What the runtime gives a running tool call: the network, through the checks above.
pub trait CodeHost: Send + Sync {
    fn fetch(&self, request: CodeFetch) -> Result<CodeFetchReply, String>;
}

/// A plugin module ready to answer tool calls. Each call starts from a fresh copy,
/// so nothing carries over from one call (or one chat) to the next.
pub trait LoadedCode: Send + Sync + std::fmt::Debug {
    /// Runs one tool. Blocking: the runtime calls it off its async threads. The
    /// error is a plain sentence the helper can pass on.
    fn call(
        &self,
        tool: &str,
        arguments: &Value,
        host: Arc<dyn CodeHost>,
    ) -> Result<String, String>;
}

/// The sealed box. `network` is whether the plugin listed any addresses; code that
/// asks for the network without listing one is refused when it loads.
pub trait PluginCodeEngine: Send + Sync + std::fmt::Debug {
    fn load(&self, module: &[u8], network: bool) -> Result<Arc<dyn LoadedCode>, SkillError>;
}

#[cfg(test)]
/// The smallest module Sovra accepts: a memory and the two entry points.
pub(crate) fn test_module() -> Vec<u8> {
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    // Type section: one function type, () -> ().
    bytes.extend([1, 4, 1, 0x60, 0, 0]);
    // Function section: two functions of type 0.
    bytes.extend([3, 3, 2, 0, 0]);
    // Memory section: one memory of one page.
    bytes.extend([5, 3, 1, 0, 1]);
    let mut exports = vec![3];
    for (name, kind, index) in [
        ("memory", 2_u8, 0_u8),
        ("sovra_alloc", 0, 0),
        ("sovra_call", 0, 1),
    ] {
        exports.push(u8::try_from(name.len()).unwrap());
        exports.extend(name.bytes());
        exports.extend([kind, index]);
    }
    bytes.push(7);
    bytes.push(u8::try_from(exports.len()).unwrap());
    bytes.extend(exports);
    // Code section: two empty bodies.
    bytes.extend([10, 7, 2, 2, 0, 0x0b, 2, 0, 0x0b]);
    bytes
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tool(name: &str) -> PluginTool {
        PluginTool {
            name: name.into(),
            title: format!("Do {name}"),
            description: format!("Use to {name}."),
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    fn module() -> Vec<u8> {
        test_module()
    }

    fn with_import(module_name: &str, name: &str) -> Vec<u8> {
        let mut bytes = module();
        let mut section = vec![1];
        for part in [module_name, name] {
            section.push(u8::try_from(part.len()).unwrap());
            section.extend(part.bytes());
        }
        section.extend([0, 0]);
        // Imports come right after the type section (8 + 6 bytes).
        let at = 14;
        let mut header = vec![2, u8::try_from(section.len()).unwrap()];
        header.extend(section);
        bytes.splice(at..at, header);
        bytes
    }

    #[test]
    fn the_module_interface_is_checked_when_the_plugin_is_added() {
        assert!(check_interface(&module(), false).is_ok());
        assert!(check_interface(&with_import("sovra", "log"), false).is_ok());
        let wasi = check_interface(&with_import("wasi_snapshot_preview1", "fd_write"), true);
        assert!(
            wasi.unwrap_err()
                .0
                .contains("wasi_snapshot_preview1.fd_write")
        );
        let fetch = check_interface(&with_import("sovra", "fetch"), false);
        assert!(fetch.unwrap_err().0.contains("lists no addresses"));
        assert!(check_interface(&with_import("sovra", "fetch"), true).is_ok());
        assert!(check_interface(&with_import("sovra", "spawn"), true).is_err());
        let mut no_exports = module();
        no_exports.truncate(8);
        assert!(
            check_interface(&no_exports, false)
                .unwrap_err()
                .0
                .contains("memory")
        );
        let mut cut = module();
        cut.truncate(cut.len() - 3);
        assert!(
            check_interface(&cut, false)
                .unwrap_err()
                .0
                .contains("not a valid")
        );
    }

    #[test]
    fn base64_round_trips_every_length_and_rejects_other_spellings() {
        for length in 0..40_u8 {
            let bytes: Vec<u8> = (0..length).map(|i| i.wrapping_mul(37)).collect();
            let text = encode_base64(&bytes);
            assert_eq!(decode_base64(&text), Some(bytes), "{text}");
        }
        assert_eq!(encode_base64(b"Man"), "TWFu");
        assert_eq!(encode_base64(b"Ma"), "TWE=");
        assert_eq!(encode_base64(b"M"), "TQ==");
        for bad in ["TWF", "TW=u", "TQ==TWFu", "TR==", "T@Fu", "===="] {
            assert_eq!(decode_base64(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_manifest_with_a_module_becomes_checked_code() {
        let manifest = CodeManifest::parse(
            br#"{"tools": [{"name": "add_days", "title": "Add days to a date",
                "description": "Use to find a later date.",
                "input_schema": {"type": "object"}}],
                "network": ["api.example.com"]}"#,
        )
        .unwrap();
        let code = manifest.with_module(&module()).unwrap();
        code.validate().unwrap();
        assert_eq!(code.module_bytes().unwrap(), module());
        assert!(code.access_summary().contains("only: api.example.com"));

        let mut tampered = code;
        tampered.module = encode_base64(b"\0asm\x01\0\0\x01");
        assert!(
            tampered.validate().is_err(),
            "the checksum covers the module"
        );
        assert!(
            CodeManifest::parse(br#"{"tools": []}"#.as_slice())
                .unwrap_err()
                .0
                .contains("between 1 and 16")
        );
        assert!(
            CodeManifest::parse(br#"{"tools": [], "run": "sh"}"#.as_slice())
                .unwrap_err()
                .0
                .contains("could not be read"),
            "unknown fields are refused"
        );
        assert!(
            CodeManifest {
                description: None,
                tools: vec![tool("a")],
                network: Vec::new(),
            }
            .with_module(b"not wasm")
            .unwrap_err()
            .0
            .contains("not a WebAssembly module")
        );
    }

    #[test]
    fn tool_names_schemas_and_hosts_are_bounded() {
        assert!(check_tools(&[tool("days_between"), tool("weekday")]).is_ok());
        for bad in ["Days", "1day", "add-days", "", &"a".repeat(49)] {
            assert!(check_tools(&[tool(bad)]).is_err(), "{bad}");
        }
        assert!(check_tools(&[tool("same"), tool("same")]).is_err());
        let mut array_schema = tool("list");
        array_schema.input_schema = json!({"type": "array"});
        assert!(check_tools(&[array_schema]).is_err());

        for good in ["api.example.com", "example.co.uk", "a-b.example"] {
            assert!(valid_host(good), "{good}");
        }
        for bad in [
            "localhost",
            "printer.local",
            "10.0.0.1",
            "127.0.0.1",
            "[::1]",
            "*.example.com",
            "https://api.example.com",
            "api.example.com:8443",
            "API.example.com",
            "example",
            "-bad.example.com",
            "api.example.com/path",
        ] {
            assert!(!valid_host(bad), "{bad}");
        }
        assert!(check_network(&["a.example".into(), "a.example".into()]).is_err());
    }

    #[test]
    fn new_code_or_new_addresses_need_approval_but_a_new_build_does_not() {
        let code = |sha: &str, hosts: &[&str]| PluginCode {
            module: String::new(),
            module_sha256: sha.into(),
            tools: vec![tool("add_days")],
            network: hosts.iter().map(|host| (*host).to_owned()).collect(),
        };
        let first = CodeChanges::between(None, Some(&code("a", &[])));
        assert!(first.code_added && first.needs_approval());
        assert_eq!(first.tools_added, ["Do add_days"]);

        let rebuilt = CodeChanges::between(Some(&code("a", &[])), Some(&code("b", &[])));
        assert!(rebuilt.code_changed && !rebuilt.needs_approval());

        let reach = CodeChanges::between(
            Some(&code("a", &["a.example"])),
            Some(&code("a", &["a.example", "b.example"])),
        );
        assert_eq!(reach.network_added, ["b.example"]);
        assert!(reach.needs_approval());

        let narrower =
            CodeChanges::between(Some(&code("a", &["a.example"])), Some(&code("a", &[])));
        assert_eq!(narrower.network_removed, ["a.example"]);
        assert!(!narrower.needs_approval());
        assert!(CodeChanges::between(None, None).is_empty());
    }

    #[test]
    fn code_reaches_only_its_listed_hosts_over_https() {
        let allowed = vec!["api.example.com".to_owned()];
        let request = |url: &str| CodeFetch {
            url: url.into(),
            method: "GET".into(),
            headers: BTreeMap::new(),
            body: String::new(),
        };
        assert!(check_fetch(&request("https://api.example.com/v1?q=1"), &allowed).is_ok());
        for refused in [
            "http://api.example.com/",
            "https://api.example.com:8443/",
            "https://user:pw@api.example.com/",
            "https://evil.example.com/",
            "https://api.example.com.evil.net/",
            "https://127.0.0.1/",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(
                check_fetch(&request(refused), &allowed).is_err(),
                "{refused}"
            );
        }
        assert!(check_fetch(&request("https://api.example.com/"), &[]).is_err());

        let mut odd = request("https://api.example.com/");
        odd.method = "CONNECT".into();
        assert!(check_fetch(&odd, &allowed).is_err());
        for header in ["Host", "cookie", "Transfer-Encoding", "bad header"] {
            let mut with_header = request("https://api.example.com/");
            with_header.headers.insert(header.into(), "x".into());
            assert!(check_fetch(&with_header, &allowed).is_err(), "{header}");
        }
        let mut keyed = request("https://api.example.com/");
        keyed
            .headers
            .insert("Accept".into(), "application/json".into());
        assert!(check_fetch(&keyed, &allowed).is_ok());
        let mut big = request("https://api.example.com/");
        big.body = "x".repeat(MAX_FETCH_BODY_BYTES + 1);
        assert!(check_fetch(&big, &allowed).is_err());
    }

    #[test]
    fn only_public_internet_addresses_are_reachable() {
        for public in [
            "93.184.216.34",
            "1.1.1.1",
            "192.1.0.255",
            "2606:4700:4700::1111",
            "2a00:1450:4009::200e",
        ] {
            assert!(public_address(public.parse().unwrap()), "{public}");
        }
        for local in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
            "2001:db8::1",
            "192.0.0.1",
            "192.88.99.1",
            "fec0::1",
            "::192.168.1.1",
            "64:ff9b::a00:1",
            "64:ff9b:1::a00:1",
            "2002:c0a8:101::1",
        ] {
            assert!(!public_address(local.parse().unwrap()), "{local}");
        }
    }
}
