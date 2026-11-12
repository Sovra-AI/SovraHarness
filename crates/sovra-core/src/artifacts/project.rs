//! Portable frontend projects: source and build metadata, independent of the host.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebsiteProject {
    pub files: BTreeMap<String, String>,
    #[serde(default = "default_output")]
    pub output_dir: String,
}

fn default_output() -> String {
    "dist".into()
}

impl WebsiteProject {
    /// Validate before any source is written to a managed build directory.
    pub fn parse(content: &str) -> Result<Self, String> {
        if content.len() > 8 * 1024 * 1024 {
            return Err("Website projects must be smaller than 8 MB.".into());
        }
        let project: Self = serde_json::from_str(content)
            .map_err(|_| "A website project needs a files object and an output_dir.".to_owned())?;
        if project.files.is_empty() || project.files.len() > 128 {
            return Err("A website project needs between 1 and 128 source files.".into());
        }
        if !safe_path(&project.output_dir) || project.output_dir.contains('/') {
            return Err(
                "The build output must be a folder name such as dist, build or out.".into(),
            );
        }
        for path in project.files.keys() {
            if !safe_path(path)
                || path
                    .split('/')
                    .any(|part| matches!(part, "node_modules" | ".git"))
            {
                return Err(format!("Use a relative source file name, not {path}."));
            }
        }
        let package: serde_json::Value =
            serde_json::from_str(project.files.get("package.json").ok_or(
                "Include package.json with a build script and the website's dependencies.",
            )?)
            .map_err(|_| "package.json is not valid JSON.".to_owned())?;
        if package
            .pointer("/scripts/build")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|script| script.trim().is_empty())
        {
            return Err(
                "Add a build script to package.json that produces the website's static files."
                    .into(),
            );
        }
        Ok(project)
    }
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 240
        && !path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|part| !matches!(part, "" | "." | "..") && !part.ends_with([' ', '.']))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn projects_keep_multiple_sources_and_reject_escaping_paths() {
        let valid = json!({"files": {"package.json":"{\"scripts\":{\"build\":\"vite build\"}}", "src/App.tsx":"export default () => <h1>Hello</h1>"}});
        assert_eq!(
            WebsiteProject::parse(&valid.to_string())
                .unwrap()
                .output_dir,
            "dist"
        );
        for path in [
            "../secret",
            "/tmp/file",
            "C:\\file",
            "src/../../file",
            "node_modules/x",
            "a/./x",
        ] {
            let mut invalid = valid.clone();
            invalid["files"][path] = json!("bad");
            assert!(
                WebsiteProject::parse(&invalid.to_string()).is_err(),
                "{path}"
            );
        }
        assert!(WebsiteProject::parse("{\"files\":{}}").is_err());
    }
}
