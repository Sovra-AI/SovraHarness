//! Native skill catalogue, editor and package import. All persistence goes through
//! the runtime protocol; this view never opens or executes a package itself.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, TestSupportExt as _, Window,
    div,
};
use sovra_core::{
    Command, NavTab,
    skills::{Skill, SkillCommand, SkillKind},
};

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, TextRole},
    shell::Shell,
};

pub(crate) struct SkillLibrary {
    search: Entity<InputState>,
    name: Entity<InputState>,
    title: Entity<InputState>,
    description: Entity<InputState>,
    tools: Entity<InputState>,
    body: Entity<TextareaState>,
    editing: Option<Skill>,
    creating: bool,
    details: Option<String>,
    deleting: Option<String>,
    pub(crate) notice: String,
    picker: Option<Task<()>>,
    _search: Subscription,
}

impl SkillLibrary {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search skills…"));
        let subscription = cx.subscribe(&search, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            search,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("weekly-review")),
            title: cx.new(|cx| InputState::new(window, cx).placeholder("Weekly review")),
            description: cx.new(|cx| {
                InputState::new(window, cx).placeholder("What this skill does and when to use it")
            }),
            tools: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Optional tool names, separated by spaces")
            }),
            body: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("Write the procedure and expected result…")
                    .auto_grow(6, 16)
            }),
            editing: None,
            creating: false,
            details: None,
            deleting: None,
            notice: String::new(),
            picker: None,
            _search: subscription,
        }
    }

    fn edit(&mut self, skill: Option<Skill>, window: &mut Window, cx: &mut Context<Shell>) {
        self.creating = skill.is_none();
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
        skill.name = view.name.read(cx).value().trim().into();
        skill.title = view.title.read(cx).value().trim().into();
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
    let query = view.search.read(cx).value();
    let editing = view.creating || view.editing.is_some();
    let mut content = v_flex().w_full().max_w(t.layout.reading_width).min_w_0().gap(t.space.medium)
        .child(div().sovra_text(TextRole::PageTitle, cx).child("Skills"))
        .child(div().sovra_text(TextRole::Body, cx).child("Type / in chat to use a skill. Skills guide your AI through a procedure using the tools already available."))
        .child(h_flex().flex_wrap().gap(t.space.small)
            .child(design::primary_button("skill-create", "New skill", cx).on_click(cx.listener(|shell, _, window, cx| shell.skill_library.edit(None, window, cx))))
            .child(design::button("skill-import", "Import folder", cx).on_click(cx.listener(|shell, _, _, cx| shell.import_skill(cx))))
            .child(design::button("skill-refresh", "Refresh", cx).ghost().on_click(cx.listener(|shell, _, _, cx| shell.skill_command(SkillCommand::Refresh, cx)))))
        .children((!view.notice.is_empty()).then(|| div().id("skill-library-notice").test_support().sovra_text(TextRole::Caption, cx).child(view.notice.clone())));
    if editing {
        content = content.child(editor(view, cx));
    } else {
        content = content
            .child(Input::new(&view.search).aria_label("Search skills"))
            .children(
                shell
                    .skills
                    .search_all(&query)
                    .into_iter()
                    .enumerate()
                    .map(|(index, skill)| card(skill, index, view, cx)),
            );
    }
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
        .child(content)
}

fn editor(view: &SkillLibrary, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex().id("skill-editor").test_support().w_full().min_w_0().gap(t.space.small)
        .child(div().sovra_text(TextRole::SectionTitle, cx).child(if view.creating { "Create a skill" } else { "Edit skill" }))
        .child(Input::new(&view.name).aria_label("Skill command name").readonly(!view.creating))
        .child(Input::new(&view.title).aria_label("Skill title"))
        .child(Input::new(&view.description).aria_label("When to use this skill"))
        .child(Input::new(&view.tools).aria_label("Required tools"))
        .child(Textarea::new(&view.body).aria_label("Skill instructions"))
        .child(div().sovra_text(TextRole::Caption, cx).child("Keep credentials out of skills. Required tools must already be enabled; importing a skill does not run its scripts."))
        .child(h_flex().flex_wrap().gap(t.space.small)
            .child(design::primary_button("skill-save", "Save skill", cx).on_click(cx.listener(|shell, _, _, cx| shell.save_skill(cx))))
            .child(design::button("skill-editor-back", "Back to skills", cx).ghost().on_click(cx.listener(|shell, _, _, cx| {
                shell.skill_library.creating = false; shell.skill_library.editing = None; cx.notify();
            }))))
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
    let mut card = v_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .p(t.space.medium)
        .sovra_surface(design::Surface::Matte, cx)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(format!("/{} · {}", skill.name, skill.title)),
        )
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .child(skill.description.clone()),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(cx.theme().muted_foreground)
                .child(format!(
                    "{} · {}",
                    if skill.builtin {
                        "Built-in"
                    } else {
                        "Your skill"
                    },
                    if skill.enabled { "Enabled" } else { "Disabled" }
                )),
        );
    let mut actions = h_flex()
        .flex_wrap()
        .gap(t.space.small)
        .child(
            design::button(("skill-use", index), "Use in chat", cx)
                .disabled(!enabled)
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.select_tab(NavTab::Chat, cx);
                    shell.fill_composer_draft(&format!("/{run_name} "), window, cx);
                })),
        )
        .child(
            design::button(("skill-details", index), "Instructions", cx)
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
                    .ghost()
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.skill_library.edit(Some(editing.clone()), window, cx);
                    })),
            )
            .child(
                design::button(
                    ("skill-toggle", index),
                    if enabled { "Disable" } else { "Enable" },
                    cx,
                )
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
                    .ghost()
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.skill_library.deleting = Some(delete_name.clone());
                        cx.notify();
                    })),
            );
    }
    card = card.child(actions);
    if view.details.as_ref() == Some(&name) {
        card = card.child(
            div()
                .sovra_text(TextRole::Body, cx)
                .child(skill.instructions.clone()),
        );
    }
    if view.deleting.as_ref() == Some(&name) {
        card = card.child(div().sovra_text(TextRole::Body, cx).child("Remove this installed skill and its bundled resources? The original import folder will remain."))
            .child(h_flex().flex_wrap().gap(t.space.small)
                .child(design::button("skill-confirm-remove", "Remove skill", cx).danger().on_click(cx.listener(move |shell, _, _, cx| {
                    shell.skill_command(SkillCommand::Remove { name: confirm_name.clone() }, cx); shell.skill_library.deleting = None;
                })))
                .child(design::button("skill-cancel-remove", "Keep skill", cx).on_click(cx.listener(|shell, _, _, cx| { shell.skill_library.deleting = None; cx.notify(); }))));
    }
    card
}
