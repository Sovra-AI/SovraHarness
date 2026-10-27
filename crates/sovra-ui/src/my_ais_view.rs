//! `My AI's` view: choose one of the 3 selection modes (`SingleGlobalModel`,
//! `PerAgentModel`, `DynamicTreeRouting`) in plain language, set effort per specialist,
//! and — folded away for the curious — switch the System-1 decision engine (`Laya`,
//! `OpenJev`, `EmbeddedHeuristic`) and inspect/test the `SovraSRX` routing tree.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::{
    DecisionBackend, ModelTier, RouteEvaluation, SelectionMode, SpecialistAgentSpec, TreeNode,
    TreeNodeKind, WorkspaceState,
};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

pub(crate) fn render_my_ais_view(
    state: &WorkspaceState,
    sim_input: &Entity<InputState>,
    advanced_open: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);

    let mut page = v_flex()
        .w_full()
        .max_w(t.layout.page_width)
        .mx_auto()
        .gap(t.space.large)
        .child(render_header(cx))
        .child(render_mode_selector(state.routing.mode, cx));
    if state.routing.mode == SelectionMode::PerAgentModel {
        page = page.child(render_specialists_section(state, cx));
    }
    let mut advanced = v_flex()
        .id("my-ais-advanced")
        .test_support()
        .w_full()
        .gap(t.space.small)
        .child(
            v_flex()
                .gap(t.space.hair)
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("How automatic choice works"),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child("Explore routing decisions and try a sample prompt."),
                ),
        )
        .child(
            h_flex().child(
                design::button(
                    "my-ais-advanced-toggle",
                    if advanced_open {
                        "Hide details"
                    } else {
                        "Show details"
                    },
                    cx,
                )
                .on_click(cx.listener(|shell, _, _, cx| shell.toggle_my_ais_advanced(cx))),
            ),
        );
    if advanced_open {
        advanced = advanced
            .child(render_router_engine_and_simulator(state, sim_input, cx))
            .child(render_routing_tree_section(state, cx));
    }
    v_flex()
        .id("sovra-my-ais-view")
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .p(t.space.large)
        .child(page.child(advanced))
}

fn render_header(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);

    h_flex()
        .w_full()
        .flex_wrap()
        .gap(t.space.small)
        .items_center()
        .justify_between()
        .child(design::page_header(
            "My AI's",
            "Choose how Sovra finds the right AI for each task.",
            cx,
        ))
        .child(
            design::button("open-models-catalog-btn", "Pick a model", cx).on_click(cx.listener(
                |shell, _, _, cx| {
                    shell.select_tab(sovra_core::NavTab::Models, cx);
                },
            )),
        )
}

/// Plain-language title and description; the core labels stay technical for logs.
fn mode_copy(mode: SelectionMode) -> (&'static str, &'static str) {
    match mode {
        SelectionMode::SingleGlobalModel => (
            "One AI for everything",
            "Always use the model you picked in Models.",
        ),
        SelectionMode::PerAgentModel => (
            "Choose per task",
            "Decide how much effort Sovra puts into each kind of task.",
        ),
        SelectionMode::DynamicTreeRouting => (
            "Automatic",
            "Sovra uses a quick AI for simple questions and a more powerful one for hard ones.",
        ),
    }
}

fn tier_copy(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Fast => "Quick",
        ModelTier::Standard => "Balanced",
        ModelTier::Deep => "Thorough",
    }
}

fn render_mode_selector(
    active_mode: SelectionMode,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);

    h_flex()
        .id("routing-mode-selector")
        .test_support()
        .w_full()
        .flex_wrap()
        .gap(t.space.small)
        .children(
            SelectionMode::ALL
                .into_iter()
                .enumerate()
                .map(|(idx, mode)| {
                    let selected = mode == active_mode;
                    let (title, description) = mode_copy(mode);
                    design::button(("routing-mode-btn", idx), "", cx)
                        .accessibility_label(title)
                        .selected(selected)
                        .flex_1()
                        .min_w(t.layout.card_min_width)
                        .h_auto()
                        .whitespace_normal()
                        .p(t.space.large)
                        .rounded(t.radius.card)
                        .child(
                            Icon::new(if selected {
                                IconName::CircleCheck
                            } else {
                                IconName::Circle
                            })
                            .small()
                            .text_color(if selected {
                                cx.theme().link
                            } else {
                                t.text_secondary
                            }),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .whitespace_normal()
                                .gap(t.space.small)
                                .child(div().sovra_text(TextRole::SectionTitle, cx).child(title))
                                .child(
                                    div()
                                        .sovra_text(TextRole::Caption, cx)
                                        .text_color(t.text_secondary)
                                        .child(description),
                                ),
                        )
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.set_routing_mode(mode, cx);
                        }))
                }),
        )
}

fn render_router_engine_and_simulator(
    state: &WorkspaceState,
    sim_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);

    v_flex()
        .id("routing-engine-panel")
        .test_support()
        .w_full()
        .p(t.space.medium)
        .gap(t.space.small)
        .sovra_surface(Surface::Matte, cx)
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap(t.space.small)
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .gap(t.space.hair)
                        .child(
                            div()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child("Decision engine"),
                        )
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(
                                    "The small, fast helper that judges how hard each question is.",
                                ),
                        ),
                )
                .child(
                    h_flex().flex_wrap().gap(t.space.compact).children(
                        DecisionBackend::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(idx, backend)| {
                                let is_active = state.routing.backend == backend;
                                design::button(("decision-backend-btn", idx), backend.label(), cx)
                                    .selected(is_active)
                                    .small()
                                    .on_click(cx.listener(move |shell, _, _, cx| {
                                        shell.set_decision_backend(backend, cx);
                                    }))
                            }),
                    ),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap(t.space.small)
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(sim_input)
                            .id("route-simulator-input")
                            .aria_label("Try a question to see which AI Sovra would pick"),
                    ),
                )
                .child(
                    design::primary_button("simulate-route-btn", "Try it", cx).on_click(
                        cx.listener(|shell, _, _, cx| {
                            shell.simulate_route_from_input(cx);
                        }),
                    ),
                ),
        )
        .children(
            state
                .last_route_evaluation
                .as_ref()
                .map(|eval| render_evaluation_trace(eval, cx)),
        )
}

fn render_evaluation_trace(
    eval: &RouteEvaluation,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let success = cx.theme().success;

    v_flex()
        .id("route-evaluation-summary")
        .test_support()
        .w_full()
        .p(t.space.small)
        .gap(t.space.compact)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap(t.space.small)
                .items_center()
                .justify_between()
                .child(div().sovra_text(TextRole::Body, cx).child(format!(
                    "Sovra would use {} · effort: {}",
                    eval.selected_model.model_id,
                    tier_copy(eval.tier)
                )))
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(success)
                        .child(format!(
                            "{} · {}",
                            eval.route_code,
                            eval.backend_used.default_model_id()
                        )),
                ),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(
                    eval.trace
                        .iter()
                        .map(|s| format!("[{}] {}", s.node_id, s.summary))
                        .collect::<Vec<_>>()
                        .join("  →  "),
                ),
        )
}

fn render_routing_tree_section(
    state: &WorkspaceState,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);

    v_flex()
        .id("routing-tree-nodes")
        .test_support()
        .w_full()
        .gap(t.space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("The questions Sovra asks itself"),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(format!(
                    "{} · {} steps",
                    state.routing.tree.name,
                    state.routing.tree.nodes.len()
                )),
        )
        .child(
            h_flex().w_full().flex_wrap().gap(t.space.small).children(
                state
                    .routing
                    .tree
                    .nodes
                    .iter()
                    .map(|node| render_tree_node_card(node, cx)),
            ),
        )
}

fn render_tree_node_card(node: &TreeNode, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let kind_badge = match &node.kind {
        TreeNodeKind::Decision {
            min_confidence_pct, ..
        } => format!("Question · needs {min_confidence_pct}% confidence"),
        TreeNodeKind::Rule { field } => format!("Rule · {field}"),
        TreeNodeKind::Difficulty => "How hard is it?".to_owned(),
        TreeNodeKind::Route { target } => {
            format!("Answer with · {}", tier_copy(target.tier))
        }
    };

    v_flex()
        .w(t.layout.side_panel_width)
        .p(t.space.small)
        .gap(t.space.compact)
        .sovra_surface(Surface::Matte, cx)
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.accent.violet)
                .child(kind_badge),
        )
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .child(node.label.clone()),
        )
}

fn render_specialists_section(
    state: &WorkspaceState,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);

    v_flex()
        .id("sovra-specialists-list")
        .test_support()
        .w_full()
        .gap(t.space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("Effort per task"),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(
                    "Quick is fastest and cheapest. Thorough takes longer but thinks harder. Click a button to change it.",
                ),
        )
        .children(
            state
                .specialists
                .iter()
                .enumerate()
                .map(|(idx, spec)| render_specialist_row(spec, idx, cx)),
        )
}

fn render_specialist_row(
    spec: &SpecialistAgentSpec,
    idx: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let spec_id = spec.id.clone();
    let current_tier = spec.preferred_tier;
    let next_tier = match current_tier {
        ModelTier::Fast => ModelTier::Standard,
        ModelTier::Standard => ModelTier::Deep,
        ModelTier::Deep => ModelTier::Fast,
    };

    h_flex()
        .id(("specialist-row", idx))
        .test_support()
        .w_full()
        .items_center()
        .justify_between()
        .gap(t.space.small)
        .p(t.space.small)
        .sovra_surface(Surface::Matte, cx)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(t.space.hair)
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .child(spec.name.clone()),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(spec.description.clone()),
                ),
        )
        .child(
            design::button(("specialist-tier-btn", idx), tier_copy(current_tier), cx)
                .accessibility_label(format!(
                    "Effort for {}: {}. Activate to change",
                    spec.name,
                    tier_copy(current_tier)
                ))
                .small()
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.cycle_specialist_tier(&spec_id, next_tier, cx);
                })),
        )
}
