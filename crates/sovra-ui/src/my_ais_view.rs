//! `My AI's` & Dynamic Tree Router view: configure the 3 selection modes
//! (`SingleGlobalModel`, `PerAgentModel`, `DynamicTreeRouting`), switch the System-1
//! decision engine (`Laya`, `OpenJev`, `EmbeddedHeuristic`), inspect/test the
//! `SovraSRX` + `CogniRouteSLM` routing tree, and customize the 22 specialist agents.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div, px,
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
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);

    v_flex()
        .id("sovra-my-ais-view")
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .p(t.space.large)
        .gap(t.space.medium)
        .overflow_y_scroll()
        .child(render_header(cx))
        .child(render_mode_selector(state.routing.mode, cx))
        .child(render_router_engine_and_simulator(state, sim_input, cx))
        .child(render_routing_tree_section(state, cx))
        .child(render_specialists_section(state, cx))
}

fn render_header(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let link = cx.theme().link;

    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .child(
            v_flex()
                .gap(px(2.))
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(link)
                        .child("MULTI-AGENT ORCHESTRATION · COGNIROUTE + SOVRASRX"),
                )
                .child(
                    div()
                        .sovra_text(TextRole::PageTitle, cx)
                        .child("My AI's & Routing Tree"),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .text_color(t.text_secondary)
                        .child(
                            "Select a single global model, configure each of the 22 SovraSRX specialists, or route dynamically via Laya / OpenJev.",
                        ),
                ),
        )
        .child(
            design::primary_button("open-models-catalog-btn", "Browse All Models", cx).on_click(
                cx.listener(|shell, _, _, cx| {
                    shell.select_tab(sovra_core::NavTab::Models, cx);
                }),
            ),
        )
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
        .gap(t.space.small)
        .children(
            SelectionMode::ALL
                .into_iter()
                .enumerate()
                .map(|(idx, mode)| {
                    let selected = mode == active_mode;
                    let surface = if selected {
                        Surface::NavActive
                    } else {
                        Surface::Matte
                    };
                    h_flex()
                        .id(("routing-mode-btn", idx))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(mode.label())
                        .cursor_pointer()
                        .flex_1()
                        .p(t.space.medium)
                        .items_center()
                        .gap(t.space.small)
                        .sovra_surface(surface, cx)
                        .child(
                            Icon::new(if selected {
                                IconName::CircleCheck
                            } else {
                                IconName::Circle
                            })
                            .small()
                            .text_color(if selected {
                                t.accent.violet
                            } else {
                                t.text_secondary
                            }),
                        )
                        .child(div().sovra_text(TextRole::Body, cx).child(mode.label()))
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
                .items_center()
                .justify_between()
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("System-1 Decision Model (Laya / OpenJev / Heuristic)"),
                )
                .child(
                    h_flex().gap(t.space.compact).children(
                        DecisionBackend::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(idx, backend)| {
                                let is_active = state.routing.backend == backend;
                                h_flex()
                                    .id(("decision-backend-btn", idx))
                                    .test_support()
                                    .role(Role::Button)
                                    .aria_label(backend.label())
                                    .cursor_pointer()
                                    .px(t.space.small)
                                    .py(px(6.))
                                    .rounded(t.radius.pill)
                                    .bg(if is_active {
                                        t.nav_active_background
                                    } else {
                                        t.subtle
                                    })
                                    .sovra_text(TextRole::Caption, cx)
                                    .child(backend.label())
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
                            .aria_label("Test a prompt through the routing tree"),
                    ),
                )
                .child(
                    design::primary_button("simulate-route-btn", "Test Prompt Through Tree", cx)
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.simulate_route_from_input(cx);
                        })),
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
        .gap(px(4.))
        .sovra_surface(Surface::Subtle, cx)
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(div().sovra_text(TextRole::Body, cx).child(format!(
                    "Decision: {} · Specialist: {} · Model: {} ({})",
                    eval.route_kind,
                    eval.specialist_id.as_deref().unwrap_or("None"),
                    eval.selected_model.model_id,
                    eval.tier.label()
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
        .child(div().sovra_text(TextRole::SectionTitle, cx).child(format!(
            "Active Decision Tree: {} ({} nodes · root={})",
            state.routing.tree.name,
            state.routing.tree.nodes.len(),
            state.routing.tree.root
        )))
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
        } => format!("Laya/Jev Question (min {min_confidence_pct}%)"),
        TreeNodeKind::Rule { field } => format!("Rule · {field}"),
        TreeNodeKind::Difficulty => "Difficulty + Heuristic Floor".to_owned(),
        TreeNodeKind::Route { target } => {
            format!(
                "Route Leaf · {} ({})",
                target.route_kind,
                target.tier.short_label()
            )
        }
    };

    v_flex()
        .w(px(260.))
        .p(t.space.small)
        .gap(px(4.))
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
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(format!(
                    "id={} · {} branches · default={}",
                    node.id,
                    node.branches.len(),
                    node.default_next.as_deref().unwrap_or("leaf")
                )),
        )
}

fn render_specialists_section(
    state: &WorkspaceState,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let global_ref = state.routing.global_model.as_ref();

    v_flex()
        .id("sovra-specialists-list")
        .test_support()
        .w_full()
        .gap(t.space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(format!(
                    "SovraSRX Specialist Agents ({} total — click any agent's tier/model button to customize)",
                    state.specialists.len()
                )),
        )
        .children(
            state
                .specialists
                .iter()
                .enumerate()
                .map(|(idx, spec)| render_specialist_row(spec, idx, global_ref, cx)),
        )
}

fn render_specialist_row(
    spec: &SpecialistAgentSpec,
    idx: usize,
    global_model: Option<&sovra_core::models::ModelKey>,
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
        .p(t.space.small)
        .sovra_surface(Surface::Matte, cx)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    h_flex()
                        .items_center()
                        .gap(t.space.small)
                        .child(
                            div()
                                .sovra_text(TextRole::Body, cx)
                                .child(spec.name.clone()),
                        )
                        .child(
                            div()
                                .px(t.space.compact)
                                .py(px(2.))
                                .rounded(t.radius.pill)
                                .bg(t.subtle)
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(spec.id.clone()),
                        ),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(spec.description.clone()),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap(t.space.small)
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(spec.assigned_model_label(global_model)),
                )
                .child(
                    h_flex()
                        .id(("specialist-tier-btn", idx))
                        .test_support()
                        .role(Role::Button)
                        .aria_label("Cycle agent tier and model")
                        .cursor_pointer()
                        .px(t.space.small)
                        .py(px(6.))
                        .rounded(t.radius.button)
                        .bg(t.nav_active_background)
                        .sovra_text(TextRole::Caption, cx)
                        .child(current_tier.label())
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.cycle_specialist_tier(&spec_id, next_tier, cx);
                        })),
                ),
        )
}
