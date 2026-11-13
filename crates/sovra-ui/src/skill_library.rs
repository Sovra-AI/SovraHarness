//! Skills catalogue: a short list of what Sovra can do, plus a plain form for your own.
//! Persistence goes through the runtime protocol. This view never opens or runs a package.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    TestSupportExt as _, Window, div, relative,
};
use sovra_core::{
    Command, NavTab,
    skills::{Skill, SkillCommand, SkillEvent, SkillKind, valid_name},
};

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
    shell::Shell,
};

/// A filter chip: everything, or exactly one catalogue section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SkillCategoryFilter {
    #[default]
    All,
    Research,
    Files,
    Computer,
    Specialists,
    Presentation,
    Workspace,
    Custom,
}

impl SkillCategoryFilter {
    const ALL: [Self; 8] = [
        Self::All,
        Self::Research,
        Self::Files,
        Self::Computer,
        Self::Specialists,
        Self::Presentation,
        Self::Workspace,
        Self::Custom,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::All => "skill-filter-all",
            Self::Research => "skill-filter-research",
            Self::Files => "skill-filter-files",
            Self::Computer => "skill-filter-computer",
            Self::Specialists => "skill-filter-specialists",
            Self::Presentation => "skill-filter-presentation",
            Self::Workspace => "skill-filter-workspace",
            Self::Custom => "skill-filter-custom",
        }
    }

    /// The section this chip shows; `None` shows them all.
    fn group(self) -> Option<&'static Group> {
        let key = match self {
            Self::All => return None,
            Self::Research => "Research",
            Self::Files => "Files",
            Self::Computer => "Computer",
            Self::Specialists => "Specialists",
            Self::Presentation => "Presentation",
            Self::Workspace => "Workspace",
            Self::Custom => OWN,
        };
        GROUPS.iter().find(|group| group.key == key)
    }

    fn label(self) -> &'static str {
        self.group().map_or("All skills", |group| group.heading)
    }

    fn icon(self) -> IconName {
        self.group()
            .map_or(IconName::LayoutDashboard, |group| group.icon)
    }

    fn matches(self, skill: &Skill) -> bool {
        self.group()
            .is_none_or(|group| group.key == group_key(skill))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorExit {
    Editing,
    ConfirmDiscard,
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
    exit: EditorExit,
    scroll: gpui_kit::ScrollHandle,
    pending_save: Option<Skill>,
    advanced_open: bool,
    /// Draft value of [`Skill::auto_apply`]; saved with the form.
    auto_apply: bool,
    deleting: Option<String>,
    pub(crate) notice: String,
    /// Width of the skills column, which sets how many cards fit in a row.
    width_class: design::WidthClass,
    picker: Option<Task<()>>,
    _search: Subscription,
}

impl SkillLibrary {
    pub(crate) fn observe_save(&mut self, event: &SkillEvent, cx: &Context<Shell>) {
        let Some(pending) = &self.pending_save else {
            return;
        };
        match event {
            SkillEvent::Saved(saved) if saved.as_ref() == pending => {
                let unchanged = self.title.read(cx).value().trim() == pending.title
                    && self.description.read(cx).value().trim() == pending.description
                    && self.body.read(cx).value().as_ref() == pending.instructions
                    && self
                        .tools
                        .read(cx)
                        .value()
                        .split_whitespace()
                        .eq(pending.tools.iter().map(String::as_str))
                    && self.auto_apply == pending.auto_apply;
                self.editing = self.pending_save.take();
                self.creating = false;
                self.notice = if unchanged {
                    "Saved. Your skill is available under Made by you."
                } else {
                    "The earlier version was saved. Save again to keep your latest changes."
                }
                .into();
            }
            SkillEvent::Error(message) => {
                self.pending_save = None;
                self.notice = format!("Your changes are still here. Try saving again. {message}");
            }
            SkillEvent::Loaded { .. } | SkillEvent::Saved(_) => self.notice = "Saving…".into(),
        }
    }

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
            exit: EditorExit::Editing,
            scroll: gpui_kit::ScrollHandle::new(),
            pending_save: None,
            advanced_open: false,
            auto_apply: true,
            deleting: None,
            notice: String::new(),
            width_class: design::WidthClass::Compact,
            picker: None,
            _search: subscription,
        }
    }

    fn has_changes(&self, cx: &Context<Shell>) -> bool {
        let original = self.editing.as_ref();
        self.name.read(cx).value().trim() != original.map_or("", |skill| &skill.name)
            || self.title.read(cx).value().trim() != original.map_or("", |skill| &skill.title)
            || self.description.read(cx).value().trim()
                != original.map_or("", |skill| &skill.description)
            || self.body.read(cx).value().as_ref()
                != original.map_or("", |skill| &skill.instructions)
            || !self.tools.read(cx).value().split_whitespace().eq(original
                .into_iter()
                .flat_map(|skill| skill.tools.iter().map(String::as_str)))
            || self.auto_apply != original.is_none_or(|skill| skill.auto_apply)
    }

    fn close_editor(&mut self) {
        self.creating = false;
        self.editing = None;
        self.exit = EditorExit::Editing;
        self.notice.clear();
    }

    fn edit(&mut self, skill: Option<Skill>, window: &mut Window, cx: &mut Context<Shell>) {
        if self.pending_save.is_some() {
            return;
        }
        self.exit = EditorExit::Editing;
        self.creating = skill.is_none();
        self.advanced_open = skill.as_ref().is_some_and(|s| !s.tools.is_empty());
        self.auto_apply = skill.as_ref().is_none_or(|s| s.auto_apply);
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
    pub(crate) fn confirm_save_for_test(&mut self, cx: &Context<Shell>) {
        let skill = self.pending_save.clone().expect("test submitted a skill");
        self.observe_save(&SkillEvent::Saved(Box::new(skill)), cx);
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

impl Shell {
    pub(crate) fn show_skill_help(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.skill_library.category = SkillCategoryFilter::All;
        self.skill_library
            .search
            .update(cx, |input, cx| input.set_value(name, window, cx));
        // An unfinished custom skill stays open; the requested card appears once
        // the person saves or cancels that edit.
        if self.skill_library.creating || self.skill_library.editing.is_some() {
            self.skill_library.notice =
                "Finish or cancel your current skill edit to see the requested helper.".into();
        }
        self.select_tab(NavTab::Skills, cx);
    }

    fn save_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = &self.skill_library;
        if view.pending_save.is_some() {
            return;
        }
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
            auto_apply: true,
            builtin: false,
            license: None,
            compatibility: None,
            metadata: std::collections::BTreeMap::new(),
        });
        let raw_title = view.title.read(cx).value().trim().to_owned();
        let raw_name = view.name.read(cx).value().trim().to_owned();
        skill.title = raw_title;
        skill.name = if let Some(original) = &view.editing {
            original.name.clone()
        } else if raw_name.is_empty() {
            self.skills.available_command_name(&skill.title)
        } else if valid_name(&raw_name) {
            raw_name
        } else {
            self.skill_library.notice = "Use lowercase letters, numbers and hyphens for the shortcut, or leave it blank so Sovra can make one.".into();
            cx.notify();
            return;
        };
        if view.creating && self.skills.find(&skill.name).is_some() {
            self.skill_library.notice = "That shortcut is already in use. Leave Shortcut blank to make a new one, or go back and edit the existing skill.".into();
            cx.notify();
            return;
        }
        skill.description = view.description.read(cx).value().trim().into();
        skill.tools = view
            .tools
            .read(cx)
            .value()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        skill.instructions = view.body.read(cx).value().to_string();
        skill.auto_apply = view.auto_apply;
        match skill.validate() {
            Ok(()) => {
                self.skill_library
                    .name
                    .update(cx, |input, cx| input.set_value(&skill.name, window, cx));
                if cx
                    .global::<RuntimeClient>()
                    .0
                    .send(Command::Skills(SkillCommand::Save(Box::new(skill.clone()))))
                    .is_ok()
                {
                    self.skill_library.exit = EditorExit::Editing;
                    self.skill_library.pending_save = Some(skill);
                    self.skill_library.notice = "Saving…".into();
                } else {
                    self.skill_library.notice =
                        "Sovra could not save right now. Your changes are still here; try again."
                            .into();
                }
                cx.notify();
            }
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
        .track_scroll(&view.scroll)
        .items_center()
        .p(t.space.medium)
        .child(
            v_flex()
                .id("skills-column")
                .test_support()
                .relative()
                .w_full()
                .max_w(t.layout.page_width)
                .min_w_0()
                .mx_auto()
                .flex_shrink_0()
                .gap(t.space.large)
                // The grid follows this column, not the window, so it also fits
                // beside the sidebar and at large text sizes.
                .child(design::width_probe(cx, |shell: &mut Shell| {
                    &mut shell.skill_library.width_class
                }))
                .child(render_header(
                    shell,
                    view,
                    editing,
                    view.pending_save.is_some() || editing,
                    cx,
                ))
                .children((!view.notice.is_empty()).then(|| {
                    div()
                        .id("skill-library-notice")
                        .test_support()
                        .aria_label(view.notice.clone())
                        .w_full()
                        .p(t.space.small)
                        .sovra_surface(Surface::Subtle, cx)
                        .sovra_text(TextRole::Caption, cx)
                        .child(view.notice.clone())
                }))
                .children(
                    (view.exit == EditorExit::ConfirmDiscard).then(|| discard_confirmation(cx)),
                )
                .children(editing.then(|| editor(view, cx)))
                .children((!editing).then(|| render_categories(shell, view, &query, cx)))
                .children((!editing).then(|| render_skills_catalog(shell, view, &query, cx))),
        )
}

/// Page title, the search box and the page's own actions in one welcoming band.
fn render_header(
    shell: &Shell,
    view: &SkillLibrary,
    editing: bool,
    saving: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let total = shell.skills.search_all("").len();
    let tip = format!(
        "{total} skills ready to use. In chat, type / to pick one, or /help to come back here."
    );
    let actions = h_flex()
        .flex_wrap()
        .items_center()
        .gap(t.space.small)
        .child(
            design::primary_button("skill-create", "Add your own", cx)
                .disabled(saving)
                .icon(IconName::Plus)
                .tooltip("Teach Sovra a reusable step-by-step recipe in your own words")
                .on_click(cx.listener(|shell, _, window, cx| {
                    shell.skill_library.edit(None, window, cx);
                })),
        )
        .child(
            design::button("skill-import", "Import", cx)
                .disabled(saving)
                .ghost()
                .icon(IconName::FolderOpen)
                .tooltip("Add a saved skill folder from your computer")
                .on_click(cx.listener(|shell, _, _, cx| shell.import_skill(cx))),
        )
        .child(
            design::button("skill-refresh", "Refresh", cx)
                .disabled(saving)
                .ghost()
                .tooltip("Reload skills from disk")
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.skill_command(SkillCommand::Refresh, cx);
                })),
        );

    let mut band = v_flex()
        .id("skills-hero")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .p(t.space.large)
        .sovra_surface(Surface::HeroBanner, cx)
        .rounded(t.radius.panel)
        .child(design::page_header(
            "Skills",
            "Everything Sovra can help with. Pick one and it fills in the chat for you — nothing is sent until you press Send.",
            cx,
        ));
    if !editing {
        band = band.child(
            div().w_full().max_w(t.layout.reading_width).child(
                Input::new(&view.search)
                    .id("skill-search")
                    .aria_label("Search skills")
                    .prefix(
                        Icon::new(IconName::Search)
                            .small()
                            .text_color(t.text_secondary),
                    )
                    .cleanable(true),
            ),
        );
    }
    band.child(
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(t.space.small)
            .child(actions)
            .child(
                div()
                    .id("skills-tip")
                    .test_support()
                    .min_w_0()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child(tip),
            ),
    )
}

fn render_categories(
    shell: &Shell,
    view: &SkillLibrary,
    query: &str,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let all_matches = shell.skills.search_all(query);

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
            let selected = view.category == cat;
            let chip = design::button(cat.id(), format!("{} · {count}", cat.label()), cx)
                .small()
                .icon(cat.icon())
                .rounded(t.radius.pill)
                .px(t.space.small)
                .selected(selected)
                .accessibility_label(format!("{}, {count} skills", cat.label()));
            let chip = if selected {
                chip.primary()
            } else {
                chip.ghost().border_1().border_color(t.material_border)
            };
            Some(chip.on_click(cx.listener(move |shell, _, _, cx| {
                shell.skill_library.category = cat;
                cx.notify();
            })))
        }))
}

/// One catalogue section: which skills it holds and how it introduces itself.
struct Group {
    /// The skill category it gathers; [`OWN`] for skills people made, [`OTHER`]
    /// for built-in categories this page does not know yet.
    key: &'static str,
    heading: &'static str,
    blurb: &'static str,
    icon: IconName,
    tone: fn(&design::TileTokens) -> design::TileTone,
}

const OWN: &str = "Your skills";
const OTHER: &str = "";

/// Page order: your own first, everyday helpers next, shortcuts to Sovra's pages last.
const GROUPS: [Group; 8] = [
    Group {
        key: OWN,
        heading: "Made by you",
        blurb: "Recipes you taught Sovra in your own words.",
        icon: IconName::BookOpen,
        tone: |tiles| tiles.categories[4],
    },
    Group {
        key: "Research",
        heading: "Web search",
        blurb: "Look things up online, with sources you can check.",
        icon: IconName::Globe,
        tone: |tiles| tiles.categories[1],
    },
    Group {
        key: "Files",
        heading: "Files & folders",
        blurb: "Read, find, save and organise your documents.",
        icon: IconName::FileText,
        tone: |tiles| tiles.categories[3],
    },
    Group {
        key: "Computer",
        heading: "Your computer",
        blurb: "Get things done on this computer. Sovra asks before changing anything.",
        icon: IconName::SquareTerminal,
        tone: |tiles| tiles.categories[5],
    },
    Group {
        key: "Specialists",
        heading: "AI helpers",
        blurb: "Experts Sovra calls on for a particular kind of job.",
        icon: IconName::Users,
        tone: |tiles| tiles.categories[2],
    },
    Group {
        key: "Presentation",
        heading: "Tables & cards",
        blurb: "Turn answers into tidy tables and cards.",
        icon: IconName::Table,
        tone: |tiles| tiles.neutral,
    },
    Group {
        key: "Workspace",
        heading: "Quick actions",
        blurb: "Shortcuts that jump straight to a part of Sovra.",
        icon: IconName::Sparkles,
        tone: |tiles| tiles.categories[0],
    },
    Group {
        key: OTHER,
        heading: "More skills",
        blurb: "Other things Sovra can help with.",
        icon: IconName::Sparkles,
        tone: |tiles| tiles.categories[0],
    },
];

fn group_key(skill: &Skill) -> &'static str {
    if !skill.builtin || skill.category == OWN {
        return OWN;
    }
    GROUPS
        .iter()
        .find(|group| group.key != OTHER && group.key == skill.category)
        .map_or(OTHER, |group| group.key)
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
        "terminal" => IconName::SquareTerminal,
        "deploy-website" => IconName::Globe,
        "stock-market" | "progress" => IconName::TrendingUp,
        "housing-market" | "housing-advisor" => IconName::House,
        "image-search" | "image-generation" => IconName::Image,
        "artifacts" | "save-artifact" => IconName::LibraryBig,
        "settings" => IconName::Settings,
        "models" => IconName::Box,
        _ => GROUPS
            .iter()
            .find(|group| group.key == group_key(skill))
            .map_or(IconName::BookOpen, |group| group.icon),
    }
}

/// A rounded coloured square holding an icon; the colour says which group it is in.
pub(crate) fn tile(icon: IconName, tone: design::TileTone, cx: &App) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(t.layout.icon_tile)
        .rounded(t.radius.button)
        .bg(tone.background)
        .child(
            Icon::new(icon)
                .size(t.layout.icon_small)
                .text_color(tone.foreground),
        )
}

fn columns(width: design::WidthClass) -> usize {
    match width {
        design::WidthClass::Compact => 1,
        design::WidthClass::Medium => 2,
        design::WidthClass::Expanded
        | design::WidthClass::Large
        | design::WidthClass::ExtraLarge => 3,
    }
}

/// Equal-width rows, so a short last row keeps the same card width as the rest.
fn grid(cells: Vec<AnyElement>, columns: usize, cx: &App) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut rows = Vec::new();
    let mut cells = cells.into_iter().peekable();
    while cells.peek().is_some() {
        let mut row = h_flex()
            .w_full()
            .min_w_0()
            .items_stretch()
            .gap(t.space.medium);
        for _ in 0..columns {
            let cell = div().flex().flex_1().flex_basis(relative(0.)).min_w_0();
            row = row.child(match cells.next() {
                Some(card) => cell.child(card),
                None => cell,
            });
        }
        rows.push(row);
    }
    v_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .children(rows)
}

fn render_skills_catalog(
    shell: &Shell,
    view: &SkillLibrary,
    query: &str,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let can_share = shell
        .groups(cx)
        .my_role()
        .is_some_and(sovra_core::group::Role::writes);
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
            .items_center()
            .p(t.space.section)
            .gap(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(tile(IconName::Search, t.tiles.neutral, cx))
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("No matching skills"),
            )
            .child(
                div()
                    .text_center()
                    .sovra_text(TextRole::Body, cx)
                    .text_color(t.text_secondary)
                    .child(
                        "Try a simpler word like “letter”, “photos” or “budget”, or choose “All skills” above.",
                    ),
            )
            .into_any_element();
    }

    let columns = columns(view.width_class);
    let has_own = matches
        .iter()
        .any(|(_, skill)| group_key(skill) == "Your skills");
    let offer_own =
        view.category == SkillCategoryFilter::All && query.trim().is_empty() && !has_own;
    let mut sections = v_flex().w_full().gap(t.space.section);
    for group in &GROUPS {
        let members: Vec<(usize, &Skill)> = matches
            .iter()
            .copied()
            .filter(|(_, skill)| group_key(skill) == group.key)
            .collect();
        if members.is_empty() {
            continue;
        }
        let tone = (group.tone)(&t.tiles);
        let count = members.len();
        let cards = members
            .into_iter()
            .map(|(index, skill)| card(skill, index, tone, view, can_share, cx).into_any_element())
            .collect();
        sections = sections.child(
            v_flex()
                .w_full()
                .min_w_0()
                .gap(t.space.medium)
                .child(section_heading(group, tone, count, cx))
                .child(grid(cards, columns, cx)),
        );
    }
    if offer_own {
        sections = sections.child(render_make_your_own(cx));
    }
    sections.into_any_element()
}

fn section_heading(
    group: &Group,
    tone: design::TileTone,
    count: usize,
    cx: &App,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(t.space.small)
        .child(tile(group.icon, tone, cx))
        .child(
            v_flex()
                .min_w_0()
                .flex_1()
                .child(
                    h_flex()
                        .min_w_0()
                        .items_center()
                        .gap(t.space.small)
                        .child(
                            div()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(group.heading),
                        )
                        .child(
                            div()
                                .px(t.space.small)
                                .rounded(t.radius.pill)
                                .bg(tone.background)
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(tone.foreground)
                                .child(count.to_string()),
                        ),
                )
                .child(
                    div()
                        .min_w_0()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(group.blurb),
                ),
        )
}

/// Shown until someone saves their first skill, so the editor is easy to find.
fn render_make_your_own(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .id("skills-make-your-own")
        .test_support()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .items_center()
        .gap(t.space.medium)
        .p(t.space.large)
        .rounded(t.radius.card)
        .border_1()
        .border_dashed()
        .border_color(t.material_border_strong)
        .child(tile(IconName::Plus, t.tiles.categories[4], cx))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(t.space.hair)
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("Teach Sovra something new"),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .text_color(t.text_secondary)
                        .child("Write the steps once, like you would for a friend, and Sovra follows them whenever you ask."),
                ),
        )
        .child(
            design::button("skills-make-your-own-start", "Add your own", cx)
                .icon(IconName::Plus)
                .on_click(cx.listener(|shell, _, window, cx| {
                    shell.skill_library.edit(None, window, cx);
                })),
        )
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
                    "Add your own"
                } else {
                    "Edit"
                }),
        )
        .child(field(
            "Name",
            "For example, Weekly family meal planner.",
            Input::new(&view.title)
                .aria_label("Skill title")
                .into_any_element(),
            cx,
        ))
        .child(field(
            "What it does",
            "One sentence about when it helps. Sovra reads this to decide when to use it.",
            Input::new(&view.description)
                .aria_label("When to use this skill")
                .into_any_element(),
            cx,
        ))
        .child(field(
            "Steps",
            "Explain it the way you would to a friend.",
            Textarea::new(&view.body)
                .aria_label("Skill instructions")
                .into_any_element(),
            cx,
        ))
        .child(
            v_flex()
                .gap(t.space.hair)
                .child(
                    Checkbox::new("skill-editor-auto-apply")
                        .cursor_pointer()
                        .label("Use it on its own when a request fits")
                        .checked(view.auto_apply)
                        .on_click(cx.listener(|shell, checked, _, cx| {
                            shell.skill_library.auto_apply = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child("Turn this off to use it only when you pick it from the / menu."),
                ),
        )
        .child(
            h_flex().child(
                design::button(
                    "skill-editor-advanced-toggle",
                    if view.advanced_open {
                        "Hide extra options"
                    } else {
                        "Extra options"
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
        form = form
            .child(field(
                "Shortcut",
                "Leave this blank and Sovra makes one from the name.",
                Input::new(&view.name)
                    .aria_label("Skill command name")
                    .readonly(!view.creating || view.pending_save.is_some())
                    .into_any_element(),
                cx,
            ))
            .child(field(
                "Tools",
                "Leave this blank unless a setup note named a tool. Keep passwords out.",
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
            .child("Keep passwords out. Importing a skill does not run anything on your computer."),
    )
    .child(
        h_flex()
            .flex_wrap()
            .gap(t.space.small)
            .child(
                design::primary_button(
                    "skill-save",
                    if view.pending_save.is_some() {
                        "Saving…"
                    } else {
                        "Save"
                    },
                    cx,
                )
                .disabled(view.pending_save.is_some())
                .on_click(cx.listener(|shell, _, window, cx| shell.save_skill(window, cx))),
            )
            .child(
                design::button("skill-editor-back", "Back", cx)
                    .disabled(view.pending_save.is_some())
                    .ghost()
                    .on_click(cx.listener(|shell, _, _, cx| {
                        if shell.skill_library.has_changes(cx) {
                            shell.skill_library.exit = EditorExit::ConfirmDiscard;
                            shell
                                .skill_library
                                .scroll
                                .set_offset(gpui_kit::Point::default());
                        } else {
                            shell.skill_library.close_editor();
                        }
                        cx.notify();
                    })),
            ),
    )
}

fn discard_confirmation(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("skill-discard-confirm")
        .test_support()
        .gap(t.space.small)
        .child(
            design::paragraph("Leave without saving your changes?").sovra_text(TextRole::Body, cx),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::primary_button("skill-keep-editing", "Keep editing", cx).on_click(
                        cx.listener(|shell, _, _, cx| {
                            shell.skill_library.exit = EditorExit::Editing;
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    design::button("skill-discard", "Discard changes", cx)
                        .danger()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.skill_library.close_editor();
                            cx.notify();
                        })),
                ),
        )
}

fn card(
    skill: &Skill,
    index: usize,
    tone: design::TileTone,
    view: &SkillLibrary,
    can_share: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let name = skill.name.clone();
    let run_name = name.clone();
    let toggle_name = name.clone();
    let delete_name = name.clone();
    let confirm_name = name.clone();
    let editing = skill.clone();
    let enabled = skill.enabled;
    let is_action = matches!(skill.kind, SkillKind::Action(_));
    let kind = skill.kind.clone();
    let copy = skill.friendly_card_copy();
    let sample_draft = copy.sample_draft.clone();
    let use_draft = format!("/{run_name} ");
    let show_example = sample_draft != use_draft;
    let mut visible = format!(
        "{}\n{}\n{}",
        copy.friendly_title, copy.summary, copy.example_display
    );
    if !enabled {
        visible.push_str("\nOff");
    }

    let primary_cta_label = if is_action { "Open now" } else { "Try in chat" };
    let primary_accessible_label = format!("{primary_cta_label}: {}", copy.friendly_title);

    let mut card = v_flex()
        .id(("skill-card", index))
        .test_support()
        .aria_label(visible)
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .p(t.space.medium)
        .sovra_surface(Surface::Glass, cx)
        .hover(|style| style.border_color(t.material_border_strong))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap(t.space.small)
                .child(tile(skill_icon(skill), tone, cx))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(copy.friendly_title),
                )
                .children((!enabled).then(|| {
                    div()
                        .flex_shrink_0()
                        .px(t.space.small)
                        .rounded(t.radius.pill)
                        .bg(t.tiles.neutral.background)
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.tiles.neutral.foreground)
                        .child("Off")
                })),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(copy.summary),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .py(t.space.compact)
                .px(t.space.small)
                .rounded(t.radius.input)
                .border_l_2()
                .border_color(tone.foreground)
                .bg(t.subtle)
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(copy.example_display),
        )
        // Pushes the buttons to the bottom so cards in a row line up.
        .child(div().flex_1());

    let mut actions = h_flex()
        .flex_wrap()
        .items_center()
        .gap(t.space.small)
        .pt(t.space.compact)
        .child(
            design::primary_button(("skill-use", index), primary_cta_label, cx)
                .small()
                .icon(if is_action {
                    IconName::ArrowRight
                } else {
                    IconName::MessageCircle
                })
                .disabled(!enabled)
                .accessibility_label(primary_accessible_label)
                .on_click(cx.listener(move |shell, _, window, cx| {
                    match kind {
                        SkillKind::Action(action) => {
                            // Opening a page must not replace an unfinished chat draft.
                            let draft = shell.composer_input.read(cx).value().to_string();
                            shell.run_skill_action(action, &draft, window, cx);
                        }
                        SkillKind::Procedure => {
                            shell.select_tab(NavTab::Chat, cx);
                            shell.fill_composer_draft(&use_draft, window, cx);
                        }
                    }
                })),
        )
        .children(show_example.then(|| {
            design::button(("skill-try-example", index), "Try the example", cx)
                .small()
                .ghost()
                .disabled(!enabled)
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.select_tab(NavTab::Chat, cx);
                    shell.fill_composer_draft(&sample_draft, window, cx);
                }))
        }));

    if !skill.builtin && can_share {
        // Only the words travel; any tools it asks for stay behind.
        let template = sovra_core::group::Template::Skill {
            name: skill.title.clone(),
            description: skill.description.clone(),
            instructions: skill.instructions.clone(),
        };
        actions = actions.child(
            design::button(("skill-share", index), "Share with my group", cx)
                .small()
                .ghost()
                .on_click(move |_, _, cx| {
                    let _ = cx.global::<RuntimeClient>().0.send(Command::Group(
                        sovra_core::group::GroupCommand::ShareTemplate {
                            template: template.clone(),
                            with: None,
                        },
                    ));
                }),
        );
    }
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

    if view.deleting.as_ref() == Some(&name) {
        card = card
            .child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .child("Remove this skill from Sovra?"),
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
