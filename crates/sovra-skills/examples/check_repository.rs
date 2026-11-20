//! Read-only check that a plugin repository is added by Sovra's **Add from GitHub**
//! with nothing left out. Built for the plugin template's CI.

use std::io::Write as _;
use std::path::Path;

use sovra_core::skills::SkillError;

fn main() -> Result<(), SkillError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [folder, repository] = args.as_slice() else {
        return Err(SkillError(
            "Usage: check_repository REPOSITORY_FOLDER OWNER/REPOSITORY".into(),
        ));
    };
    let check = sovra_skills::check_repository(Path::new(folder), repository)?;
    if !check.left_out.is_empty() {
        return Err(SkillError(format!(
            "Sovra would leave out: {}.",
            check.left_out.join("; ")
        )));
    }
    let mut out = std::io::stdout().lock();
    let mut write = || -> std::io::Result<()> {
        for skill in &check.package.skills {
            writeln!(
                out,
                "skill {} ({}), {} file(s)",
                skill.name,
                skill.title,
                skill.resources.len()
            )?;
        }
        for service in &check.package.services {
            writeln!(
                out,
                "suggested connection {}: {}",
                service.name, service.address
            )?;
        }
        if let Some(code) = &check.package.code {
            for tool in &code.tools {
                writeln!(out, "tool {} ({})", tool.name, tool.title)?;
            }
            writeln!(out, "code: {}", code.access_summary())?;
        }
        writeln!(
            out,
            "ok: Sovra adds {} as \"{}\" with {} skill(s){} and {} suggested connection(s).",
            repository,
            check.package.title,
            check.package.skills.len(),
            check
                .package
                .code
                .as_ref()
                .map_or_else(String::new, |code| format!(
                    ", {} tool(s)",
                    code.tools.len()
                )),
            check.package.services.len()
        )
    };
    write().map_err(|_| SkillError("The report could not be written.".into()))
}
