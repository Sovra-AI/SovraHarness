//! Batch models stay inspectable, but never enter a chat picker.

use super::*;

#[gpui_kit::test]
fn batch_models_are_hidden_from_chat_and_disabled_in_details(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    let (window, shell) = open_shell(cx, client, 1600., 820.);
    for event in fixture_events(vec![
        model("google/gemini-3.8-flash", "Gemini Flash", "text", true),
        model(
            "google/gemini-3.8-flash:batch",
            "Gemini Flash (batch)",
            "text",
            true,
        ),
    ]) {
        events
            .send_blocking(Event::Models(event))
            .expect("fixture event");
    }
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        window.click("models-browse-all", cx);
        let view = shell.read(cx).models();
        assert_eq!(view.read(cx).visible_models().len(), 1);
        assert_eq!(
            view.read(cx).visible_models()[0].model_id,
            "google/gemini-3.8-flash"
        );
        view.update(cx, |view, cx| {
            view.show_kind(None, window, cx);
            view.set_search(":batch", window, cx);
        });
        window.render_frame(cx);
        assert_eq!(view.read(cx).visible_models().len(), 1);
        window.click(("model-row", 0usize), cx);
        assert!(window.find("model-details").visible());
        let action = window.find("model-activate");
        assert_eq!(action.label(), Some("Can't be used in chat"));
        assert_eq!(action.disabled(), Some(true));
        window.click("model-activate", cx);
    })
    .expect("window exists");
    assert!(
        !std::iter::from_fn(|| commands.try_recv().ok())
            .any(|c| matches!(c, Command::Models(ModelCommand::Activate { .. })))
    );
}
