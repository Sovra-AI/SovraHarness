//! File workspace for a `/coding` run.
//!
//! The shortcut starts this run instead of a chat reply. The model writes
//! relative source files here; packaging turns them into library artifacts.
//! Nothing in this module touches the disk.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::artifacts::{ArtifactKind, WebsiteProject};

/// Skill name that starts the file-writing run.
pub const CODING_SKILL: &str = "coding";
/// Specialist id named by that skill. The run replaces a normal delegation.
pub const CODING_AGENT_ID: &str = "CodingAgent";

pub const MAX_CODE_FILES: usize = 64;
pub const MAX_CODE_FILE_BYTES: usize = 256 * 1024;
pub const MAX_CODE_WORKSPACE_BYTES: usize = 2 * 1024 * 1024;

/// Whether this skill invocation must write files instead of chatting.
pub fn is_coding_skill(skill_name: &str) -> bool {
    skill_name == CODING_SKILL
}

/// Whether the tools list names the coding specialist.
pub fn skill_requests_coding(tools: &[String]) -> bool {
    tools.iter().any(|tool| tool == CODING_AGENT_ID)
}

/// What a typed message is asking for, before any model call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingRequest {
    /// Build or change a page, program, or other source.
    Build,
    /// Ask what existing code means.
    Explain,
    /// Leave the message on the normal chat path.
    Chat,
}

/// Whether this turn should write files.
///
/// The coding shortcut does. A private chat does too when the words ask for
/// something to be built or changed and that helper is still switched on.
/// An explanation, another shortcut, or a message from outside the person's
/// own chat stays on the normal path.
pub fn starts_coding_run(
    skill_name: Option<&str>,
    skill_tools: &[String],
    coding_enabled: bool,
    private_chat: bool,
    prompt: &str,
) -> bool {
    if skill_name.is_some_and(is_coding_skill) || skill_requests_coding(skill_tools) {
        return true;
    }
    private_chat
        && skill_name.is_none()
        && coding_enabled
        && classify_coding_request(prompt) == CodingRequest::Build
}

/// Build or change versus explain, from the words alone.
pub fn classify_coding_request(text: &str) -> CodingRequest {
    let text = normalize_request(text);
    if text.is_empty() {
        return CodingRequest::Chat;
    }
    if is_howto(&text) && !has_build_request(&text) {
        return if has_deliverable(&text) || asks_to_explain(&text) {
            CodingRequest::Explain
        } else {
            CodingRequest::Chat
        };
    }
    let explain = asks_to_explain(&text);
    let build = (has_action(&text) || has_build_request(&text)) && has_deliverable(&text);
    if build && !(explain && question_led(&text)) {
        CodingRequest::Build
    } else if explain {
        CodingRequest::Explain
    } else {
        CodingRequest::Chat
    }
}

fn normalize_request(text: &str) -> String {
    let lower = text.to_lowercase().replace(['-', '—', '_'], " ");
    let mut out = String::with_capacity(lower.len());
    let mut pending_space = false;
    for ch in lower.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    out
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
}

fn has_word(text: &str, word: &str) -> bool {
    words(text).any(|part| part == word)
}

fn asks_to_explain(text: &str) -> bool {
    const PHRASES: &[&str] = &[
        "what does this",
        "what does that",
        "what does the code",
        "what does the following",
        "what does the function",
        "what does the snippet",
        "what does it do",
        "what does it mean",
        "what this code",
        "what this function",
        "what this snippet",
        "what this means",
        "explain this",
        "explain the code",
        "explain the snippet",
        "explain the function",
        "explain a snippet",
        "explain the following",
        "explain how",
        "how does this",
        "how does the code",
        "how does the function",
        "how does it work",
        "what is this code",
        "what is this function",
        "what is this snippet",
        "walk me through",
        "help me understand",
        "tell me what this",
        "can you explain",
        "could you explain",
        "please explain",
        "meaning of this",
        "meaning of the code",
        "meaning of the function",
    ];
    PHRASES.iter().any(|phrase| text.contains(phrase))
}

fn is_howto(text: &str) -> bool {
    const PHRASES: &[&str] = &[
        "how do i",
        "how do you",
        "how can i",
        "how can you",
        "how to",
        "how would i",
        "how should i",
        "how does one",
    ];
    PHRASES.iter().any(|phrase| text.contains(phrase))
}

fn has_build_request(text: &str) -> bool {
    const CUES: &[&str] = &[
        "can you build",
        "can you make",
        "can you create",
        "can you write",
        "can you fix",
        "can you change",
        "can you add",
        "can you update",
        "could you build",
        "could you make",
        "could you create",
        "could you write",
        "could you fix",
        "please build",
        "please make",
        "please create",
        "please write",
        "please fix",
        "please change",
        "please add",
        "i want",
        "i need",
        "i would like",
        "i'd like",
    ];
    CUES.iter().any(|cue| text.contains(cue))
}

fn question_led(text: &str) -> bool {
    const STARTS: &[&str] = &[
        "what ",
        "explain ",
        "how ",
        "why ",
        "tell me ",
        "walk me ",
        "help me understand",
        "can you explain",
        "could you explain",
        "please explain",
    ];
    STARTS.iter().any(|prefix| text.starts_with(prefix))
}

fn has_action(text: &str) -> bool {
    const ACTIONS: &[&str] = &[
        "build",
        "make",
        "create",
        "write",
        "fix",
        "change",
        "update",
        "add",
        "redesign",
        "generate",
        "implement",
        "develop",
        "code",
    ];
    ACTIONS.iter().any(|action| has_word(text, action))
}

fn has_deliverable(text: &str) -> bool {
    const PHRASES: &[&str] = &[
        "web page",
        "web site",
        "landing page",
        "home page",
        "html page",
        "contact form",
        "source file",
        "source code",
    ];
    const WORDS: &[&str] = &[
        "website",
        "webpage",
        "homepage",
        "program",
        "programme",
        "script",
        "app",
        "html",
        "css",
        "javascript",
        "typescript",
        "react",
        "vue",
        "svelte",
        "component",
        "function",
        "stylesheet",
        "site",
    ];
    if PHRASES.iter().any(|phrase| text.contains(phrase))
        || WORDS.iter().any(|word| has_word(text, word))
    {
        return true;
    }
    (has_word(text, "page") || has_word(text, "pages"))
        && !words(text).any(|word| {
            matches!(
                word,
                "letter"
                    | "book"
                    | "document"
                    | "recipe"
                    | "pdf"
                    | "chapter"
                    | "novel"
                    | "magazine"
                    | "newspaper"
            )
        })
}

/// One saved piece of a coding run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedCode {
    pub title: String,
    pub kind: ArtifactKind,
    pub language: String,
    pub content: String,
}

/// Relative files written during one `/coding` run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeWorkspace {
    files: BTreeMap<String, String>,
}

impl CodeWorkspace {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn list(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    pub fn read(&self, path: &str) -> Result<String, String> {
        accept_code_path(path)?;
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| format!("There is no file named {path}."))
    }

    pub fn write(&mut self, path: &str, content: &str) -> Result<(), String> {
        accept_code_path(path)?;
        if content.len() > MAX_CODE_FILE_BYTES {
            return Err("That file is too large. Keep each file under 256 KB.".into());
        }
        let replacing = self.files.contains_key(path);
        if !replacing && self.files.len() >= MAX_CODE_FILES {
            return Err("This coding run can keep at most 64 files.".into());
        }
        let without = self.total_bytes() - self.files.get(path).map_or(0, String::len);
        if without.saturating_add(content.len()) > MAX_CODE_WORKSPACE_BYTES {
            return Err("This coding run can keep at most 2 MB of source.".into());
        }
        self.files.insert(path.to_owned(), content.to_owned());
        Ok(())
    }

    /// Replace one exact occurrence of `old` with `new`.
    pub fn edit(&mut self, path: &str, old: &str, new: &str) -> Result<(), String> {
        if old.is_empty() {
            return Err("Say which text to replace.".into());
        }
        let current = self.read(path)?;
        let count = current.matches(old).count();
        if count == 0 {
            return Err("That text was not found. Read the file and copy the exact text.".into());
        }
        if count > 1 {
            return Err("That text appears more than once. Include more surrounding lines.".into());
        }
        let updated = current.replacen(old, new, 1);
        self.write(path, &updated)
    }

    fn total_bytes(&self) -> usize {
        self.files.values().map(String::len).sum()
    }
}

/// Turn the workspace into library items. A framework site with a build script
/// becomes one website project. A page becomes a web page, and any extra files
/// are saved beside it. Other source is saved file by file.
pub fn package_code(workspace: &CodeWorkspace, request: &str) -> Result<Vec<SavedCode>, String> {
    if workspace.files.is_empty() {
        return Err("No files were written.".into());
    }
    let fallback = coding_title(request);
    if workspace.files.contains_key("package.json") {
        let content = serde_json::to_string(&WebsiteProject {
            files: workspace.files.clone(),
            output_dir: "dist".into(),
        })
        .map_err(|_| "Could not pack the website project.".to_owned())?;
        if WebsiteProject::parse(&content).is_ok() {
            return Ok(vec![SavedCode {
                title: fallback,
                kind: ArtifactKind::Project,
                language: "website-project".into(),
                content,
            }]);
        }
    }
    let html_key = workspace
        .files
        .keys()
        .find(|path| path.as_str() == "index.html")
        .or_else(|| workspace.files.keys().find(|path| is_html_path(path)))
        .cloned();
    if let Some(html_key) = html_key {
        let html = &workspace.files[&html_key];
        let mut saved = vec![SavedCode {
            title: html_title(html).unwrap_or_else(|| fallback.clone()),
            kind: ArtifactKind::Webpage,
            language: "html".into(),
            content: html.clone(),
        }];
        for (path, content) in &workspace.files {
            if path == &html_key {
                continue;
            }
            saved.push(file_piece(path, content, path));
        }
        return Ok(saved);
    }
    let alone = workspace.files.len() == 1;
    Ok(workspace
        .files
        .iter()
        .map(|(path, content)| {
            let title = if alone {
                fallback.as_str()
            } else {
                path.as_str()
            };
            file_piece(path, content, title)
        })
        .collect())
}

/// Short library title taken from the person's request.
pub fn coding_title(request: &str) -> String {
    let words: Vec<&str> = request.split_whitespace().take(8).collect();
    if words.is_empty() {
        "Saved code".into()
    } else {
        let mut title = words.join(" ");
        if request.split_whitespace().count() > 8 {
            title.push('…');
        }
        title
    }
}

fn is_html_path(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"))
}

pub fn language_for_path(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "html",
        "css" => "css",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "rs" => "rust",
        "py" => "python",
        "json" => "json",
        "md" => "markdown",
        "svg" => "svg",
        "vue" => "vue",
        "svelte" => "svelte",
        _ => "text",
    }
}

fn file_piece(path: &str, content: &str, title: &str) -> SavedCode {
    let language = language_for_path(path).to_owned();
    SavedCode {
        title: title.to_owned(),
        kind: ArtifactKind::for_language(&language),
        language,
        content: content.to_owned(),
    }
}

fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title>")? + "<title>".len();
    let end = lower[start..].find("</title>")? + start;
    let title = html[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title: String = title.chars().take(80).collect();
    (!title.is_empty()).then_some(title)
}

pub fn accept_code_path(path: &str) -> Result<(), String> {
    let refused = path.is_empty()
        || path.len() > 240
        || path.starts_with('/')
        || path.contains(['\\', ':', '\0'])
        || path.split('/').any(|part| {
            matches!(part, "" | "." | ".." | "node_modules" | ".git") || part.ends_with([' ', '.'])
        });
    if refused {
        Err(format!(
            "Use a relative file name such as index.html or src/App.tsx, not {path}."
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_skill_is_the_file_writing_shortcut() {
        assert!(is_coding_skill("coding"));
        assert!(!is_coding_skill("deep-research"));
        assert!(skill_requests_coding(&["CodingAgent".into()]));
        assert!(!skill_requests_coding(&["WebSearchAgent".into()]));
    }

    #[test]
    fn plain_build_requests_start_the_file_writing_run_and_explanations_do_not() {
        let build = [
            "Build a website for our garden club with meeting times and upcoming events",
            "Build a garden-club page",
            "Please make a one-page site for the garden club",
            "Can you create a simple webpage that lists Tuesday meetings?",
            "Write a small program that prints the club members",
            "Fix the garden club page so the heading says Welcome",
            "Change our website to add the next meeting",
            "I want a React site for the school fair",
            "Add a contact form to the club page",
            "Make an HTML page for the garden club",
            "Generate a landing page for the bake sale",
            "Update the script so it shows the meeting time",
            "Can you build a page and explain the code?",
        ];
        for phrase in build {
            assert_eq!(
                classify_coding_request(phrase),
                CodingRequest::Build,
                "{phrase}"
            );
            assert!(starts_coding_run(None, &[], true, true, phrase), "{phrase}");
        }
        let explain = [
            "What does this code mean?",
            "Explain this snippet",
            "What does this function do?",
            "Can you explain this Python loop?",
            "How does this function work?",
            "Tell me what this code does",
            "Walk me through this snippet",
            "What does the following code mean?\nfunction add(a, b) { return a + b }",
            "Explain how to build a website",
            "How do I build a website?",
            "Please explain this code",
        ];
        for phrase in explain {
            assert_eq!(
                classify_coding_request(phrase),
                CodingRequest::Explain,
                "{phrase}"
            );
            assert!(
                !starts_coding_run(None, &[], true, true, phrase),
                "{phrase}"
            );
        }
        let chat = [
            "Hello",
            "What's the weather tomorrow?",
            "Remind me to water the garden",
            "Write a polite letter to our landlord about the leaky tap",
            "The door code is 4821",
            "Search the web for garden club ideas",
            "How much free space is left on my computer?",
            "Create a simple monthly household budget table",
            "Add milk to the shopping list",
            "What page is the recipe on?",
            "Fix the leaky tap in the kitchen",
            "How do I bake a cake?",
            "",
        ];
        for phrase in chat {
            assert_eq!(
                classify_coding_request(phrase),
                CodingRequest::Chat,
                "{phrase}"
            );
            assert!(
                !starts_coding_run(None, &[], true, true, phrase),
                "{phrase}"
            );
        }
        assert!(starts_coding_run(
            Some("coding"),
            &["CodingAgent".into()],
            true,
            true,
            "What does this code mean?"
        ));
        assert!(!starts_coding_run(
            None,
            &[],
            true,
            false,
            "Build a website for our garden club"
        ));
        assert!(!starts_coding_run(
            None,
            &[],
            false,
            true,
            "Build a website for our garden club"
        ));
        assert!(!starts_coding_run(
            Some("web-search"),
            &["WebSearchAgent".into()],
            true,
            true,
            "Build a website for our garden club"
        ));
    }

    #[test]
    fn writes_edits_and_refuses_escaping_paths() {
        let mut workspace = CodeWorkspace::default();
        workspace
            .write("index.html", "<title>Club</title><h1>Hi</h1>")
            .unwrap();
        workspace
            .edit("index.html", "<h1>Hi</h1>", "<h1>Garden</h1>")
            .unwrap();
        assert_eq!(
            workspace.read("index.html").unwrap(),
            "<title>Club</title><h1>Garden</h1>"
        );
        assert!(workspace.edit("index.html", "Garden", "x").is_ok());
        workspace.write("notes.txt", "Garden Garden").unwrap();
        assert!(workspace.edit("notes.txt", "Garden", "Plot").is_err());
        for path in ["../secret", "/tmp/x", "src/../../x", "node_modules/x", ""] {
            assert!(workspace.write(path, "no").is_err(), "{path}");
        }
    }

    #[test]
    fn packages_a_page_a_project_and_a_source_file() {
        let html = "<title>Garden club</title><style>body{color:green}</style><h1>Tuesday</h1><script>console.log('meet')</script>";
        let mut alone = CodeWorkspace::default();
        alone.write("index.html", html).unwrap();
        let saved = package_code(&alone, "Build a garden club page").unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].kind, ArtifactKind::Webpage);
        assert_eq!(saved[0].title, "Garden club");
        assert_eq!(saved[0].content, html);
        assert!(saved[0].content.contains("<style"));
        assert!(saved[0].content.contains("<script"));

        let mut page = CodeWorkspace::default();
        page.write("index.html", "<title>Garden club</title><p>Tuesday</p>")
            .unwrap();
        page.write("styles.css", "body{color:green}").unwrap();
        let saved = package_code(&page, "Build a garden club website").unwrap();
        assert_eq!(saved[0].kind, ArtifactKind::Webpage);
        assert_eq!(saved[0].title, "Garden club");
        assert_eq!(saved[1].language, "css");

        let mut project = CodeWorkspace::default();
        project
            .write("package.json", r#"{"scripts":{"build":"vite build"}}"#)
            .unwrap();
        project
            .write("src/App.tsx", "export default () => <h1>Hello</h1>")
            .unwrap();
        let saved = package_code(&project, "React garden site").unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].kind, ArtifactKind::Project);
        WebsiteProject::parse(&saved[0].content).unwrap();

        let mut source = CodeWorkspace::default();
        source.write("main.rs", "fn main() {}").unwrap();
        let saved = package_code(&source, "A tiny rust program").unwrap();
        assert_eq!(saved[0].kind, ArtifactKind::Code);
        assert_eq!(saved[0].language, "rust");
        assert_eq!(saved[0].title, "A tiny rust program");
        assert_eq!(
            package_code(&CodeWorkspace::default(), "empty"),
            Err("No files were written.".to_owned())
        );
    }
}
