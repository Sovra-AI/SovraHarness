//! Development controls and live native UI observation. Inputs use GPUI dispatch.
use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, TextRole},
    models_view::{ModelsView, caption},
    settings_view::settings_section,
};
use gpui_kit::component::{ActiveTheme as _, checkbox::Checkbox};
use gpui_kit::test::{ElementSnapshot, TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    App, Context, InputEvent as _, IntoElement, Keystroke, MouseButton, ParentElement as _,
    ScrollDelta, ScrollWheelEvent, Styled as _, Window, point,
};
use serde_json::{Value, json};
use sovra_core::{
    Command,
    development::{DevelopmentCommand, MAX_ELEMENTS, UiRequest},
};

impl ModelsView {
    pub(crate) fn render_development_section(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let enabled = self.development.enabled;
        settings_section("settings-development", "Development mode", "Let coding assistants inspect and test this app through a local MCP connection. They can operate real controls and change your data. Use a test workspace for destructive tests.", cx)
            .child(Checkbox::new("development-mode-toggle").cursor_pointer().label("Enable development mode").checked(enabled)
                .on_click(cx.listener(|view, checked: &bool, _, cx| {
                    if crate::remote_control_view::is_remote_input(cx) { return; }
                    // Close the UI gate synchronously; queued input cannot overtake the off switch.
                    view.development.enabled = *checked;
                    if !checked { view.development.generation = None; }
                    let _ = cx.global::<RuntimeClient>().0.send(Command::Development(DevelopmentCommand::SetEnabled(*checked)));
                    cx.notify();
                })))
            .child(caption(if enabled { "On · local testing access is enabled" } else { "Off · coding assistants cannot control Sovra" }, cx))
            .child(gpui_kit::div().min_w_0().sovra_text(TextRole::Caption, cx).child("Connect with this command (stdio MCP):"))
            .child(gpui_kit::div().w_full().min_w_0().truncate().sovra_text(TextRole::Caption, cx).child(self.development.connection_command.clone()))
            .child(design::button("development-copy-command", "Copy connection command", cx)
                .on_click(cx.listener(|view, _, _, cx| {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(view.development.connection_command.clone()));
                })))
            .children(self.development.error.as_ref().map(|error| caption(error.clone(), cx).text_color(cx.theme().danger)))
    }
}

fn target_id(element: &ElementSnapshot, generation: &str) -> String {
    format!("{generation}:{:?}", element.path())
}

fn target(window: &Window, id: &str, generation: &str) -> Result<ElementSnapshot, String> {
    let mut matches = window
        .elements()
        .into_iter()
        .filter(|element| target_id(element, generation) == id);
    let element = matches
        .next()
        .ok_or("Target is no longer present; inspect again.")?;
    if matches.next().is_some() {
        return Err("Target is ambiguous; inspect again.".into());
    }
    if !element.visible() || element.disabled() == Some(true) {
        return Err("Target is hidden or disabled; scroll it into view before acting.".into());
    }
    Ok(element)
}

pub(crate) enum Output {
    Data(Value),
    Image(image::RgbaImage),
}

impl Output {
    /// PNG compression stays off the UI thread.
    pub(crate) fn encode(self) -> Result<Value, String> {
        match self {
            Self::Data(value) => Ok(value),
            Self::Image(image) => {
                use base64::Engine as _;
                let image = image::DynamicImage::ImageRgba8(image).thumbnail(1600, 1600);
                let mut bytes = std::io::Cursor::new(Vec::new());
                image
                    .write_to(&mut bytes, image::ImageFormat::Png)
                    .map_err(|_| "Could not encode screenshot")?;
                if bytes.get_ref().len() > 5 * 1024 * 1024 {
                    return Err("Screenshot exceeds its size limit.".into());
                }
                Ok(
                    json!({"png": base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())}),
                )
            }
        }
    }
}

/// Called with no borrowed view: rendering or dispatch may re-enter any entity.
pub(crate) fn execute(
    request: UiRequest,
    generation: &str,
    viewport: gpui_kit::Bounds<gpui_kit::Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Result<Output, String> {
    request.validate().map_err(str::to_owned)?;
    window.render_frame(cx);
    let result = match request {
        UiRequest::Screenshot {} => {
            return window.render_to_image().map(Output::Image).map_err(|_| {
                "Screenshot capture is unavailable on this platform renderer.".into()
            });
        }
        UiRequest::Inspect {} => {
            let mut elements = window.elements();
            // Long pages must not crowd currently visible controls out of the budget.
            elements.sort_by_key(|element| !element.visible());
            let total = elements.len();
            let elements = elements.into_iter().take(MAX_ELEMENTS).map(|element| {
                let bounds = element.bounds();
                json!({
                    "target": target_id(&element, generation), "label": element.label().map(|label| label.chars().take(1024).collect::<String>()),
                    "role": element.role().map(|role| format!("{role:?}")),
                    "visible": element.visible(), "disabled": element.disabled(),
                    "checked": element.checked(), "selected": element.selected(), "expanded": element.expanded(),
                    "bounds": {"x": f32::from(bounds.origin.x), "y": f32::from(bounds.origin.y), "width": f32::from(bounds.size.width), "height": f32::from(bounds.size.height)}
                })
            }).collect::<Vec<_>>();
            // Native accessibility values can contain passwords/API keys. Omit all
            // input values by default; add explicitly reviewed safe diagnostics only.
            Ok(
                json!({"elements": elements, "total": total, "truncated": total > MAX_ELEMENTS, "input_values": "omitted for privacy", "appearance": if cx.theme().is_dark() { "dark" } else { "light" }, "interface_scale_percent": design::interface_scale(cx).percent()}),
            )
        }
        UiRequest::Click { target: id } => {
            let element = target(window, &id, generation)?;
            let position = element.bounds().intersect(&viewport).center();
            window.pointer_move(position, None, cx);
            window.pointer_down(position, MouseButton::Left, cx);
            window.pointer_up(position, MouseButton::Left, cx);
            Ok(
                json!({"input_delivered": true, "next": "Inspect to verify the result; asynchronous work may still be running."}),
            )
        }
        UiRequest::Type { text } => {
            // Pre-validate every character before delivering any part of the input.
            let keys = text
                .chars()
                .map(|character| {
                    let value = character.to_string();
                    let mut key = Keystroke::parse(&value)
                        .map_err(|_| "Text contains an unsupported keystroke")?;
                    key.key_char = Some(value);
                    Ok::<_, &str>(key)
                })
                .collect::<Result<Vec<_>, _>>()?;
            for key in keys {
                if !window.dispatch_keystroke(key, cx) {
                    return Err("The focused control did not accept text. Inspect and focus an input before trying again; earlier characters may have been delivered.".into());
                }
            }
            window.render_frame(cx);
            Ok(json!({"input_delivered": true, "next": "Inspect to verify the result."}))
        }
        UiRequest::Key { key } => {
            Keystroke::parse(&key).map_err(|_| "Invalid GPUI keystroke")?;
            window.press(&key, cx);
            Ok(json!({"input_delivered": true, "next": "Inspect to verify the result."}))
        }
        UiRequest::Scroll { target: id, lines } => {
            let element = target(window, &id, generation)?;
            let position = element.bounds().intersect(&viewport).center();
            window.dispatch_event(
                ScrollWheelEvent {
                    position,
                    delta: ScrollDelta::Lines(point(0., f32::from(lines))),
                    modifiers: window.modifiers(),
                    ..ScrollWheelEvent::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            Ok(json!({"input_delivered": true, "next": "Inspect to discover visible controls."}))
        }
    };
    result.map(Output::Data)
}
