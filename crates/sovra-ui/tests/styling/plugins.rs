//! Catalog controls use the native command boundary at narrow and enlarged sizes.
use super::*;
use sovra_core::{
    Command, Event,
    plugins::{
        InstalledPlugin, PluginCatalog, PluginCommand, PluginEntry, PluginEvent, PluginLibrary,
    },
};

fn reveal(
    window: &mut gpui_kit::Window,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut gpui_kit::App,
) {
    reveal_in(window, "sovra-plugins-view", id, cx);
}

fn reveal_in(
    window: &mut gpui_kit::Window,
    pane_id: &'static str,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut gpui_kit::App,
) {
    let id = id.into();
    for _ in 0..60 {
        window.render_frame(cx);
        let pane = window.find(pane_id).bounds();
        let target = window.find(id.clone()).bounds();
        assert!(
            target.left() >= pane.left() && target.right() <= pane.right(),
            "plugin control overflows horizontally"
        );
        if target.top() >= pane.top() && target.bottom() <= pane.bottom() {
            return;
        }
        let delta = if target.top() < pane.top() {
            pane.top() - target.top()
        } else {
            pane.bottom() - target.bottom()
        };
        window.scroll(
            pane_id,
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), delta)),
            cx,
        );
    }
    panic!("plugin control must be reachable");
}

#[gpui_kit::test]
fn catalog_download_and_activation_controls_fit_supported_sizes(cx: &mut TestAppContext) {
    for (width, height, scale) in [
        (640., 420., 100),
        (2560., 1440., 100),
        (640., 420., 200),
        (2560., 1440., 200),
    ] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(scale),
                    cx,
                );
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| Shell::new(0, window, cx));
                shell.update(cx, |shell, cx| {
                    shell.select_tab(sovra_core::NavTab::Plugins, cx);
                });
                Root::new(shell, window, cx)
            });
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "plugin-catalog-url", cx);
                window.click("plugin-catalog-url", cx);
                window.input("https://example.github.io/plugins/catalog.json", cx);
                reveal(window, "plugin-browse", cx);
                window.click("plugin-browse", cx);
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Plugins(PluginCommand::Browse { url }) if url == "https://example.github.io/plugins/catalog.json")));
            let entry = PluginEntry::for_package(
                "https://example.github.io/plugins/package.json".into(),
                include_bytes!("../../../../examples/plugins/packages/weekly-planning-1.0.0.json"),
            )
            .unwrap();
            events
                .send_blocking(Event::Plugins(PluginEvent::Catalog {
                    url: "https://example.github.io/plugins/catalog.json".into(),
                    catalog: PluginCatalog {
                        schema_version: 1,
                        plugins: vec![entry.clone()],
                        next: Some("https://example.github.io/plugins/page2.json".into()),
                        previous: None,
                    },
                }))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("plugin-install", 0usize), cx);
                window.click(("plugin-install", 0usize), cx);
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Plugins(PluginCommand::Install { id }) if id == entry.id)));
            events
                .send_blocking(Event::Plugins(PluginEvent::Loaded(PluginLibrary {
                    catalog_url: Some("https://example.github.io/plugins/catalog.json".into()),
                    installed: vec![InstalledPlugin {
                        entry: entry.clone(),
                        catalog_url: "https://example.github.io/plugins/catalog.json".into(),
                        enabled: false,
                        github: None,
                    }],
                })))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("plugin-toggle", 0usize), cx);
                window.click(("plugin-toggle", 0usize), cx);
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Plugins(PluginCommand::SetEnabled { id, enabled: true }) if id == entry.id)));
            cx.update_window(window.into(), |_, window, _| window.remove_window())
                .unwrap();
        }
    }
}

#[gpui_kit::test]
fn plugin_services_use_the_existing_connection_form_without_auto_connecting(
    cx: &mut TestAppContext,
) {
    use sovra_core::plugins::{PluginPackage, PluginService};
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        shell.update(cx, |shell, cx| {
            shell.select_tab(sovra_core::NavTab::Plugins, cx);
        });
        Root::new(shell, window, cx)
    });
    let mut package: PluginPackage = serde_json::from_slice(include_bytes!(
        "../../../../examples/plugins/packages/weekly-planning-1.0.0.json"
    ))
    .unwrap();
    package.services.push(PluginService {
        name: "Planning service".into(),
        address: "https://planning.example/mcp".into(),
    });
    let entry = PluginEntry::for_package(
        "https://example.com/package.json".into(),
        &serde_json::to_vec(&package).unwrap(),
    )
    .unwrap();
    for enabled in [false, true] {
        events
            .send_blocking(Event::Plugins(PluginEvent::Loaded(PluginLibrary {
                catalog_url: None,
                installed: vec![InstalledPlugin {
                    entry: entry.clone(),
                    catalog_url: "https://example.com/catalog.json".into(),
                    enabled,
                    github: None,
                }],
            })))
            .unwrap();
        events
            .send_blocking(Event::Plugins(PluginEvent::Details {
                id: entry.id.clone(),
                package: package.clone(),
            }))
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            reveal(window, ("plugin-service", 0usize), cx);
            if enabled {
                assert_ne!(
                    window.find(("plugin-service", 0usize)).disabled(),
                    Some(true)
                );
            } else {
                assert_eq!(
                    window.find(("plugin-service", 0usize)).disabled(),
                    Some(true)
                );
            }
            if enabled {
                window.click(("plugin-service", 0usize), cx);
            }
        })
        .unwrap();
    }
    assert!(
        !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Mcp(sovra_core::mcp::McpCommand::Connect { .. })
        ))
    );
    cx.update_window(window.into(), |_, window, cx| {
        reveal_in(window, "connections-scroll", "mcp-connect", cx);
        assert_ne!(window.find("mcp-connect").disabled(), Some(true));
        window.click("mcp-connect", cx);
    })
    .unwrap();
    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Mcp(sovra_core::mcp::McpCommand::Connect { name, address, secret: None }) if name == "Planning service" && address == "https://planning.example/mcp")));
}

fn github_plugin() -> InstalledPlugin {
    let snapshot = sovra_core::plugins::RepositorySnapshot {
        repository: sovra_core::plugins::GitHubRepository::parse("owner/writing-skills")
            .expect("valid repository"),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        description: Some("Helpers for writing letters and notes.".into()),
        skills: vec![
            sovra_core::plugins::repository_skill(
                "letters".into(),
                "letters",
                "Write a kind letter.",
                "Ask who the letter is for.".into(),
            )
            .expect("valid skill"),
        ],
        manifests: Vec::new(),
        module: None,
        left_out: Vec::new(),
    };
    let (entry, _, pin) = snapshot.package().expect("package");
    InstalledPlugin {
        entry,
        catalog_url: snapshot.repository.web_url(),
        enabled: true,
        github: Some(pin),
    }
}

#[gpui_kit::test]
fn github_add_check_and_update_controls_fit_supported_sizes(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(scale),
                    cx,
                );
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| Shell::new(0, window, cx));
                shell.update(cx, |shell, cx| {
                    shell.select_tab(sovra_core::NavTab::Plugins, cx);
                });
                Root::new(shell, window, cx)
            });
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "plugin-github-address", cx);
                window.click("plugin-github-address", cx);
                window.input("https://github.com/owner/writing-skills", cx);
                reveal(window, "plugin-github-add", cx);
                let address = window.find("plugin-github-address").bounds();
                let add = window.find("plugin-github-add").bounds();
                assert!(
                    add.size.width <= address.size.width,
                    "Add from GitHub is wider than its address field"
                );
                window.click("plugin-github-add", cx);
                reveal(window, "plugin-catalog-url", cx);
                reveal(window, "plugin-browse", cx);
                let catalog = window.find("plugin-catalog-url").bounds();
                let load = window.find("plugin-browse").bounds();
                assert!(
                    load.size.width <= catalog.size.width,
                    "Load catalog is wider than its address field"
                );
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Plugins(PluginCommand::AddFromGitHub { address }) if address == "https://github.com/owner/writing-skills")));
            let plugin = github_plugin();
            let id = plugin.entry.id.clone();
            events
                .send_blocking(Event::Plugins(PluginEvent::Loaded(PluginLibrary {
                    catalog_url: None,
                    installed: vec![plugin],
                })))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("plugin-check-update", 0usize), cx);
                window.click(("plugin-check-update", 0usize), cx);
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(&command, Command::Plugins(PluginCommand::CheckForUpdate { id: checked }) if *checked == id)));
            events
                .send_blocking(Event::Plugins(PluginEvent::UpdateReady {
                    id: id.clone(),
                    changes: sovra_core::plugins::PluginChanges {
                        added: vec![
                            "A much longer skill title that has to wrap on a narrow window".into(),
                        ],
                        changed: vec!["letters".into()],
                        ..Default::default()
                    },
                }))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                // The review can be taller than a small window; its button must still be reachable
                // and nothing in it may overflow sideways.
                reveal(window, "plugin-apply-update", cx);
                window.click("plugin-apply-update", cx);
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(&command, Command::Plugins(PluginCommand::ApplyUpdate { id: applied }) if *applied == id)));
        }
    }
}

#[gpui_kit::test]
fn removing_a_plugin_asks_twice_and_fits_supported_sizes(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(scale),
                    cx,
                );
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| Shell::new(0, window, cx));
                shell.update(cx, |shell, cx| {
                    shell.select_tab(sovra_core::NavTab::Plugins, cx);
                });
                Root::new(shell, window, cx)
            });
            let plugin = github_plugin();
            let id = plugin.entry.id.clone();
            events
                .send_blocking(Event::Plugins(PluginEvent::Loaded(PluginLibrary {
                    catalog_url: None,
                    installed: vec![plugin],
                })))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            while commands.try_recv().is_ok() {}
            cx.update_window(window.into(), |_, window, cx| {
                assert!(window.try_find(("plugin-remove-warning", 0usize)).is_none());
                reveal(window, ("plugin-remove", 0usize), cx);
                window.click(("plugin-remove", 0usize), cx);
                window.render_frame(cx);
                // The first press only explains what removing does.
                reveal(window, ("plugin-remove-warning", 0usize), cx);
            })
            .unwrap();
            assert!(
                commands.try_recv().is_err(),
                "one press must not remove anything"
            );
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("plugin-remove", 0usize), cx);
                window.click(("plugin-remove", 0usize), cx);
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(&command, Command::Plugins(PluginCommand::Remove { id: removed }) if *removed == id)));
            events
                .send_blocking(Event::Plugins(
                    PluginEvent::Loaded(PluginLibrary::default()),
                ))
                .unwrap();
            events
                .send_blocking(Event::Plugins(PluginEvent::Removed {
                    id: id.clone(),
                    title: "owner/writing-skills".into(),
                }))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find(("plugin-remove", 0usize)).is_none());
                assert!(window.try_find(("plugin-remove-warning", 0usize)).is_none());
                reveal(window, "plugin-notice", cx);
                window.remove_window();
            })
            .unwrap();
        }
    }
}

fn plugins_window(
    cx: &mut TestAppContext,
    (width, height, scale): (f32, f32, u16),
    mode: ThemeMode,
) -> (
    gpui_kit::WindowHandle<Root>,
    async_channel::Receiver<Command>,
    async_channel::Sender<Event>,
) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| {
        sovra_ui::init(client, cx);
        Theme::change(mode, None, cx);
        design::set_interface_scale(sovra_core::models::InterfaceScale::from_percent(scale), cx);
    });
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        shell.update(cx, |shell, cx| {
            shell.select_tab(sovra_core::NavTab::Plugins, cx);
        });
        Root::new(shell, window, cx)
    });
    (window, commands, events)
}

const SIZES: [(f32, f32, u16); 3] = [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)];

#[gpui_kit::test]
fn a_first_visit_offers_the_example_plugin_at_supported_sizes(cx: &mut TestAppContext) {
    for sizing in SIZES {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (window, commands, _events) = plugins_window(cx, sizing, mode);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "plugin-add-example", cx);
                window.click("plugin-add-example", cx);
                reveal(window, "plugin-template-link", cx);
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(&command, Command::Plugins(PluginCommand::AddFromGitHub { address }) if address == sovra_core::plugins::PLUGIN_TEMPLATE)));
            cx.update_window(window.into(), |_, window, _| window.remove_window())
                .unwrap();
        }
    }
}

#[gpui_kit::test]
fn whats_included_opens_inside_its_plugin_card_and_closes_without_a_command(
    cx: &mut TestAppContext,
) {
    use sovra_core::plugins::{PluginPackage, PluginService};
    for sizing in SIZES {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (window, commands, events) = plugins_window(cx, sizing, mode);
            let plugin = github_plugin();
            let id = plugin.entry.id.clone();
            let mut package: PluginPackage = serde_json::from_slice(include_bytes!(
                "../../../../examples/plugins/packages/weekly-planning-1.0.0.json"
            ))
            .unwrap();
            package.services.push(PluginService {
                name: "A planning service with a long name that has to wrap".into(),
                address: "https://planning.example/a/very/long/address/for/the/mcp/server".into(),
            });
            events
                .send_blocking(Event::Plugins(PluginEvent::Loaded(PluginLibrary {
                    catalog_url: None,
                    installed: vec![plugin],
                })))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                assert!(window.try_find("plugin-empty").is_none());
                reveal(window, ("plugin-details", 0usize), cx);
                window.click(("plugin-details", 0usize), cx);
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(&command, Command::Plugins(PluginCommand::Inspect { id: inspected }) if *inspected == id)));
            events
                .send_blocking(Event::Plugins(PluginEvent::Details {
                    id: id.clone(),
                    package,
                }))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("plugin-service", 0usize), cx);
                let card = window.find(("plugin-card", 0usize)).bounds();
                let included = window.find(("plugin-included", 0usize)).bounds();
                assert!(
                    included.top() >= card.top() && included.bottom() <= card.bottom(),
                    "what's included belongs to its plugin's card"
                );
                reveal(window, ("plugin-details", 0usize), cx);
                window.click(("plugin-details", 0usize), cx);
                window.render_frame(cx);
                assert!(window.try_find(("plugin-included", 0usize)).is_none());
            })
            .unwrap();
            assert!(
                commands.try_recv().is_err(),
                "hiding the details asks the runtime for nothing"
            );
            cx.update_window(window.into(), |_, window, _| window.remove_window())
                .unwrap();
        }
    }
}

#[gpui_kit::test]
fn the_two_ways_to_add_sit_side_by_side_only_when_they_fit(cx: &mut TestAppContext) {
    for sizing in SIZES {
        let (window, _commands, _events) = plugins_window(cx, sizing, ThemeMode::Light);
        cx.update_window(window.into(), |_, window, cx| {
            // The first frame measures the column; the second lays out for it.
            window.render_frame(cx);
            reveal(window, "plugin-github-add", cx);
            let github = window.find("plugin-add-github").bounds();
            let catalog = window.find("plugin-add-catalog").bounds();
            let wide = sizing.0 > 2000.;
            if wide {
                assert!(
                    catalog.left() > github.right(),
                    "wide windows put the catalog beside GitHub"
                );
            } else {
                assert!(
                    catalog.left() <= github.left(),
                    "narrow windows stack the two ways"
                );
            }
            window.remove_window();
        })
        .unwrap();
    }
}

/// This repository's date helper as Sovra would add it from GitHub.
fn code_plugin() -> (InstalledPlugin, sovra_core::plugins::PluginPackage) {
    use sovra_core::plugins::{GitHubRepository, RepositorySnapshot};
    let snapshot = RepositorySnapshot {
        repository: GitHubRepository::parse("Sovra-AI/SovraHarness")
            .expect("the date helper is a valid plugin"),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        description: None,
        skills: Vec::new(),
        manifests: vec![(
            sovra_core::plugins::CODE_MANIFEST_PATH.into(),
            include_bytes!("../../../../sovra-plugin.json").to_vec(),
        )],
        module: Some(include_bytes!("../../../../plugin.wasm").to_vec()),
        left_out: Vec::new(),
    };
    let (entry, bytes, pin) = snapshot
        .package()
        .expect("the date helper is a valid plugin");
    let package = entry
        .verify_package(&bytes)
        .expect("the date helper is a valid plugin");
    (
        InstalledPlugin {
            entry,
            catalog_url: snapshot.repository.web_url(),
            enabled: false,
            github: Some(pin),
        },
        package,
    )
}

#[gpui_kit::test]
fn a_code_plugin_turns_on_only_after_its_access_is_shown_and_allowed(cx: &mut TestAppContext) {
    let (plugin, package) = code_plugin();
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| {
            sovra_ui::init(client, cx);
            design::set_interface_scale(
                sovra_core::models::InterfaceScale::from_percent(scale),
                cx,
            );
        });
        let window = cx.open_window(size(px(width), px(height)), |window, cx| {
            let shell = cx.new(|cx| Shell::new(0, window, cx));
            shell.update(cx, |shell, cx| {
                shell.select_tab(sovra_core::NavTab::Plugins, cx);
            });
            Root::new(shell, window, cx)
        });
        let settle = |cx: &mut TestAppContext| {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
        };
        events
            .send_blocking(Event::Plugins(PluginEvent::Loaded(PluginLibrary {
                catalog_url: None,
                installed: vec![plugin.clone()],
            })))
            .unwrap();
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            reveal(window, ("plugin-toggle", 0usize), cx);
            window.click(("plugin-toggle", 0usize), cx);
        })
        .unwrap();
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(
            sent.iter().any(|command| matches!(command, Command::Plugins(PluginCommand::Inspect { id }) if *id == plugin.entry.id)),
            "Turn on first shows what the code can reach: {sent:?}"
        );
        assert!(
            !sent.iter().any(|command| matches!(
                command,
                Command::Plugins(PluginCommand::SetEnabled { .. })
            )),
            "a code plugin is never switched on by the first press"
        );

        // Until the details arrive, there is nothing to allow.
        cx.update_window(window.into(), |_, window, cx| {
            reveal(window, ("plugin-code-allow", 0usize), cx);
            window.click(("plugin-code-allow", 0usize), cx);
        })
        .unwrap();
        assert!(commands.try_recv().is_err(), "Allow waits for the details");

        events
            .send_blocking(Event::Plugins(PluginEvent::Details {
                id: plugin.entry.id.clone(),
                package: package.clone(),
            }))
            .unwrap();
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let question = window
                .find(("plugin-code-approval", 0usize))
                .label()
                .unwrap_or_default()
                .to_owned();
            assert!(question.starts_with("Allow Sovra-AI/SovraHarness to run its own code?"));
            assert!(question.contains("cannot use the internet"), "{question}");
            assert!(
                question.contains("Count the days between two dates"),
                "{question}"
            );
            reveal(window, ("plugin-code-allow", 0usize), cx);
            window.click(("plugin-code-allow", 0usize), cx);
        })
        .unwrap();
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::Plugins(PluginCommand::AllowCode { id, package_sha256 }) if *id == plugin.entry.id && *package_sha256 == plugin.entry.sha256)));
        cx.update_window(window.into(), |_, window, _| window.remove_window())
            .unwrap();
    }
}

/// The official plugin as it is once installed, with its starter skill.
fn official_plugin(enabled: bool) -> (InstalledPlugin, sovra_core::plugins::PluginPackage) {
    let official = &sovra_core::plugins::OFFICIAL_PLUGINS[0];
    let snapshot = sovra_core::plugins::RepositorySnapshot {
        repository: official.github(),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        description: Some(official.summary.into()),
        skills: vec![
            sovra_core::plugins::repository_skill(
                official.starter().skill.into(),
                official.starter().skill,
                "Start a pool of computers.",
                "Ask which computer should lead the pool.".into(),
            )
            .expect("valid skill"),
        ],
        manifests: Vec::new(),
        module: None,
        left_out: Vec::new(),
    };
    let (entry, bytes, pin) = snapshot.package().expect("package");
    let package = entry.verify_package(&bytes).expect("verified package");
    let plugin = InstalledPlugin {
        entry,
        catalog_url: snapshot.repository.web_url(),
        enabled,
        github: Some(pin),
    };
    (plugin, package)
}

#[gpui_kit::test]
fn an_official_plugin_installs_with_one_click_then_offers_to_try_it(cx: &mut TestAppContext) {
    let official = &sovra_core::plugins::OFFICIAL_PLUGINS[0];
    for sizing in SIZES {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(sizing.2),
                    cx,
                );
            });
            let mut shell = None;
            let window = cx.open_window(size(px(sizing.0), px(sizing.1)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                view.update(cx, |shell, cx| {
                    shell.select_tab(sovra_core::NavTab::Plugins, cx);
                });
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.expect("shell view");
            while commands.try_recv().is_ok() {}
            cx.update_window(window.into(), |_, window, cx| {
                // What it does is on the page before anything is installed.
                window.render_frame(cx);
                assert!(window.try_find(("plugin-official-card", 0usize)).is_some());
                assert!(window.try_find(("plugin-official-try", 0usize)).is_none());
                assert!(
                    window
                        .try_find(("plugin-official-action", 1usize))
                        .is_none(),
                    "next steps appear once it is on"
                );
                reveal(window, ("plugin-official-install", 0usize), cx);
                window.click(("plugin-official-install", 0usize), cx);
            })
            .unwrap();
            let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
            assert!(
                matches!(sent.as_slice(), [Command::Plugins(PluginCommand::InstallOfficial { id })] if *id == official.id()),
                "one click sends exactly one install: {sent:?}"
            );

            let (plugin, package) = official_plugin(true);
            events
                .send_blocking(Event::Plugins(PluginEvent::Loaded(PluginLibrary {
                    catalog_url: None,
                    installed: vec![plugin],
                })))
                .unwrap();
            events
                .send_blocking(Event::Skills(sovra_core::skills::SkillEvent::Loaded {
                    skills: package.skills(true),
                    issues: Vec::new(),
                }))
                .unwrap();
            events
                .send_blocking(Event::Plugins(PluginEvent::Added {
                    id: official.id(),
                    package,
                    left_out: Vec::new(),
                    enabled: true,
                }))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                assert!(
                    window
                        .try_find(("plugin-official-install", 0usize))
                        .is_none()
                );
                reveal(window, "plugin-notice", cx);
                // It is also listed with the plugins you have, under its everyday name.
                assert!(window.try_find(("plugin-card", 0usize)).is_some());
                for step in 1..official.actions.len() {
                    // Every next step stays reachable, wrapping at narrow sizes.
                    reveal(window, ("plugin-official-action", step), cx);
                }
                reveal(window, ("plugin-official-try", 0usize), cx);
                window.click(("plugin-official-try", 0usize), cx);
                assert_eq!(
                    shell.read(cx).workspace().active_tab,
                    sovra_core::NavTab::Chat,
                    "Try it in chat opens the chat with its starter request"
                );
                window.remove_window();
            })
            .unwrap();
        }
    }
}
