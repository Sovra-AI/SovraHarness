//! GPUI views — the only crate that depends on `gpui-kit`.
//!
//! The UI reaches the runtime exclusively through the [`RuntimeClient`] global
//! (a [`sovra_runtime::Client`]); it never blocks and never touches tokio.

mod agui_widgets;
mod app_menu;
mod artifacts_view;
mod assets;
mod chat_history_view;
mod chat_view;
mod code_panel;
mod composer;
mod connections_view;
pub mod design;
mod development_view;
mod file_index_view;
mod github_card;
mod google_card;
mod groups_view;
mod inspector;
mod knowledge_review;
mod knowledge_view;
mod login;
mod markdown_segments;
mod memory_view;
mod message_review;
mod model_connection_editor;
mod model_details;
mod model_home;
mod models_view;
mod my_ais_view;
mod native_surface;
mod onboarding_bar;
mod remote_control_view;
mod schedules_view;
mod service_connect_view;
mod settings_view;
mod shell;
mod sidebar;
mod skill_library;
mod skill_menu;
mod team_view;
mod terminal_card;
mod terminal_settings;
mod widgets;
mod work_log;

use gpui_kit::component::TitleBar;
use gpui_kit::{App, AppContext as _, Global, TitlebarOptions, WindowBounds, WindowOptions};
use sovra_runtime::Client;

pub use models_view::ModelsView;
pub use shell::Shell;

const PRODUCT_NAME: &str = "SovraHarness";
/// The name people see on the window, matching the sidebar brand.
const WINDOW_BRAND: &str = "Sovra";

/// The runtime handle, installed once as a GPUI global so any view can reach it with
/// `cx.global::<RuntimeClient>()`.
#[derive(Debug, Clone)]
pub struct RuntimeClient(pub Client);

impl Global for RuntimeClient {}

/// Runs the desktop UI on the current (main) thread until the user quits.
pub fn run(client: Client, session_namespace: u128) {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            init(client, cx);
            open_main_window(cx, session_namespace);
            cx.activate(true);
        });
}

/// Registers globals, actions, key bindings and menus. Tests call this too.
pub fn init(client: Client, cx: &mut App) {
    gpui_kit::init(cx);
    design::init(cx);
    cx.set_global(RuntimeClient(client));
    cx.on_app_quit(|cx| {
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(sovra_core::Command::Shutdown);
        async {}
    })
    .detach();
    app_menu::init(PRODUCT_NAME, cx);
}

fn open_main_window(cx: &mut App, session_namespace: u128) {
    let layout = design::tokens(cx).layout;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(design::initial_window_bounds(cx))),
        window_min_size: Some(gpui_kit::size(
            layout.window_min_width,
            layout.window_min_height,
        )),
        // Window managers, taskbars and Alt-Tab read this even when the app draws
        // its own title bar; the shell keeps it in step with the open page.
        titlebar: Some(TitlebarOptions {
            title: Some(WINDOW_BRAND.into()),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    };
    if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Shell::new(session_namespace, window, cx))
    }) {
        tracing::error!(%error, "failed to open the main window");
        cx.quit();
    }
}
