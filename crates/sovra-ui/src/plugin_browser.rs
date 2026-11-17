//! Paged remote discovery and local plugin controls on the Skills page.

use gpui_kit::component::{
    Disableable as _, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
};
use sovra_core::{
    Command,
    plugins::{PluginCommand, PluginEntry, PluginEvent, PluginState},
};

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
    shell::Shell,
};

const INSTALLED_PAGE_SIZE: usize = 20;

pub(crate) struct PluginBrowser {
    pub(crate) open: bool,
    pub(crate) state: PluginState,
    url: Entity<InputState>,
    search: Entity<InputState>,
    installed_page: usize,
    _search: Subscription,
}

impl PluginBrowser {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter this page and your installed plugins")
        });
        let subscription = cx.subscribe(&search, |shell, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                shell.skill_library.plugins.installed_page = 0;
                cx.notify();
            }
        });
        Self {
            open: false,
            state: PluginState::default(),
            url: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("https://publisher.github.io/plugins/catalog.json")
            }),
            search,
            installed_page: 0,
            _search: subscription,
        }
    }
}

impl Shell {
    pub(crate) fn plugin_command(&mut self, command: PluginCommand, cx: &mut Context<Self>) {
        if self.skill_library.plugins.state.busy {
            return;
        }
        if matches!(&command, PluginCommand::Browse { .. }) {
            self.skill_library.plugins.state.catalog = None;
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
        self.skill_library.plugins.state.apply(&event);
        cx.notify();
    }
}

pub(crate) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let browser = &shell.skill_library.plugins;
    let state = &browser.state;
    let busy = state.busy;
    let query = browser.search.read(cx).value().trim().to_lowercase();
    let matches = |entry: &PluginEntry| {
        [&entry.title, &entry.description, &entry.id]
            .iter()
            .any(|text| text.to_lowercase().contains(&query))
    };
    let installed: Vec<_> = state
        .library
        .installed
        .iter()
        .filter(|plugin| matches(&plugin.entry))
        .collect();
    let last_page = installed.len().saturating_sub(1) / INSTALLED_PAGE_SIZE;
    let page = browser.installed_page.min(last_page);
    let mut panel = v_flex().id("plugin-browser").test_support().w_full().min_w_0()
        .gap(t.space.medium).p(t.space.medium).sovra_surface(Surface::Matte, cx)
        .child(design::page_header("Plugins", "Download extra skills from a catalog you trust. Installed plugins work offline; turn them on when you want Sovra to use their instructions.", cx))
        .child(div().w_full().max_w(t.layout.reading_width).child(Input::new(&browser.url).id("plugin-catalog-url").aria_label("Plugin catalog address")))
        .child(design::button("plugin-browse", "Load catalog", cx).disabled(busy).on_click(cx.listener(|shell, _, _, cx| {
            let browser = &shell.skill_library.plugins;
            let typed = browser.url.read(cx).value();
            let url = if typed.trim().is_empty() { browser.state.library.catalog_url.clone().unwrap_or_default() } else { typed.trim().to_owned() };
            shell.plugin_command(PluginCommand::Browse { url }, cx);
        })))
        .children(state.library.catalog_url.as_ref().map(|url| div().min_w_0().sovra_text(TextRole::Caption, cx).child(format!("Saved catalog: {url}"))))
        .child(div().w_full().max_w(t.layout.reading_width).child(Input::new(&browser.search).id("plugin-search").aria_label("Filter plugins on this page and installed plugins")))
        .child(div().id("plugin-notice").test_support().aria_label(state.notice.clone()).sovra_text(TextRole::Caption, cx).child(state.notice.clone()))
        .child(div().sovra_text(TextRole::SectionTitle, cx).child(format!("Installed ({})", state.library.installed.len())));
    if installed.is_empty() {
        panel = panel.child(
            div()
                .sovra_text(TextRole::Body, cx)
                .child("No installed plugins match. Load a catalog to find one."),
        );
    }
    for (index, plugin) in installed
        .iter()
        .skip(page * INSTALLED_PAGE_SIZE)
        .take(INSTALLED_PAGE_SIZE)
        .enumerate()
    {
        let id = plugin.entry.id.clone();
        let detail_id = id.clone();
        let enabled = plugin.enabled;
        panel = panel.child(
            v_flex()
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .child(div().sovra_text(TextRole::Body, cx).child(format!(
                    "{} · {} · {}",
                    plugin.entry.title,
                    plugin.entry.version,
                    if enabled { "On" } else { "Off" }
                )))
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .child(plugin.entry.description.clone()),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::button(
                                ("plugin-toggle", index),
                                if enabled { "Turn off" } else { "Turn on" },
                                cx,
                            )
                            .accessibility_label(format!(
                                "{} {}",
                                if enabled { "Turn off" } else { "Turn on" },
                                plugin.entry.title
                            ))
                            .disabled(busy)
                            .on_click(cx.listener(
                                move |shell, _, _, cx| {
                                    shell.plugin_command(
                                        PluginCommand::SetEnabled {
                                            id: id.clone(),
                                            enabled: !enabled,
                                        },
                                        cx,
                                    );
                                },
                            )),
                        )
                        .child(
                            design::button(("plugin-details", index), "What's included", cx)
                                .disabled(busy)
                                .accessibility_label(format!(
                                    "What's included in {}",
                                    plugin.entry.title
                                ))
                                .on_click(cx.listener(move |shell, _, _, cx| {
                                    shell.plugin_command(
                                        PluginCommand::Inspect {
                                            id: detail_id.clone(),
                                        },
                                        cx,
                                    );
                                })),
                        ),
                ),
        );
    }
    if last_page > 0 {
        panel = panel.child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("plugin-installed-previous", "Previous installed", cx)
                        .disabled(page == 0)
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.skill_library.plugins.installed_page = page.saturating_sub(1);
                            cx.notify();
                        })),
                )
                .child(
                    design::button("plugin-installed-next", "More installed", cx)
                        .disabled(page == last_page)
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.skill_library.plugins.installed_page = page + 1;
                            cx.notify();
                        })),
                ),
        );
    }
    if let Some((id, package)) = &state.details {
        let enabled = state
            .library
            .installed
            .iter()
            .any(|plugin| &plugin.entry.id == id && plugin.enabled);
        panel = panel.child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("What's included"),
        );
        for skill in &package.skills {
            panel = panel.child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .child(format!("{}: {}", skill.title, skill.description)),
            );
        }
        for (index, service) in package.services.iter().enumerate() {
            let name = service.name.clone();
            let address = service.address.clone();
            panel = panel.child(
                v_flex()
                    .min_w_0()
                    .gap(t.space.small)
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .child(format!("{name}: {address}")),
                    )
                    .child(
                        design::button(("plugin-service", index), "Set up connection", cx)
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
        panel = panel.child(div().sovra_text(TextRole::Caption, cx).child("Connections are set up separately. To stop a connected service, turn it off in Connections."));
    }
    if let Some(catalog) = &state.catalog {
        panel = panel.child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("Available on this catalog page"),
        );
        for (index, entry) in catalog
            .plugins
            .iter()
            .filter(|entry| matches(entry))
            .enumerate()
        {
            let installed = state
                .library
                .installed
                .iter()
                .find(|plugin| plugin.entry.id == entry.id);
            let same = installed.is_some_and(|plugin| plugin.entry.sha256 == entry.sha256);
            let compatible = entry.api_version == 1;
            let id = entry.id.clone();
            let label = if !compatible {
                "Unavailable"
            } else if same {
                "Installed"
            } else if installed.is_some() {
                "Update"
            } else {
                "Download"
            };
            panel = panel.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .child(
                        div()
                            .sovra_text(TextRole::Body, cx)
                            .child(format!("{} · {}", entry.title, entry.version)),
                    )
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .child(entry.description.clone()),
                    )
                    .child(
                        design::button(("plugin-install", index), label, cx)
                            .disabled(busy || same || !compatible)
                            .accessibility_label(format!("{label} {}", entry.title))
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                shell.plugin_command(PluginCommand::Install { id: id.clone() }, cx);
                            })),
                    ),
            );
        }
        panel = panel.child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("plugin-catalog-previous", "Previous catalog page", cx)
                        .disabled(busy || catalog.previous.is_none())
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.plugin_command(PluginCommand::PreviousPage, cx);
                        })),
                )
                .child(
                    design::button("plugin-catalog-next", "Next catalog page", cx)
                        .disabled(busy || catalog.next.is_none())
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.plugin_command(PluginCommand::NextPage, cx);
                        })),
                ),
        );
    }
    panel
}
