//! Team page: the user's AI helpers, ported from `SovraSRX` Team ("Your
//! teammates"). Plain words throughout: each helper is a card with a face, a
//! job and one line on how they help; editing is one short form.
//!
//! The roster lives in [`sovra_core::WorkspaceState::team`] (the shell folds
//! `Event::Teams`); this view owns only its form and asks the runtime to change
//! members through `Command::Teams`.

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    switch::Switch,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    WeakEntity, Window, div,
};
use sovra_core::Command;
use sovra_core::teams::{TeamCommand, TeamMember, helper_templates, member_id_for};

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

/// "Ask my team" (or one member, by `@handle`): the shell switches to chat.
pub(crate) struct AskTeam {
    pub(crate) member: Option<String>,
}

pub(crate) struct TeamView {
    shell: WeakEntity<Shell>,
    /// The member being edited; `builtin`/`id` are kept from it on save.
    editing: Option<TeamMember>,
    adding: bool,
    error: Option<String>,
    name: Entity<InputState>,
    icon: Entity<InputState>,
    role: Entity<InputState>,
    tagline: Entity<InputState>,
    instructions: Entity<TextareaState>,
}

impl gpui_kit::EventEmitter<AskTeam> for TeamView {}

impl TeamView {
    pub(crate) fn new(
        shell: WeakEntity<Shell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        Self {
            shell,
            editing: None,
            adding: false,
            error: None,
            name: input("e.g. Grace", window, cx),
            icon: input("One emoji, e.g. 🌻", window, cx),
            role: input("e.g. Garden expert", window, cx),
            tagline: input("e.g. Knows which plants grow best and when", window, cx),
            instructions: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("In your own words, how should this helper answer?")
                    .auto_grow(3, 8)
            }),
        }
    }

    fn members(&self, cx: &Context<Self>) -> Vec<TeamMember> {
        self.shell
            .upgrade()
            .map(|shell| shell.read(cx).workspace().team.members.clone())
            .unwrap_or_default()
    }

    fn send(command: TeamCommand, cx: &mut Context<Self>) {
        let _ = cx.global::<RuntimeClient>().0.send(Command::Teams(command));
    }

    fn edit(&mut self, member: TeamMember, window: &mut Window, cx: &mut Context<Self>) {
        for (input, value) in [
            (&self.name, &member.name),
            (&self.icon, &member.icon),
            (&self.role, &member.role),
            (&self.tagline, &member.tagline),
        ] {
            input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
        self.instructions.update(cx, |input, cx| {
            input.set_value(member.instructions.clone(), window, cx);
        });
        self.editing = Some(member);
        self.adding = false;
        self.error = None;
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(mut member) = self.editing.clone() else {
            return;
        };
        let read = |input: &Entity<InputState>, cx: &Context<Self>| {
            input.read(cx).value().trim().to_owned()
        };
        member.name = read(&self.name, cx);
        if member.name.is_empty() {
            self.error = Some("Give your helper a name.".into());
            cx.notify();
            return;
        }
        member.icon = read(&self.icon, cx).chars().take(4).collect();
        if member.icon.is_empty() {
            "🙂".clone_into(&mut member.icon);
        }
        member.role = read(&self.role, cx);
        member.tagline = read(&self.tagline, cx);
        self.instructions
            .read(cx)
            .value()
            .trim()
            .clone_into(&mut member.instructions);
        if member.id.is_empty() {
            member.id = member_id_for(&member.name, &self.members(cx));
        }
        Self::send(TeamCommand::SaveMember(member), cx);
        self.editing = None;
        self.error = None;
        cx.notify();
    }

    fn add_from(&mut self, template: TeamMember, window: &mut Window, cx: &mut Context<Self>) {
        let members = self.members(cx);
        let mut member = template;
        member.id = if members.iter().any(|m| m.id == member.id) {
            member_id_for(&member.name, &members)
        } else {
            member.id
        };
        self.edit(member, window, cx);
    }
}

impl Render for TeamView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_share = self.shell.upgrade().is_some_and(|shell| {
            shell
                .read(cx)
                .groups(cx)
                .my_role()
                .is_some_and(sovra_core::group::Role::writes)
        });
        let t = design::tokens(cx);
        let members = self.members(cx);
        let mut page = v_flex()
            .w_full()
            .max_w(t.layout.page_width)
            .mx_auto()
            .gap(t.space.large)
            .child(Self::header(cx))
            .child(how_it_works(cx));
        if let Some(member) = &self.editing {
            page = page.child(self.form(member, cx));
        }
        page = page.child(
            h_flex()
                .id("team-members")
                .test_support()
                .w_full()
                .flex_wrap()
                .gap(t.space.medium)
                .children(
                    members
                        .iter()
                        .enumerate()
                        .map(|(index, member)| member_card(member, index, can_share, cx)),
                ),
        );
        page = page.child(self.add_section(&members, cx));
        v_flex()
            .id("sovra-team-view")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .p(t.space.large)
            .child(page)
    }
}

impl TeamView {
    fn header(cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(t.space.small)
            .child(
                design::page_header(
                    "Your team",
                    "A group of AI helpers, each good at something different. Ask them together and they talk it through, then Alex sums up what to do.",
                    cx,
                )
                .flex_1()
                .min_w_0(),
            )
            .child(
                design::primary_button("team-ask", "Ask my team", cx)
                    .flex_shrink_0()
                    .accessibility_label("Ask my team a question in Chat")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(AskTeam { member: None }))),
            )
    }

    fn form(&self, member: &TeamMember, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let field = |label: &'static str, child: AnyElement, cx: &Context<Self>| {
            v_flex()
                .gap(t.space.compact)
                .min_w_0()
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(label),
                )
                .child(child)
        };
        let title = if member.name.is_empty() {
            "New helper".to_owned()
        } else {
            format!("Edit {}", member.name)
        };
        let reset = member.builtin.then(|| {
            let id = member.id.clone();
            design::button("team-form-reset", "Restore original", cx)
                .ghost()
                .on_click(cx.listener(move |view, _, _, cx| {
                    Self::send(TeamCommand::ResetMember { id: id.clone() }, cx);
                    view.editing = None;
                    cx.notify();
                }))
        });
        v_flex()
            .id("team-member-form")
            .test_support()
            .w_full()
            .max_w(t.layout.reading_width)
            .gap(t.space.medium)
            .p(t.space.medium)
            .sovra_surface(Surface::Elevated, cx)
            .child(div().sovra_text(TextRole::SectionTitle, cx).child(title))
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap(t.space.medium)
                    .child(
                        div().flex_1().min_w(t.layout.filter_width).child(field(
                            "Name",
                            Input::new(&self.name)
                                .aria_label("Helper name")
                                .into_any_element(),
                            cx,
                        )),
                    )
                    .child(
                        div().w(t.layout.filter_width).child(field(
                            "Picture (one emoji)",
                            Input::new(&self.icon)
                                .aria_label("Helper emoji")
                                .into_any_element(),
                            cx,
                        )),
                    ),
            )
            .child(field(
                "What they do",
                Input::new(&self.role)
                    .aria_label("What this helper does")
                    .into_any_element(),
                cx,
            ))
            .child(field(
                "One line about them",
                Input::new(&self.tagline)
                    .aria_label("One line about this helper")
                    .into_any_element(),
                cx,
            ))
            .child(field(
                "How they answer",
                Textarea::new(&self.instructions)
                    .aria_label("How this helper answers")
                    .into_any_element(),
                cx,
            ))
            .children(self.error.clone().map(|error| {
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(cx.theme().danger)
                    .child(error)
            }))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::primary_button("team-form-save", "Save helper", cx)
                            .on_click(cx.listener(|view, _, _, cx| view.save(cx))),
                    )
                    .child(
                        design::button("team-form-cancel", "Cancel", cx).on_click(cx.listener(
                            |view, _, _, cx| {
                                view.editing = None;
                                view.error = None;
                                cx.notify();
                            },
                        )),
                    )
                    .children(reset),
            )
    }

    fn add_section(
        &self,
        members: &[TeamMember],
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let templates: Vec<TeamMember> = helper_templates()
            .into_iter()
            .filter(|template| !members.iter().any(|m| m.name == template.name))
            .collect();
        let mut section = v_flex()
            .id("team-add")
            .test_support()
            .w_full()
            .gap(t.space.medium)
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(t.space.small)
                    .child(
                        div()
                            .sovra_text(TextRole::SectionTitle, cx)
                            .child("Add a helper"),
                    )
                    .child(
                        design::button(
                            "team-add-toggle",
                            if self.adding {
                                "Hide ideas"
                            } else {
                                "Add a helper"
                            },
                            cx,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.adding = !view.adding;
                            cx.notify();
                        })),
                    ),
            );
        if self.adding {
            section = section.child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap(t.space.medium)
                    .children(templates.into_iter().enumerate().map(|(index, template)| {
                        let pick = template.clone();
                        card_shell(("team-template", index), cx)
                            .child(member_identity(&template, cx))
                            .child(
                                design::button(("team-template-add", index), "Add to my team", cx)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.add_from(pick.clone(), window, cx);
                                    })),
                            )
                    }))
                    .child(
                        card_shell("team-template-blank", cx)
                            .child(
                                div()
                                    .sovra_text(TextRole::SectionTitle, cx)
                                    .child("Make your own"),
                            )
                            .child(
                                div()
                                    .sovra_text(TextRole::Body, cx)
                                    .text_color(t.text_secondary)
                                    .child("Describe a helper in your own words."),
                            )
                            .child(design::button("team-template-new", "Start", cx).on_click(
                                cx.listener(|view, _, window, cx| {
                                    view.add_from(
                                        TeamMember {
                                            id: String::new(),
                                            name: String::new(),
                                            icon: "🙂".into(),
                                            role: String::new(),
                                            tagline: String::new(),
                                            instructions: String::new(),
                                            expertise: Vec::new(),
                                            conductor: false,
                                            active: true,
                                            builtin: false,
                                        },
                                        window,
                                        cx,
                                    );
                                }),
                            )),
                    ),
            );
        }
        section
    }
}

fn how_it_works(cx: &mut Context<TeamView>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let step = |n: &'static str, text: &'static str, cx: &Context<TeamView>| {
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
        .id("team-how-it-works")
        .w_full()
        .flex_wrap()
        .gap(t.space.small)
        .child(step("1", "Ask a question in your own words.", cx))
        .child(step("2", "The right helpers share their view.", cx))
        .child(step("3", "Alex sums up and suggests next steps.", cx))
}

fn card_shell(
    id: impl Into<gpui_kit::ElementId>,
    cx: &Context<TeamView>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let t = design::tokens(cx);
    v_flex()
        .id(id)
        .flex_1()
        .min_w(t.layout.card_min_width)
        .max_w(t.layout.card_max_width)
        .gap(t.space.small)
        .p(t.space.medium)
        .sovra_surface(Surface::Glass, cx)
}

fn member_identity(member: &TeamMember, cx: &Context<TeamView>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .items_center()
        .child(crate::chat_view::member_avatar(
            &member.icon,
            t.layout.logo_medium,
            cx,
        ))
        .child(
            v_flex()
                .min_w_0()
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .text_ellipsis()
                        .child(member.name.clone()),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(if member.conductor {
                            format!("{} · leads the team", member.role)
                        } else {
                            member.role.clone()
                        }),
                ),
        )
}

fn member_card(
    member: &TeamMember,
    index: usize,
    can_share: bool,
    cx: &mut Context<TeamView>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    // Built-in helpers come with Sovra, so only the person's own are worth sharing.
    let share = (can_share && !member.builtin).then(|| {
        let template = sovra_core::group::Template::Helper {
            name: member.name.clone(),
            icon: member.icon.clone(),
            role: member.role.clone(),
            tagline: member.tagline.clone(),
            instructions: member.instructions.clone(),
            expertise: member.expertise.clone(),
        };
        design::button(("team-member-share", index), "Share with my group", cx)
            .ghost()
            .small()
            .on_click(cx.listener(move |_, _, _, cx| {
                let _ = cx.global::<RuntimeClient>().0.send(Command::Group(
                    sovra_core::group::GroupCommand::ShareTemplate {
                        template: template.clone(),
                        with: None,
                    },
                ));
            }))
    });
    let toggled = member.clone();
    let edited = member.clone();
    let asked = member.id.clone();
    let remove = (!member.builtin).then(|| {
        let id = member.id.clone();
        design::button(("team-member-remove", index), "Remove", cx)
            .ghost()
            .small()
            .on_click(cx.listener(move |_, _, _, cx| {
                TeamView::send(TeamCommand::RemoveMember { id: id.clone() }, cx);
            }))
    });
    card_shell(("team-member", index), cx)
        .test_support()
        .opacity(if member.active {
            1.
        } else {
            t.chat.action_idle_opacity
        })
        .child(member_identity(member, cx))
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(member.tagline.clone()),
        )
        .child(
            Switch::new(("team-member-active", index))
                .checked(member.active)
                .label(if member.active {
                    "On the team"
                } else {
                    "Taking a break"
                })
                .on_click(cx.listener(move |_, checked: &bool, _, cx| {
                    let mut member = toggled.clone();
                    member.active = *checked;
                    TeamView::send(TeamCommand::SaveMember(member), cx);
                })),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.compact)
                .children(member.active.then(|| {
                    design::button(
                        ("team-member-ask", index),
                        format!("Ask {}", member.name),
                        cx,
                    )
                    .small()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(AskTeam {
                            member: Some(asked.clone()),
                        });
                    }))
                }))
                .child(
                    design::button(("team-member-edit", index), "Edit", cx)
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.edit(edited.clone(), window, cx);
                        })),
                )
                .children(share)
                .children(remove),
        )
}

/// In the chat box: one button switches between Sovra alone and the whole team.
pub(crate) fn composer_team_button(
    shell: &Shell,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let team = shell.state.team_mode;
    let button = design::round_icon_button("composer-team-mode", t.chat.send_size)
        .icon(gpui_kit::component::Icon::new(
            gpui_kit::assets::IconName::Users,
        ))
        .selected(team)
        .accessibility_label(if team {
            "My team is answering. Click so only Sovra answers"
        } else {
            "Ask my team instead of Sovra"
        })
        .tooltip(if team {
            "Your team answers · click for Sovra only"
        } else {
            "Ask my team"
        })
        .on_click(cx.listener(|shell, _, _, cx| {
            shell.state.team_mode = !shell.state.team_mode;
            shell.refresh(cx);
        }));
    if team {
        button
            .bg(cx.theme().primary)
            .text_color(cx.theme().primary_foreground)
    } else {
        button.ghost()
    }
}

/// Shown above the chat box while the team is on, so it is never a surprise.
/// While a team turn runs, it becomes the live board instead: replies stream
/// into the chat above, which scrolls the in-chat board out of view.
pub(crate) fn composer_team_note(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if let Some(activity) = live_team_activity(shell) {
        return Some(
            crate::agui_widgets::render_docked_team_board(&activity, cx).into_any_element(),
        );
    }
    if !shell.state.team_mode {
        return None;
    }
    let t = design::tokens(cx);
    let helpers = shell.state.team.active().filter(|m| !m.conductor).count();
    Some(
        h_flex()
            .id("composer-team-note")
            .test_support()
            .w_full()
            .min_w_0()
            .items_center()
            .justify_between()
            .gap(t.space.compact)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .text_ellipsis()
                    .child(format!("Your team ({helpers} helpers) will answer")),
            )
            .child(
                design::button("composer-team-off", "Ask Sovra only", cx)
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.state.team_mode = false;
                        shell.refresh(cx);
                    })),
            )
            .into_any_element(),
    )
}

/// The team card of the turn that is still running in this conversation.
fn live_team_activity(shell: &Shell) -> Option<sovra_core::teams::TeamActivity> {
    if !shell.state.is_generating {
        return None;
    }
    shell.state.messages.iter().rev().find_map(|message| {
        message.widgets.iter().find_map(|widget| match widget {
            sovra_core::AgUiWidget::TeamActivity(activity) if !activity.finished => {
                Some(activity.clone())
            }
            _ => None,
        })
    })
}
