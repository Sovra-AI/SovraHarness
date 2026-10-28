//! Native skill catalogue, guided explainer, category explorer, 1-click starter prompts,
//! plain-language custom skill creator, and package import. All persistence goes through
//! the runtime protocol; this view never opens or executes a package itself.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    TestSupportExt as _, Window, div,
};
use sovra_core::{
    Command, NavTab,
    skills::{Skill, SkillCommand, SkillKind, valid_name},
};

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
    shell::Shell,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SkillCategoryFilter {
    #[default]
    All,
    Workspace,
    Files,
    Research,
    Specialists,
    Presentation,
    Custom,
}

impl SkillCategoryFilter {
    const ALL: [Self; 7] = [
        Self::All,
        Self::Workspace,
        Self::Files,
        Self::Research,
        Self::Specialists,
        Self::Presentation,
        Self::Custom,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::All => "skill-filter-all",
            Self::Workspace => "skill-filter-workspace",
            Self::Files => "skill-filter-files",
            Self::Research => "skill-filter-research",
            Self::Specialists => "skill-filter-specialists",
            Self::Presentation => "skill-filter-presentation",
            Self::Custom => "skill-filter-custom",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All skills",
            Self::Workspace => "Quick actions",
            Self::Files => "Files & folders",
            Self::Research => "Web search",
            Self::Specialists => "AI helpers",
            Self::Presentation => "Tables & cards",
            Self::Custom => "Made by you",
        }
    }

    fn matches(self, skill: &Skill) -> bool {
        match self {
            Self::All => true,
            Self::Workspace => skill.category == "Workspace",
            Self::Files => skill.category == "Files",
            Self::Research => skill.category == "Research",
            Self::Specialists => skill.category == "Specialists",
            Self::Presentation => skill.category == "Presentation",
            Self::Custom => !skill.builtin || skill.category == "Your skills",
        }
    }
}

pub(crate) struct SkillLibrary {
    search: Entity<InputState>,
    category: SkillCategoryFilter,
    name: Entity<InputState>,
    title: Entity<InputState>,
    description: Entity<InputState>,
    tools: Entity<InputState>,
    body: Entity<TextareaState>,
    editing: Option<Skill>,
    creating: bool,
    advanced_open: bool,
    details: Option<String>,
    deleting: Option<String>,
    pub(crate) notice: String,
    picker: Option<Task<()>>,
    _search: Subscription,
}

impl SkillLibrary {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("What do you want help with? (e.g. letter, budget, photos, web)…")
        });
        let subscription = cx.subscribe(&search, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            search,
            category: SkillCategoryFilter::All,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("e.g. weekly-check-in")),
            title: cx.new(|cx| InputState::new(window, cx).placeholder("e.g. Weekly check-in")),
            description: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("e.g. Summarize my week and suggest 3 priorities")
            }),
            tools: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Optional tool names, separated by spaces")
            }),
            body: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("Write step-by-step instructions in your own words…")
                    .auto_grow(3, 8)
            }),
            editing: None,
            creating: false,
            advanced_open: false,
            details: None,
            deleting: None,
            notice: String::new(),
            picker: None,
            _search: subscription,
        }
    }

    fn edit(&mut self, skill: Option<Skill>, window: &mut Window, cx: &mut Context<Shell>) {
        self.creating = skill.is_none();
        self.advanced_open = skill.as_ref().is_some_and(|s| !s.tools.is_empty());
        for (input, value) in [
            (&self.name, skill.as_ref().map_or("", |skill| &skill.name)),
            (&self.title, skill.as_ref().map_or("", |skill| &skill.title)),
            (
                &self.description,
                skill.as_ref().map_or("", |skill| &skill.description),
            ),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        let tools = skill
            .as_ref()
            .map_or_else(String::new, |skill| skill.tools.join(" "));
        self.tools
            .update(cx, |input, cx| input.set_value(tools, window, cx));
        self.body.update(cx, |input, cx| {
            input.set_value(
                skill.as_ref().map_or("", |skill| &skill.instructions),
                window,
                cx,
            );
        });
        self.editing = skill;
        self.notice.clear();
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn set_editor_fields_for_test(
        &mut self,
        name: &str,
        title: &str,
        description: &str,
        body: &str,
        window: &mut Window,
        cx: &mut Context<Shell>,
    ) {
        self.name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.title
            .update(cx, |input, cx| input.set_value(title, window, cx));
        self.description
            .update(cx, |input, cx| input.set_value(description, window, cx));
        self.body
            .update(cx, |input, cx| input.set_value(body, window, cx));
    }
}

fn slugify_command_name(title: &str) -> String {
    let mut out = String::new();
    let mut prev_hyphen = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if !out.is_empty() && !prev_hyphen {
            out.push('-');
            prev_hyphen = true;
        }
        if out.len() >= 64 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

impl Shell {
    fn save_skill(&mut self, cx: &mut Context<Self>) {
        let view = &self.skill_library;
        let mut skill = view.editing.clone().unwrap_or_else(|| Skill {
            name: String::new(),
            title: String::new(),
            description: String::new(),
            category: "Your skills".into(),
            aliases: vec![],
            kind: SkillKind::Procedure,
            instructions: String::new(),
            tools: vec![],
            argument_hint: "<request>".into(),
            requires_input: false,
            enabled: true,
            builtin: false,
            license: None,
            compatibility: None,
            metadata: std::collections::BTreeMap::new(),
        });
        let raw_title = view.title.read(cx).value().trim().to_owned();
        let raw_name = view.name.read(cx).value().trim().to_owned();
        skill.title = raw_title;
        skill.name = if raw_name.is_empty() {
            slugify_command_name(&skill.title)
        } else if valid_name(&raw_name) {
            raw_name
        } else {
            slugify_command_name(&raw_name)
        };
        skill.description = view.description.read(cx).value().trim().into();
        skill.tools = view
            .tools
            .read(cx)
            .value()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        skill.instructions = view.body.read(cx).value().to_string();
        match skill.validate() {
            Ok(()) => self.skill_command(SkillCommand::Save(Box::new(skill)), cx),
            Err(error) => {
                self.skill_library.notice = error.to_string();
                cx.notify();
            }
        }
    }

    fn skill_command(&mut self, command: SkillCommand, cx: &mut Context<Self>) {
        self.skill_library.notice = match cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Skills(command))
        {
            Ok(()) => "Updating skills…".into(),
            Err(_) => "The runtime is unavailable. Your draft is still here.".into(),
        };
        cx.notify();
    }

    fn import_skill(&mut self, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Import skill folder".into()),
        });
        self.skill_library.picker = Some(cx.spawn(async move |this, cx| {
            let result = selected.await;
            let _ = this.update(cx, |shell, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(directory) = paths.into_iter().next() {
                        shell.skill_command(SkillCommand::Import { directory }, cx);
                    }
                }
                Ok(Ok(None)) => {}
                _ => {
                    shell.skill_library.notice =
                        "The skill picker could not open. Please try again.".into();
                    cx.notify();
                }
            });
        }));
    }
}

pub(crate) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let view = &shell.skill_library;
    let query = view.search.read(cx).value().to_string();
    let editing = view.creating || view.editing.is_some();

    v_flex()
        .id("sovra-skills-view")
        .test_support()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .overflow_y_scroll()
        .items_center()
        .p(t.space.large)
        .child(
            v_flex()
                .w_full()
                .max_w(t.layout.page_width)
                .min_w_0()
                .flex_shrink_0()
                .gap(t.space.large)
                .child(render_header(cx))
                .children((!view.notice.is_empty()).then(|| {
                    div()
                        .id("skill-library-notice")
                        .test_support()
                        .w_full()
                        .p(t.space.small)
                        .sovra_surface(Surface::Subtle, cx)
                        .sovra_text(TextRole::Caption, cx)
                        .child(view.notice.clone())
                }))
                .children(editing.then(|| editor(view, cx)))
                .children((!editing).then(|| how_it_works(cx)))
                .children((!editing).then(|| render_search_and_categories(shell, view, &query, cx)))
                .children((!editing).then(|| render_skills_catalog(shell, view, &query, cx))),
        )
}

fn render_header(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .w_full()
        .gap(t.space.small)
        .child(design::page_header(
            "What Sovra Can Do for You",
            "Every helper, tool, and shortcut built into your personal AI. Pick any card below and press “Try in chat” or “Try example” — no technical knowledge needed.",
            cx,
        ))
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::primary_button("skill-create", "New skill", cx)
                        .icon(IconName::Plus)
                        .tooltip("Teach Sovra a reusable step-by-step recipe in your own words")
                        .on_click(cx.listener(|shell, _, window, cx| {
                            shell.skill_library.edit(None, window, cx);
                        })),
                )
                .child(
                    design::button("skill-import", "Import folder", cx)
                        .tooltip("Add a saved skill folder from your computer")
                        .on_click(cx.listener(|shell, _, _, cx| shell.import_skill(cx))),
                )
                .child(
                    design::button("skill-refresh", "Refresh", cx)
                        .ghost()
                        .tooltip("Reload skills from disk")
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.skill_command(SkillCommand::Refresh, cx);
                        })),
                ),
        )
}

fn how_it_works(cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let step = |n: &'static str, text: &'static str| {
        h_flex()
            .flex_1()
            .min_w(t.layout.card_min_width)
            .gap(t.space.small)
            .items_center()
            .p(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .flex_shrink_0()
                    .size(t.layout.icon_medium)
                    .rounded_full()
                    .bg(cx.theme().primary)
                    .text_color(cx.theme().primary_foreground)
                    .flex()
                    .items_center()
                    .justify_center()
                    .sovra_text(TextRole::Caption, cx)
                    .child(n),
            )
            .child(div().min_w_0().sovra_text(TextRole::Body, cx).child(text))
    };
    h_flex()
        .id("skills-how-it-works")
        .test_support()
        .w_full()
        .flex_wrap()
        .gap(t.space.small)
        .child(step(
            "1",
            "Pick any card below — from web search and reading files to asking a specialist.",
        ))
        .child(step(
            "2",
            "Press “Try example” to test a ready-made question, or “Try in chat” to write your own.",
        ))
        .child(step(
            "3",
            "Press “New skill” anytime to teach Sovra your own repeatable recipe in plain English.",
        ))
}

fn render_search_and_categories(
    shell: &Shell,
    view: &SkillLibrary,
    query: &str,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let all_matches = shell.skills.search_all(query);

    v_flex()
        .w_full()
        .gap(t.space.small)
        .child(
            div()
                .w_full()
                .max_w(t.layout.reading_width)
                .child(Input::new(&view.search).aria_label("Search skills")),
        )
        .child(
            h_flex()
                .id("skill-category-filters")
                .test_support()
                .w_full()
                .flex_wrap()
                .gap(t.space.compact)
                .children(SkillCategoryFilter::ALL.into_iter().filter_map(|cat| {
                    let count = all_matches.iter().filter(|s| cat.matches(s)).count();
                    if cat == SkillCategoryFilter::Custom && count == 0 && view.category != cat {
                        return None;
                    }
                    Some(
                        design::button(cat.id(), format!("{} ({count})", cat.label()), cx)
                            .small()
                            .ghost()
                            .selected(view.category == cat)
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                shell.skill_library.category = cat;
                                cx.notify();
                            })),
                    )
                })),
        )
}

fn category_explanation(category: &str) -> (&'static str, &'static str, IconName) {
    match category {
        "Workspace" => (
            "Quick actions & navigation",
            "One-click shortcuts that open files, folders, WhatsApp QR linking, or pages inside Sovra right away.",
            IconName::Sparkles,
        ),
        "Research" => (
            "Search the live web",
            "Look up current facts, news, and cited links from the web with no setup needed.",
            IconName::Globe,
        ),
        "Files" => (
            "Read your files & folders",
            "Attach documents, search connected folders, inspect file history, and save work to your library.",
            IconName::FileText,
        ),
        "Presentation" => (
            "Visual cards, tables & checklists",
            "Ask Sovra to present answers as clean tables, interactive cards, choices, or progress trackers.",
            IconName::Table,
        ),
        "Specialists" => (
            "Specialist AI helpers",
            "Send your question straight to a dedicated specialist — letters, budgets, coding, stocks, housing, or research.",
            IconName::Users,
        ),
        _ => (
            "Your custom skills",
            "Personal recipes you created or imported so Sovra follows your exact steps.",
            IconName::BookOpen,
        ),
    }
}

fn skill_icon(skill: &Skill) -> IconName {
    match skill.name.as_str() {
        "files" | "read-files" | "document-reader" | "document-author" | "documentation" => {
            IconName::FileText
        }
        "index-folder" | "search-files" | "file-catalog" | "file-history" | "map-files"
        | "file-knowledge" => IconName::Database,
        "web-search" | "connections" | "google-workspace" | "microsoft-workspace" => {
            IconName::Globe
        }
        "new" | "team" => IconName::MessageCircle,
        "schedules" | "scheduler" | "todos" => IconName::Calendar,
        "table" | "spreadsheet" => IconName::Table,
        "coding" | "execution" | "github" => IconName::Code,
        "stock-market" | "progress" => IconName::TrendingUp,
        "housing-market" | "housing-advisor" => IconName::House,
        "image-search" | "image-generation" => IconName::Image,
        "artifacts" | "save-artifact" => IconName::LibraryBig,
        "settings" => IconName::Settings,
        "models" => IconName::Box,
        _ => match skill.category.as_str() {
            "Workspace" => IconName::Sparkles,
            "Research" => IconName::Globe,
            "Files" => IconName::FileText,
            "Presentation" => IconName::Table,
            "Specialists" => IconName::Users,
            _ => IconName::BookOpen,
        },
    }
}

fn render_skills_catalog(
    shell: &Shell,
    view: &SkillLibrary,
    query: &str,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let matches: Vec<(usize, &Skill)> = shell
        .skills
        .search_all(query)
        .into_iter()
        .enumerate()
        .filter(|(_, skill)| view.category.matches(skill))
        .collect();

    if matches.is_empty() {
        return v_flex()
            .id("skills-empty-state")
            .test_support()
            .w_full()
            .p(t.space.large)
            .gap(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("No matching skills"),
            )
            .child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .text_color(t.text_secondary)
                    .child("Try a different search word, or choose “All skills” above."),
            )
            .into_any_element();
    }

    let ordered_categories = [
        "Your skills",
        "Workspace",
        "Research",
        "Files",
        "Presentation",
        "Specialists",
    ];

    let mut sections = v_flex().w_full().gap(t.space.large);
    for cat_key in ordered_categories {
        let group: Vec<(usize, &Skill)> = matches
            .iter()
            .copied()
            .filter(|(_, skill)| {
                if cat_key == "Your skills" {
                    !skill.builtin || skill.category == "Your skills"
                } else {
                    skill.builtin && skill.category == cat_key
                }
            })
            .collect();
        if group.is_empty() {
            continue;
        }
        let (heading, subtitle, icon) = category_explanation(cat_key);
        sections = sections.child(
            v_flex()
                .w_full()
                .gap(t.space.medium)
                .child(
                    v_flex()
                        .gap(t.space.hair)
                        .child(
                            h_flex()
                                .items_center()
                                .gap(t.space.small)
                                .child(Icon::new(icon).small().text_color(cx.theme().link))
                                .child(div().sovra_text(TextRole::SectionTitle, cx).child(heading)),
                        )
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(subtitle),
                        ),
                )
                .child(
                    h_flex().w_full().flex_wrap().gap(t.space.medium).children(
                        group
                            .into_iter()
                            .map(|(index, skill)| card(skill, index, view, cx)),
                    ),
                ),
        );
    }
    sections.into_any_element()
}

fn editor(view: &SkillLibrary, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let field =
        |label: &'static str, hint: &'static str, control: AnyElement, cx: &Context<Shell>| {
            v_flex()
                .w_full()
                .min_w_0()
                .gap(t.space.hair)
                .child(div().sovra_text(TextRole::SectionTitle, cx).child(label))
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(hint),
                )
                .child(control)
        };

    let mut form = v_flex()
        .id("skill-editor")
        .test_support()
        .w_full()
        .max_w(t.layout.reading_width)
        .min_w_0()
        .p(t.space.medium)
        .gap(t.space.small)
        .sovra_surface(Surface::Elevated, cx)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(if view.creating {
                    "Create a skill"
                } else {
                    "Edit skill"
                }),
        )
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap(t.space.medium)
                .child(
                    div().flex_1().min_w(t.layout.filter_width).child(field(
                        "1. Skill title",
                        "Shown on the card (for example: “Weekly family meal planner”).",
                        Input::new(&view.title)
                            .aria_label("Skill title")
                            .into_any_element(),
                        cx,
                    )),
                )
                .child(
                    div().flex_1().min_w(t.layout.filter_width).child(field(
                        "2. Shortcut word (optional)",
                        "Leave blank and Sovra will make one from your title.",
                        Input::new(&view.name)
                            .aria_label("Skill command name")
                            .readonly(!view.creating)
                            .into_any_element(),
                        cx,
                    )),
                ),
        )
        .child(field(
            "3. When should you use it?",
            "One short sentence explaining what this skill helps with.",
            Input::new(&view.description)
                .aria_label("When to use this skill")
                .into_any_element(),
            cx,
        ))
        .child(field(
            "4. Step-by-step instructions for Sovra",
            "Write naturally, just like explaining a task to a helpful friend.",
            Textarea::new(&view.body)
                .aria_label("Skill instructions")
                .into_any_element(),
            cx,
        ))
        .child(
            h_flex().child(
                design::button(
                    "skill-editor-advanced-toggle",
                    if view.advanced_open {
                        "Hide optional tool settings"
                    } else {
                        "Show optional tool settings"
                    },
                    cx,
                )
                .small()
                .ghost()
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.skill_library.advanced_open = !shell.skill_library.advanced_open;
                    cx.notify();
                })),
            ),
        );

    if view.advanced_open {
        form = form.child(field(
            "Optional: specific tools this skill may use",
            "Most people can leave this blank. Keep passwords or API keys out of skills.",
            Input::new(&view.tools)
                .aria_label("Required tools")
                .into_any_element(),
            cx,
        ));
    }

    form.child(
        div()
            .sovra_text(TextRole::Caption, cx)
            .text_color(t.text_secondary)
            .child("Keep credentials out of skills. Required tools must already be enabled; importing a skill does not run its scripts."),
    )
    .child(
        h_flex()
            .flex_wrap()
            .gap(t.space.small)
            .child(
                design::primary_button("skill-save", "Save skill", cx)
                    .on_click(cx.listener(|shell, _, _, cx| shell.save_skill(cx))),
            )
            .child(
                design::button("skill-editor-back", "Back to skills", cx)
                    .ghost()
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.skill_library.creating = false;
                        shell.skill_library.editing = None;
                        cx.notify();
                    })),
            ),
    )
}

fn card(
    skill: &Skill,
    index: usize,
    view: &SkillLibrary,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let name = skill.name.clone();
    let run_name = name.clone();
    let details_name = name.clone();
    let toggle_name = name.clone();
    let delete_name = name.clone();
    let confirm_name = name.clone();
    let editing = skill.clone();
    let enabled = skill.enabled;
    let is_action = matches!(skill.kind, SkillKind::Action(_));
    let copy = skill.friendly_card_copy();
    let sample_draft = copy.sample_draft.clone();
    let showing_details = view.details.as_ref() == Some(&name);

    let badge_label = if !skill.builtin {
        if enabled {
            "Made by you"
        } else {
            "Made by you · Off"
        }
    } else if is_action {
        "Instant action"
    } else {
        "Guided helper"
    };

    let primary_cta_label = if is_action { "Open now" } else { "Try in chat" };

    let mut card = v_flex()
        .id(("skill-card", index))
        .test_support()
        .flex_1()
        .min_w(t.layout.card_min_width)
        .max_w(t.layout.card_max_width)
        .gap(t.space.small)
        .p(t.space.medium)
        .sovra_surface(Surface::Glass, cx)
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(t.space.compact)
                .child(
                    h_flex()
                        .min_w_0()
                        .flex_1()
                        .items_center()
                        .gap(t.space.small)
                        .child(
                            Icon::new(skill_icon(skill))
                                .small()
                                .text_color(cx.theme().link),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(copy.friendly_title),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap(t.space.hair)
                        .child(
                            div()
                                .px(t.space.compact)
                                .py(t.space.hair)
                                .rounded(t.radius.pill)
                                .bg(t.nav_active_background)
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(cx.theme().link)
                                .child(format!("/{}", skill.name)),
                        )
                        .child(
                            div()
                                .px(t.space.compact)
                                .py(t.space.hair)
                                .rounded(t.radius.pill)
                                .bg(t.subtle)
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(badge_label),
                        ),
                ),
        )
        .child(div().sovra_text(TextRole::Body, cx).child(copy.summary))
        .child(
            div()
                .w_full()
                .p(t.space.small)
                .rounded(t.radius.input)
                .bg(t.subtle)
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(copy.example_display),
        );

    let mut actions = h_flex()
        .flex_wrap()
        .items_center()
        .gap(t.space.small)
        .child(
            design::primary_button(("skill-use", index), primary_cta_label, cx)
                .small()
                .disabled(!enabled)
                .accessibility_label(format!("Use {} in chat", skill.title))
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.select_tab(NavTab::Chat, cx);
                    shell.fill_composer_draft(&format!("/{run_name} "), window, cx);
                })),
        )
        .children((!is_action).then(|| {
            design::button(("skill-try-example", index), "Try example", cx)
                .small()
                .disabled(!enabled)
                .tooltip("Fill the chat box with this ready-made example question")
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.select_tab(NavTab::Chat, cx);
                    shell.fill_composer_draft(&sample_draft, window, cx);
                }))
        }))
        .child(
            design::button(
                ("skill-details", index),
                if showing_details {
                    "Hide steps"
                } else {
                    "How it works"
                },
                cx,
            )
            .small()
            .ghost()
            .on_click(cx.listener(move |shell, _, _, cx| {
                shell.skill_library.details = (shell.skill_library.details.as_ref()
                    != Some(&details_name))
                .then(|| details_name.clone());
                cx.notify();
            })),
        );

    if !skill.builtin {
        actions = actions
            .child(
                design::button(("skill-edit", index), "Edit", cx)
                    .small()
                    .ghost()
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.skill_library.edit(Some(editing.clone()), window, cx);
                    })),
            )
            .child(
                design::button(
                    ("skill-toggle", index),
                    if enabled { "Turn off" } else { "Turn on" },
                    cx,
                )
                .small()
                .ghost()
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.skill_command(
                        SkillCommand::SetEnabled {
                            name: toggle_name.clone(),
                            enabled: !enabled,
                        },
                        cx,
                    );
                })),
            )
            .child(
                design::button(("skill-remove", index), "Remove", cx)
                    .small()
                    .ghost()
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.skill_library.deleting = Some(delete_name.clone());
                        cx.notify();
                    })),
            );
    }

    card = card.child(actions);

    if showing_details {
        card = card.child(
            v_flex()
                .id(("skill-instructions-box", index))
                .test_support()
                .w_full()
                .p(t.space.small)
                .gap(t.space.hair)
                .sovra_surface(Surface::Subtle, cx)
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(cx.theme().link)
                        .child("Full instructions Sovra follows:"),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .child(skill.instructions.clone()),
                ),
        );
    }

    if view.deleting.as_ref() == Some(&name) {
        card = card
            .child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .child("Remove this installed skill and its bundled resources? The original import folder will remain."),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::button("skill-confirm-remove", "Remove skill", cx)
                            .danger()
                            .small()
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                shell.skill_command(
                                    SkillCommand::Remove {
                                        name: confirm_name.clone(),
                                    },
                                    cx,
                                );
                                shell.skill_library.deleting = None;
                            })),
                    )
                    .child(
                        design::button("skill-cancel-remove", "Keep skill", cx)
                            .small()
                            .on_click(cx.listener(|shell, _, _, cx| {
                                shell.skill_library.deleting = None;
                                cx.notify();
                            })),
                    ),
            );
    }
    card
}
