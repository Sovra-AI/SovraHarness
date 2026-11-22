//! The Plugins page: the plugins you have as cards, ways to add one, and a catalog's
//! listing. Everything it changes goes through `PluginCommand`; it never runs a package.
//! A plugin that brings code is turned on only after its card has shown what the
//! code can reach and the person has chosen **Allow and turn on**.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    TestSupportExt as _, Window, div, relative,
};
use sovra_core::{
    Command,
    plugins::{
        CODE_API_VERSION, InstalledPlugin, OFFICIAL_PLUGINS, OfficialPlugin, PLUGIN_TEMPLATE,
        PluginChanges, PluginCommand, PluginEntry, PluginEvent, PluginPackage, PluginState,
        official_plugin,
    },
};

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
    shell::Shell,
    skill_library::tile,
};

const INSTALLED_PAGE_SIZE: usize = 20;
/// Room for each official plugin's action buttons in one ID range.
const OFFICIAL_ACTION_SLOTS: usize = 8;
mod cleanup;

pub(crate) struct PluginBrowser {
    scroll: gpui_kit::ScrollHandle,
    pub(crate) state: PluginState,
    url: Entity<InputState>,
    github: Entity<InputState>,
    search: Entity<InputState>,
    installed_page: usize,
    /// The plugin whose **Remove** was pressed once; a second press removes it.
    confirm_remove: Option<String>,
    /// The code plugin whose **Turn on** was pressed; it turns on only after the
    /// person has seen what its code can reach and allowed it.
    confirm_code: Option<String>,
    /// The official plugin whose **Install** is running, so only its button says so.
    installing: Option<String>,
    /// The page column's width, so the two ways of adding sit side by side only
    /// when both fit.
    width_class: design::WidthClass,
    _search: Subscription,
}

impl PluginBrowser {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a plugin by name or words"));
        let subscription = cx.subscribe(&search, |shell, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                shell.plugins.installed_page = 0;
                cx.notify();
            }
        });
        Self {
            scroll: gpui_kit::ScrollHandle::new(),
            state: PluginState::default(),
            url: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("https://publisher.github.io/plugins/catalog.json")
            }),
            github: cx.new(|cx| {
                InputState::new(window, cx).placeholder("https://github.com/owner/repository")
            }),
            search,
            installed_page: 0,
            confirm_remove: None,
            confirm_code: None,
            installing: None,
            width_class: design::WidthClass::Compact,
            _search: subscription,
        }
    }
}

impl Shell {
    pub(crate) fn plugin_command(&mut self, command: PluginCommand, cx: &mut Context<Self>) {
        if self.plugins.state.busy {
            return;
        }
        self.plugins.confirm_remove = None;
        self.plugins.confirm_code = None;
        self.plugins.installing = match &command {
            PluginCommand::InstallOfficial { id } => Some(id.clone()),
            PluginCommand::Browse { .. }
            | PluginCommand::Install { .. }
            | PluginCommand::NextPage
            | PluginCommand::PreviousPage
            | PluginCommand::Inspect { .. }
            | PluginCommand::SetEnabled { .. }
            | PluginCommand::AddFromGitHub { .. }
            | PluginCommand::CheckForUpdate { .. }
            | PluginCommand::ApplyUpdate { .. }
            | PluginCommand::Remove { .. }
            | PluginCommand::AllowCode { .. } => None,
        };
        if matches!(&command, PluginCommand::Browse { .. }) {
            self.plugins.state.catalog = None;
        }
        let event = match cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Plugins(command))
        {
            Ok(()) => PluginEvent::Busy,
            Err(_) => PluginEvent::Error(
                "The runtime is unavailable. Try again when Sovra is ready.".into(),
            ),
        };
        self.plugins.state.apply(&event);
        cx.notify();
    }
}

/// The Plugins page. Plugins bring skills *and* suggested connections, so they have
/// their own page instead of living inside Skills or Connections.
pub(crate) fn page(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("sovra-plugins-view")
        .test_support()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .overflow_y_scroll()
        .track_scroll(&shell.plugins.scroll)
        .items_center()
        .p(t.space.medium)
        .child(
            v_flex()
                .id("plugin-browser")
                .test_support()
                .relative()
                .w_full()
                .max_w(t.layout.page_width)
                .min_w_0()
                .mx_auto()
                .flex_shrink_0()
                .gap(t.space.large)
                // Measures this column, not the window, so the layout also fits
                // beside the sidebar and at large text sizes.
                .child(design::width_probe(cx, |shell: &mut Shell| {
                    &mut shell.plugins.width_class
                }))
                .child(design::page_header(
                    "Plugins",
                    "A plugin adds ready-made skills or tools to Sovra, and can suggest online services to connect. A plugin's own code runs only in a sealed box, and Sovra asks before it does anything on your computer. Plugins you add yourself stay off until you turn them on.",
                    cx,
                ))
                .children(notice(&shell.plugins.state, cx))
                .child(official_section(shell, cx))
                .child(installed_section(shell, cx))
                .child(add_section(shell, cx))
                .children(catalog_section(shell, cx)),
        )
}

/// What just happened, or what went wrong. Hidden when there is nothing to say.
fn notice(state: &PluginState, cx: &Context<Shell>) -> Option<impl IntoElement + use<>> {
    let t = design::tokens(cx);
    if state.notice.is_empty() {
        return None;
    }
    let (icon, tone) = if state.problem {
        (IconName::TriangleAlert, t.tiles.categories[3])
    } else if state.busy {
        (IconName::LoaderCircle, t.tiles.neutral)
    } else {
        (IconName::Info, t.tiles.categories[1])
    };
    Some(
        h_flex()
            .id("plugin-notice")
            .test_support()
            .aria_label(state.notice.clone())
            .w_full()
            .min_w_0()
            .items_start()
            .gap(t.space.small)
            .p(t.space.small)
            .rounded(t.radius.input)
            .border_l_2()
            .border_color(tone.foreground)
            .bg(tone.background)
            .child(
                Icon::new(icon)
                    .flex_shrink_0()
                    .size(t.layout.icon_small)
                    .text_color(tone.foreground),
            )
            .child(
                design::paragraph(state.notice.clone())
                    .id("plugin-notice-text")
                    .min_w_0()
                    .flex_1()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx),
            ),
    )
}

/// A section title with an optional count and a one-line explanation.
fn heading(title: &str, count: Option<usize>, blurb: &str, cx: &Context<Shell>) -> Div {
    let t = design::tokens(cx);
    v_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.compact)
        .child(
            h_flex()
                .min_w_0()
                .items_center()
                .gap(t.space.small)
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(title.to_owned()),
                )
                .children(count.map(|count| pill(count.to_string(), t.tiles.neutral, cx))),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(blurb.to_owned()),
        )
}

fn pill(text: impl Into<SharedString>, tone: design::TileTone, cx: &Context<Shell>) -> Div {
    let t = design::tokens(cx);
    div()
        .flex_shrink_0()
        .px(t.space.small)
        .rounded(t.radius.pill)
        .bg(tone.background)
        .sovra_text(TextRole::Caption, cx)
        .text_color(tone.foreground)
        .child(text.into())
}

fn card(cx: &Context<Shell>) -> Div {
    let t = design::tokens(cx);
    v_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .p(t.space.medium)
        .sovra_surface(Surface::Glass, cx)
}

/// Where a plugin came from, in words.
fn source(plugin: &InstalledPlugin) -> String {
    plugin.github.as_ref().map_or_else(
        || format!("From a catalog · version {}", plugin.entry.version),
        |pin| {
            if official_plugin(&plugin.entry.id).is_some() {
                format!("Made by Sovra · version {}", pin.short_commit())
            } else {
                // The title is already the repository's name.
                format!("From GitHub · version {}", pin.short_commit())
            }
        },
    )
}

/// The name a plugin goes by: an official plugin's everyday name, else its own title.
fn display_title(plugin: &InstalledPlugin) -> String {
    official_plugin(&plugin.entry.id).map_or_else(
        || plugin.entry.title.clone(),
        |official| official.title.into(),
    )
}

/// The picture on an official plugin's card.
fn official_icon(plugin: &OfficialPlugin) -> IconName {
    match plugin.repository {
        "Sovra-AI/sovra-compute-pool" => IconName::Cpu,
        _ => IconName::Puzzle,
    }
}

/// Plugins Sovra makes and keeps working, each installed and turned on with one click.
fn official_section(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut section = v_flex()
        .id("plugin-official")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .child(heading(
            "Made by Sovra",
            None,
            "Ready-made plugins from the Sovra team. One click installs everything and turns it on.",
            cx,
        ));
    for (index, plugin) in OFFICIAL_PLUGINS.iter().enumerate() {
        section = section.child(official_card(index, plugin, &shell.plugins, cx));
    }
    section
}

fn official_card(
    index: usize,
    plugin: &'static OfficialPlugin,
    browser: &PluginBrowser,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let state = &browser.state;
    let busy = state.busy;
    let id = plugin.id();
    let installed = state
        .library
        .installed
        .iter()
        .find(|installed| installed.entry.id == id);
    let enabled = installed.is_some_and(|installed| installed.enabled);
    let installing = busy && browser.installing.as_deref() == Some(id.as_str());
    let title = plugin.title;

    let status = match (installed.is_some(), enabled) {
        (true, true) => Some(pill("Installed · On", t.tiles.categories[2], cx)),
        (true, false) => Some(pill("Installed · Off", t.tiles.neutral, cx)),
        (false, _) => None,
    };
    let mut does = v_flex().w_full().min_w_0().gap(t.space.compact).child(
        div()
            .sovra_text(TextRole::Caption, cx)
            .text_color(t.text_secondary)
            .child("What it does"),
    );
    for line in plugin.does {
        does = does.child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_start()
                .gap(t.space.small)
                .child(
                    Icon::new(IconName::Check)
                        .flex_shrink_0()
                        .size(t.layout.icon_small)
                        .text_color(t.tiles.categories[2].foreground),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .whitespace_normal()
                        .sovra_text(TextRole::Body, cx)
                        .child(*line),
                ),
        );
    }

    let web = plugin.github().web_url();
    let learn = design::button(("plugin-official-learn", index), "Learn more", cx)
        .ghost()
        .icon(IconName::ExternalLink)
        .accessibility_label(format!("Learn more about {title} on its web page"))
        .on_click(move |_, _, cx| cx.open_url(&web));
    let mut actions = h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .items_center()
        .gap(t.space.small);
    if enabled {
        // Each button starts one of its skills in chat with a ready request, so the
        // person always has an obvious next step once it is set up.
        for (step, action) in plugin.actions.iter().enumerate() {
            let command = plugin.command(action);
            let button = if step == 0 {
                design::primary_button(("plugin-official-try", index), action.label, cx)
                    .icon(IconName::MessageCircle)
            } else {
                design::button(
                    (
                        "plugin-official-action",
                        index * OFFICIAL_ACTION_SLOTS + step,
                    ),
                    action.label,
                    cx,
                )
            };
            actions = actions.child(
                button
                    .accessibility_label(format!("{}: start it in chat", action.label))
                    .disabled(busy)
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.stage_skill(&command, action.prompt, window, cx);
                    })),
            );
        }
    } else {
        let (label, icon) = match (installing, installed.is_some()) {
            (true, _) => ("Installing…", IconName::LoaderCircle),
            (false, true) => ("Turn on", IconName::Play),
            (false, false) => ("Install", IconName::Download),
        };
        let install_id = id.clone();
        actions = actions.child(
            design::primary_button(("plugin-official-install", index), label, cx)
                .icon(icon)
                .accessibility_label(format!("{label} {title}"))
                .disabled(busy)
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.plugin_command(
                        PluginCommand::InstallOfficial {
                            id: install_id.clone(),
                        },
                        cx,
                    );
                })),
        );
    }
    let actions = v_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.compact)
        .pt(t.space.compact)
        .children(enabled.then(|| {
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child("What you can do next")
        }))
        .child(actions.child(learn));

    card(cx)
        .id(("plugin-official-card", index))
        .test_support()
        .aria_label(format!(
            "{title}, made by Sovra, {}. {}",
            match (installed.is_some(), enabled) {
                (true, true) => "installed and on",
                (true, false) => "installed and off",
                (false, _) => "not installed",
            },
            plugin.summary
        ))
        .hover(|style| style.border_color(t.material_border_strong))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(t.space.small)
                .child(tile(official_icon(plugin), t.tiles.categories[4], cx))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .whitespace_normal()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(title),
                )
                .child(pill("Made by Sovra", t.tiles.categories[1], cx))
                .children(status),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .max_w(t.layout.reading_width)
                .whitespace_normal()
                .sovra_text(TextRole::Body, cx)
                .child(plugin.summary),
        )
        .child(does)
        .child(
            div()
                .w_full()
                .min_w_0()
                .max_w(t.layout.reading_width)
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(plugin.needs),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .max_w(t.layout.reading_width)
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(plugin.good_to_know),
        )
        .child(actions)
}

fn matches(entry: &PluginEntry, query: &str) -> bool {
    [&entry.title, &entry.description, &entry.id]
        .iter()
        .any(|text| text.to_lowercase().contains(query))
}

fn installed_section(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let browser = &shell.plugins;
    let state = &browser.state;
    let total = state.library.installed.len();
    let query = browser.search.read(cx).value().trim().to_lowercase();
    let installed: Vec<_> = state
        .library
        .installed
        .iter()
        .filter(|plugin| matches(&plugin.entry, &query))
        .collect();
    let last_page = installed.len().saturating_sub(1) / INSTALLED_PAGE_SIZE;
    let page = browser.installed_page.min(last_page);
    let mut section = v_flex()
        .id("plugin-installed")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .child(heading(
            "Your plugins",
            Some(total),
            "Turn a plugin on to let Sovra use its skills. Its skills then appear on the Skills page.",
            cx,
        ));
    // Worth searching only once the list is longer than a glance.
    if total > 3 || state.catalog.is_some() {
        section = section.child(
            div().w_full().max_w(t.layout.reading_width).child(
                Input::new(&browser.search)
                    .id("plugin-search")
                    .aria_label("Find a plugin on this page by name or words"),
            ),
        );
    }
    if total == 0 {
        return section.child(empty_state(state.busy, cx));
    }
    if installed.is_empty() {
        section = section.child(
            div()
                .id("plugin-no-match")
                .test_support()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child("None of your plugins match. Clear the search box to see them all."),
        );
    }
    for (index, plugin) in installed
        .iter()
        .skip(page * INSTALLED_PAGE_SIZE)
        .take(INSTALLED_PAGE_SIZE)
        .enumerate()
    {
        section = section.child(plugin_card(index, plugin, shell, cx));
    }
    if last_page > 0 {
        section = section.child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("plugin-installed-previous", "Previous plugins", cx)
                        .disabled(page == 0)
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.plugins.installed_page = page.saturating_sub(1);
                            cx.notify();
                        })),
                )
                .child(
                    design::button("plugin-installed-next", "More plugins", cx)
                        .disabled(page == last_page)
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.plugins.installed_page = page + 1;
                            cx.notify();
                        })),
                ),
        );
    }
    section
}

/// First visit: say what a plugin is and offer one safe plugin to try.
fn empty_state(busy: bool, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    card(cx)
        .id("plugin-empty")
        .test_support()
        .child(
            h_flex()
                .min_w_0()
                .items_center()
                .gap(t.space.small)
                .child(tile(IconName::Puzzle, t.tiles.categories[0], cx))
                .child(
                    div()
                        .min_w_0()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("No plugins yet"),
                ),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .max_w(t.layout.reading_width)
                .whitespace_normal()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child("Try the example plugin to see how plugins work. It adds one skill that helps you write a thank-you note. You can remove it again at any time."),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .pt(t.space.compact)
                .child(
                    design::primary_button("plugin-add-example", "Add the example plugin", cx)
                        .icon(IconName::Sparkles)
                        .disabled(busy)
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.plugin_command(
                                PluginCommand::AddFromGitHub {
                                    address: PLUGIN_TEMPLATE.into(),
                                },
                                cx,
                            );
                        })),
                ),
        )
}

fn plugin_card(
    index: usize,
    plugin: &InstalledPlugin,
    shell: &Shell,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let browser = &shell.plugins;
    let t = design::tokens(cx);
    let state = &browser.state;
    let busy = state.busy;
    let title = display_title(plugin);
    let enabled = plugin.enabled;
    let id = plugin.entry.id.clone();
    let confirming = browser.confirm_remove.as_deref() == Some(id.as_str());
    let details = state
        .details
        .as_ref()
        .filter(|(shown, _)| *shown == id)
        .map(|(_, package)| package);
    let update = state
        .update
        .as_ref()
        .filter(|(shown, _)| *shown == id)
        .map(|(_, changes)| changes);
    let (icon, tone) = if plugin.github.is_some() {
        (IconName::Code, t.tiles.categories[1])
    } else {
        (IconName::LibraryBig, t.tiles.categories[5])
    };
    let status = if enabled {
        pill("On", t.tiles.categories[2], cx)
    } else {
        pill("Off", t.tiles.neutral, cx)
    };

    let toggle_id = id.clone();
    let has_code = plugin.entry.api_version == CODE_API_VERSION;
    let approving = browser.confirm_code.as_deref() == Some(id.as_str());
    let toggle_label = if enabled { "Turn off" } else { "Turn on" };
    let toggle = if enabled {
        design::button(("plugin-toggle", index), toggle_label, cx)
    } else {
        design::primary_button(("plugin-toggle", index), toggle_label, cx)
    }
    .small()
    .accessibility_label(format!("{toggle_label} {title}"))
    .disabled(busy || approving)
    .on_click(cx.listener(move |shell, _, _, cx| {
        if has_code && !enabled {
            // Show what the code can reach first; `plugin_command` clears the
            // question, so it is asked after the details are requested.
            shell.plugin_command(
                PluginCommand::Inspect {
                    id: toggle_id.clone(),
                },
                cx,
            );
            shell.plugins.confirm_code = Some(toggle_id.clone());
            cx.notify();
        } else {
            shell.plugin_command(
                PluginCommand::SetEnabled {
                    id: toggle_id.clone(),
                    enabled: !enabled,
                },
                cx,
            );
        }
    }));

    let detail_id = id.clone();
    let open = details.is_some();
    let details_button = design::button(
        ("plugin-details", index),
        if open {
            "Hide what's included"
        } else {
            "What's included"
        },
        cx,
    )
    .small()
    .icon(if open {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    })
    .disabled(busy)
    .accessibility_label(if open {
        format!("Hide what's included in {title}")
    } else {
        format!("What's included in {title}")
    })
    .on_click(cx.listener(move |shell, _, _, cx| {
        if open {
            shell.plugins.state.details = None;
            cx.notify();
        } else {
            shell.plugin_command(
                PluginCommand::Inspect {
                    id: detail_id.clone(),
                },
                cx,
            );
        }
    }));

    let remove_id = id.clone();
    let remove = design::button(
        ("plugin-remove", index),
        if confirming {
            "Click again to remove"
        } else {
            "Remove"
        },
        cx,
    )
    .small()
    .ghost()
    .icon(IconName::Trash)
    .disabled(busy)
    .accessibility_label(if confirming {
        format!("Confirm removing {title}")
    } else {
        format!("Remove {title}")
    })
    .on_click(cx.listener(move |shell, _, _, cx| {
        if shell.plugins.confirm_remove.as_ref() == Some(&remove_id) {
            shell.plugin_command(
                PluginCommand::Remove {
                    id: remove_id.clone(),
                },
                cx,
            );
        } else {
            if !open {
                shell.plugin_command(
                    PluginCommand::Inspect {
                        id: remove_id.clone(),
                    },
                    cx,
                );
            }
            shell.plugins.confirm_remove = Some(remove_id.clone());
            cx.notify();
        }
    }));

    let mut actions = h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .items_center()
        .gap(t.space.small)
        .pt(t.space.compact)
        .child(toggle)
        .children((!confirming).then_some(details_button));
    if plugin.github.is_some() {
        let update_id = id.clone();
        actions = actions.child(
            design::button(("plugin-check-update", index), "Check for update", cx)
                .small()
                .disabled(busy)
                .accessibility_label(format!("Check for an update to {title}"))
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.plugin_command(
                        PluginCommand::CheckForUpdate {
                            id: update_id.clone(),
                        },
                        cx,
                    );
                })),
        );
    }
    actions = actions.child(remove);

    let mut card = card(cx)
        .id(("plugin-card", index))
        .test_support()
        .aria_label(format!(
            "{title}, {}. {}",
            if enabled { "on" } else { "off" },
            source(plugin)
        ))
        .hover(|style| style.border_color(t.material_border_strong))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap(t.space.small)
                .child(tile(icon, tone, cx))
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .min_w_0()
                                .whitespace_normal()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(title.clone()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .whitespace_normal()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(source(plugin)),
                        ),
                )
                .children(has_code.then(|| pill("Tools", t.tiles.categories[4], cx)))
                .child(status),
        );
    if !plugin.entry.description.trim().is_empty() {
        card = card.child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(plugin.entry.description.clone()),
        );
    }
    card = card.child(actions);
    if confirming {
        card = card.child(
            div()
                .id(("plugin-remove-warning", index))
                .test_support()
                .w_full()
                .min_w_0()
                .p(t.space.small)
                .rounded(t.radius.input)
                .bg(t.tiles.categories[3].background)
                .text_color(t.tiles.categories[3].foreground)
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .child(
                    "Removing deletes the plugin and its skills on this computer. Review any changes you made with it before you confirm.",
                ),
        ).child(cleanup::review(index, plugin, details, shell, cx));
    } else if approving {
        card = card.child(approval(
            index,
            &id,
            &plugin.entry.sha256,
            &title,
            details,
            busy,
            cx,
        ));
    } else if let Some(package) = details {
        card = card.child(included(index, package, enabled, busy, cx));
    }
    if let Some(changes) = update {
        card = card.child(update_review(&id, &title, changes, enabled, busy, cx));
    }
    card
}

/// The question before a plugin's code may run: what it adds and what it can reach.
fn approval(
    index: usize,
    id: &str,
    // A library reload clears shown details, so this is the package on screen.
    package_sha256: &str,
    title: &str,
    package: Option<&PluginPackage>,
    busy: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let code = package.and_then(|package| package.code.as_ref());
    // Read in full by screen readers: the question, what it can reach and its tools.
    let spoken = code.map_or_else(
        || format!("Allow {title} to run its own code? Checking what it can do."),
        |code| {
            format!(
                "Allow {title} to run its own code? {} Tools: {}.",
                code.access_summary(),
                code.tools
                    .iter()
                    .map(|tool| tool.title.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
    );
    let mut panel = v_flex()
        .id(("plugin-code-approval", index))
        .test_support()
        .aria_label(spoken)
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .p(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Body, cx)
                .child(format!("Allow {title} to run its own code?")),
        );
    match code {
        Some(code) => {
            panel = panel.child(access_line(code, cx));
            for tool in &code.tools {
                panel = panel.child(tool_row(&tool.title, &tool.description, cx));
            }
        }
        None => {
            panel = panel.child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child("Checking what this plugin can do…"),
            );
        }
    }
    let allow_id = id.to_owned();
    let allow_sha256 = package_sha256.to_owned();
    panel.child(
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .gap(t.space.small)
            .pt(t.space.compact)
            .child(
                design::primary_button(("plugin-code-allow", index), "Allow and turn on", cx)
                    .small()
                    .icon(IconName::Lock)
                    // Only once the person can read what they are allowing.
                    .disabled(busy || code.is_none())
                    .accessibility_label(format!("Allow {title} to run its code and turn it on"))
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.plugin_command(
                            PluginCommand::AllowCode {
                                id: allow_id.clone(),
                                package_sha256: allow_sha256.clone(),
                            },
                            cx,
                        );
                    })),
            )
            .child(
                design::button(("plugin-code-cancel", index), "Cancel", cx)
                    .small()
                    .accessibility_label(format!("Keep {title} off"))
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.plugins.confirm_code = None;
                        shell.plugins.state.details = None;
                        cx.notify();
                    })),
            ),
    )
}

/// One line saying what the plugin's code can reach.
fn access_line(code: &sovra_core::plugins::PluginCode, cx: &Context<Shell>) -> Div {
    let t = design::tokens(cx);
    h_flex()
        .w_full()
        .min_w_0()
        .items_start()
        .gap(t.space.small)
        .child(
            Icon::new(if code.network.is_empty() {
                IconName::Lock
            } else {
                IconName::Globe
            })
            .flex_shrink_0()
            .size(t.layout.icon_small)
            .text_color(t.text_secondary),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .child(code.access_summary()),
        )
}

fn tool_row(title: &str, description: &str, cx: &Context<Shell>) -> Div {
    let t = design::tokens(cx);
    h_flex()
        .w_full()
        .min_w_0()
        .items_start()
        .gap(t.space.small)
        .child(
            Icon::new(IconName::Wrench)
                .flex_shrink_0()
                .size(t.layout.icon_small)
                .text_color(t.text_secondary),
        )
        .child(
            v_flex()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Body, cx)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(description.to_owned()),
                ),
        )
}

/// The skills and suggested connections inside one plugin, shown in its card.
fn included(
    index: usize,
    package: &PluginPackage,
    enabled: bool,
    busy: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut panel = v_flex()
        .id(("plugin-included", index))
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .p(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .children((!package.skills.is_empty()).then(|| {
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(match package.skills.len() {
                    1 => "1 skill".to_owned(),
                    count => format!("{count} skills"),
                })
        }));
    for skill in &package.skills {
        let files = match skill.resources.len() {
            0 => String::new(),
            1 => " Comes with 1 file it can read.".to_owned(),
            count => format!(" Comes with {count} files it can read."),
        };
        panel = panel.child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_start()
                .gap(t.space.small)
                .child(
                    Icon::new(IconName::BookOpen)
                        .flex_shrink_0()
                        .size(t.layout.icon_small)
                        .text_color(t.text_secondary),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .min_w_0()
                                .whitespace_normal()
                                .sovra_text(TextRole::Body, cx)
                                .child(skill.title.clone()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .whitespace_normal()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(format!("{}{files}", skill.description)),
                        ),
                ),
        );
    }
    if let Some(code) = &package.code {
        panel = panel
            .child(
                div()
                    .pt(t.space.compact)
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child(match code.tools.len() {
                        1 => "1 tool, from the plugin's own code".to_owned(),
                        count => format!("{count} tools, from the plugin's own code"),
                    }),
            )
            .child(access_line(code, cx));
        for tool in &code.tools {
            panel = panel.child(tool_row(&tool.title, &tool.description, cx));
        }
    }
    if !package.services.is_empty() {
        panel = panel.child(
            div()
                .pt(t.space.compact)
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child("Suggested connections"),
        );
    }
    for (index, service) in package.services.iter().enumerate() {
        let name = service.name.clone();
        let address = service.address.clone();
        panel = panel.child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(t.space.small)
                .child(
                    Icon::new(IconName::Plug)
                        .flex_shrink_0()
                        .size(t.layout.icon_small)
                        .text_color(t.text_secondary),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .flex_basis(relative(0.5))
                        .child(
                            div()
                                .min_w_0()
                                .whitespace_normal()
                                .sovra_text(TextRole::Body, cx)
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .whitespace_normal()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(address.clone()),
                        ),
                )
                .child(
                    design::button(("plugin-service", index), "Set up connection", cx)
                        .small()
                        .disabled(!enabled || busy)
                        .accessibility_label(format!("Set up {}", service.name))
                        .on_click(cx.listener(move |shell, _, window, cx| {
                            shell.present_connections(
                                Some(sovra_core::CatalogueKind::WebAddress),
                                false,
                                window,
                                cx,
                            );
                            shell.models().update(cx, |models, cx| {
                                models.service_name.update(cx, |input, cx| {
                                    input.set_value(name.clone(), window, cx);
                                });
                                models.service_address.update(cx, |input, cx| {
                                    input.set_value(address.clone(), window, cx);
                                });
                            });
                        })),
                ),
        );
    }
    let footer = match (enabled, package.services.is_empty()) {
        (false, true) => Some("Turn the plugin on to use it."),
        (false, false) => Some(
            "Turn the plugin on to use it and set up its connections. Connections are set up separately, in Connections.",
        ),
        (true, false) => Some(
            "Connections are set up separately. To stop a connected service, turn it off in Connections.",
        ),
        (true, true) => None,
    };
    panel.children(footer.map(|text| {
        div()
            .w_full()
            .min_w_0()
            .whitespace_normal()
            .sovra_text(TextRole::Caption, cx)
            .text_color(t.text_secondary)
            .child(text)
    }))
}

/// Shows what an update would change, so nothing new reaches a helper unseen.
fn update_review(
    id: &str,
    title: &str,
    changes: &PluginChanges,
    enabled: bool,
    busy: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut lines = Vec::new();
    for (label, names) in [
        ("New skill", &changes.added),
        ("Changed skill", &changes.changed),
        ("Removed skill", &changes.removed),
        ("New suggested connection", &changes.connections_added),
        ("Removed suggested connection", &changes.connections_removed),
        ("New tool", &changes.code.tools_added),
        ("Removed tool", &changes.code.tools_removed),
        ("Can now contact", &changes.code.network_added),
        ("No longer contacts", &changes.code.network_removed),
    ] {
        lines.extend(names.iter().map(|name| format!("{label}: {name}")));
    }
    let code = &changes.code;
    if code.code_added {
        lines.push("It now brings its own code, which runs in a sealed box.".into());
    } else if code.code_removed {
        lines.push("It no longer brings any code.".into());
    } else if code.code_changed {
        lines.push("Its code has changed.".into());
    }
    if enabled && code.needs_approval() {
        lines.push(
            "Updating turns this plugin off, because its code would be able to do something new. Turn it on again to allow that."
                .into(),
        );
    }
    if lines.is_empty() {
        lines.push("Only files Sovra doesn't use have changed.".into());
    }
    lines.extend(
        changes
            .left_out
            .iter()
            .map(|reason| format!("Can't be added: {reason}")),
    );
    let mut review = v_flex()
        .id("plugin-update-review")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .p(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .child("A newer version is ready"),
        );
    for line in lines {
        review = review.child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .child(line),
        );
    }
    let apply_id = id.to_owned();
    review.child(
        h_flex()
            .flex_wrap()
            .gap(t.space.small)
            .pt(t.space.compact)
            .child(
                design::primary_button("plugin-apply-update", "Update", cx)
                    .small()
                    .disabled(busy)
                    .accessibility_label(format!("Update {title}"))
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.plugin_command(
                            PluginCommand::ApplyUpdate {
                                id: apply_id.clone(),
                            },
                            cx,
                        );
                    })),
            )
            .child(
                design::button("plugin-skip-update", "Not now", cx)
                    .small()
                    .disabled(busy)
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.plugins.state.update = None;
                        cx.notify();
                    })),
            ),
    )
}

/// The two ways to add a plugin, plus where to start making one.
fn add_section(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let browser = &shell.plugins;
    let busy = browser.state.busy;
    let github = card(cx)
        .id("plugin-add-github")
        .test_support()
        .child(
            h_flex()
                .min_w_0()
                .items_center()
                .gap(t.space.small)
                .child(tile(IconName::Code, t.tiles.categories[1], cx))
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("From GitHub"),
                ),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child("Paste a repository's address. Sovra adds its skills and suggested connections exactly as they are now."),
        )
        .child(
            Input::new(&browser.github)
                .id("plugin-github-address")
                .aria_label("GitHub repository address"),
        )
        .child(
            design::primary_button("plugin-github-add", "Add from GitHub", cx)
                .self_start()
                .icon(IconName::Plus)
                .disabled(busy)
                .on_click(cx.listener(|shell, _, _, cx| {
                    let address = shell.plugins.github.read(cx).value().trim().to_owned();
                    shell.plugin_command(PluginCommand::AddFromGitHub { address }, cx);
                })),
        );
    let catalog = card(cx)
        .id("plugin-add-catalog")
        .test_support()
        .child(
            h_flex()
                .min_w_0()
                .items_center()
                .gap(t.space.small)
                .child(tile(IconName::LibraryBig, t.tiles.categories[5], cx))
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("From a catalog"),
                ),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child("A catalog is a list of plugins a publisher shares on their website. Paste its address to see what it offers."),
        )
        .child(
            Input::new(&browser.url)
                .id("plugin-catalog-url")
                .aria_label("Plugin catalog address"),
        )
        .child(
            design::button("plugin-browse", "Load catalog", cx)
                .self_start()
                .disabled(busy)
                .on_click(cx.listener(|shell, _, _, cx| {
                    let browser = &shell.plugins;
                    let typed = browser.url.read(cx).value();
                    let url = if typed.trim().is_empty() {
                        browser.state.library.catalog_url.clone().unwrap_or_default()
                    } else {
                        typed.trim().to_owned()
                    };
                    shell.plugin_command(PluginCommand::Browse { url }, cx);
                })),
        )
        .children(browser.state.library.catalog_url.as_ref().map(|url| {
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(format!(
                    "Last used: {url}. Leave the box empty to load it again."
                ))
        }));
    let side_by_side = browser.width_class.fits_detail_pane();
    let ways = if side_by_side {
        h_flex()
            .w_full()
            .min_w_0()
            .items_stretch()
            .gap(t.space.medium)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .flex_basis(relative(0.))
                    .min_w_0()
                    .child(github),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .flex_basis(relative(0.))
                    .min_w_0()
                    .child(catalog),
            )
    } else {
        v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.medium)
            .child(github)
            .child(catalog)
    };
    v_flex()
        .id("plugin-add")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .child(heading(
            "Add a plugin",
            None,
            "Only add plugins from people you trust. Nothing in a plugin is run on your computer.",
            cx,
        ))
        .child(ways)
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(t.space.small)
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(
                            "Want to share your own skills? Start from the free plugin template.",
                        ),
                )
                .child(
                    design::button("plugin-template-link", "Open the plugin template", cx)
                        .small()
                        .ghost()
                        .icon(IconName::ExternalLink)
                        .on_click(|_, _, cx| cx.open_url(PLUGIN_TEMPLATE)),
                ),
        )
}

/// One page of a loaded catalog.
fn catalog_section(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let t = design::tokens(cx);
    let browser = &shell.plugins;
    let state = &browser.state;
    let catalog = state.catalog.as_ref()?;
    let busy = state.busy;
    let query = browser.search.read(cx).value().trim().to_lowercase();
    let entries: Vec<_> = catalog
        .plugins
        .iter()
        .filter(|entry| matches(entry, &query))
        .collect();
    let mut section = v_flex()
        .id("plugin-catalog")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .child(heading(
            "Available from this catalog",
            Some(entries.len()),
            "Adding a plugin downloads it switched off. Turn it on in Your plugins.",
            cx,
        ));
    for (index, entry) in entries.into_iter().enumerate() {
        let installed = state
            .library
            .installed
            .iter()
            .find(|plugin| plugin.entry.id == entry.id);
        let same = installed.is_some_and(|plugin| plugin.entry.sha256 == entry.sha256);
        let compatible = entry.api_version == 1;
        let id = entry.id.clone();
        let label = if !compatible {
            "Needs a newer Sovra"
        } else if same {
            "Added"
        } else if installed.is_some() {
            "Update"
        } else {
            "Add"
        };
        let action = if same || !compatible {
            design::button(("plugin-install", index), label, cx)
        } else {
            design::primary_button(("plugin-install", index), label, cx)
        };
        section = section.child(
            card(cx)
                .id(("plugin-catalog-entry", index))
                .test_support()
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(t.space.small)
                        .child(tile(IconName::Puzzle, t.tiles.categories[0], cx))
                        .child(
                            v_flex()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .min_w_0()
                                        .whitespace_normal()
                                        .sovra_text(TextRole::SectionTitle, cx)
                                        .child(entry.title.clone()),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .sovra_text(TextRole::Caption, cx)
                                        .text_color(t.text_secondary)
                                        .child(format!("Version {}", entry.version)),
                                ),
                        )
                        .children(same.then(|| pill("Added", t.tiles.categories[2], cx))),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Body, cx)
                        .text_color(t.text_secondary)
                        .child(entry.description.clone()),
                )
                .child(
                    action
                        .small()
                        .self_start()
                        .icon(IconName::Download)
                        .disabled(busy || same || !compatible)
                        .accessibility_label(format!("{label} {}", entry.title))
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.plugin_command(PluginCommand::Install { id: id.clone() }, cx);
                        })),
                ),
        );
    }
    section = section.child(
        h_flex()
            .flex_wrap()
            .gap(t.space.small)
            .child(
                design::button("plugin-catalog-previous", "Previous page", cx)
                    .disabled(busy || catalog.previous.is_none())
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.plugin_command(PluginCommand::PreviousPage, cx);
                    })),
            )
            .child(
                design::button("plugin-catalog-next", "Next page", cx)
                    .disabled(busy || catalog.next.is_none())
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.plugin_command(PluginCommand::NextPage, cx);
                    })),
            ),
    );
    Some(section.into_any_element())
}
