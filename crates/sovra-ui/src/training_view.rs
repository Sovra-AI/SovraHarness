//! The Training page: teach a small model from text the person chooses, watch it
//! learn, and keep the result. The view only sends commands and shows the
//! runtime's snapshots; it never reads files or does any training itself.

mod advanced;
mod choices;
mod form;
mod helping;
mod knobs;
mod library;
mod live;
mod parts;
mod setup;
mod words;

use std::path::PathBuf;

use crate::{
    RuntimeClient,
    design::{self, WidthClass},
};
use form::{Facts, Form, SourceChoice};
use gpui_kit::component::{
    input::{InputEvent, InputState, TextareaState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    TestSupportExt as _, Window, div,
};
use knobs::Knob;
use sovra_core::{
    Command,
    training::{
        HardwareInfo, LocalOptimizer, LrSchedule, ModelId, TrainingBase, TrainingCommand,
        TrainingEvent, TrainingPhase, TrainingPreset, TrainingSource, TrainingState,
    },
};

pub(crate) struct TrainingView {
    state: TrainingState,
    form: Form,
    /// Stands in until the runtime has reported this computer's hardware.
    unknown_hardware: HardwareInfo,
    /// A command is on its way; cleared by the next snapshot. Stops double sends.
    pending: bool,
    stale: Stale,
    width_class: WidthClass,
    advanced_open: bool,
    all_models_listed: bool,
    text: Entity<TextareaState>,
    pasted: Pasted,
    name: Entity<InputState>,
    /// The name as the person typed it. Until they type one, the default name
    /// follows the chosen model.
    typed_name: Option<String>,
    /// One input per [`Knob`], in [`Knob::ALL`] order.
    knobs: Vec<Entity<InputState>>,
    try_prompt: Entity<InputState>,
    /// The library model whose Try it panel is open.
    trying: Option<ModelId>,
    file_notice: Option<String>,
    picker: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// Input controls whose text the form's choices have moved on from.
#[derive(Default)]
struct Stale {
    /// The expert number fields no longer show the form's settings.
    knobs: bool,
    /// The name field no longer shows the default name for the chosen model.
    name: bool,
}

/// Length and blankness of the pasted text, kept so a frame never copies it.
#[derive(Clone, Copy)]
struct Pasted {
    bytes: usize,
    blank: bool,
}

/// Borrows only the fields the form's decisions need, so the form itself can be
/// changed in the same statement.
fn facts<'a>(state: &'a TrainingState, unknown: &'a HardwareInfo, pasted: Pasted) -> Facts<'a> {
    Facts {
        hardware: state.hardware.as_ref().unwrap_or(unknown),
        models: &state.models,
        pasted_bytes: pasted.bytes,
        pasted_blank: pasted.blank,
        in_group: state.group.is_some(),
    }
}

impl TrainingView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let unknown_hardware = HardwareInfo::default();
        let form = Form::new(&unknown_hardware);
        let name_value = form.default_name(&[]);
        let mut subscriptions = Vec::new();
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Paste the text you want it to learn from.")
                .auto_grow(3, 8)
        });
        subscriptions.push(cx.subscribe_in(
            &text,
            window,
            |view, text, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = text.read(cx).value();
                    view.pasted = Pasted {
                        bytes: value.len(),
                        blank: value.trim().is_empty(),
                    };
                    view.settings_follow(cx);
                }
            },
        ));
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Name for your model")
                .default_value(name_value)
        });
        subscriptions.push(cx.subscribe_in(
            &name,
            window,
            |view, name, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    view.typed_name = Some(name.read(cx).value().to_string());
                    cx.notify();
                }
            },
        ));
        let knobs: Vec<Entity<InputState>> = Knob::ALL
            .into_iter()
            .map(|knob| {
                let shown = knob.shown(&form.settings);
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(shown)
                        .step(knob.step())
                        .min(0.)
                });
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    |view, _, event: &InputEvent, _, cx| {
                        if matches!(event, InputEvent::Change) {
                            let (schedule, optimizer) =
                                (view.form.settings.schedule, view.form.settings.optimizer);
                            view.settings_edited(schedule, optimizer, cx);
                        }
                    },
                ));
                input
            })
            .collect();
        let try_prompt = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Type a beginning, such as: The garden")
        });
        subscriptions.push(cx.subscribe_in(
            &try_prompt,
            window,
            |view, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.run_trial(cx);
                }
            },
        ));
        Self {
            state: TrainingState::default(),
            form,
            unknown_hardware,
            pending: false,
            stale: Stale::default(),
            width_class: WidthClass::Compact,
            advanced_open: false,
            all_models_listed: false,
            text,
            pasted: Pasted {
                bytes: 0,
                blank: true,
            },
            name,
            typed_name: None,
            knobs,
            try_prompt,
            trying: None,
            file_notice: None,
            picker: None,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn apply(&mut self, event: &TrainingEvent, cx: &mut Context<Self>) {
        let before = self.form.base;
        self.state.apply(event);
        self.pending = false;
        if self
            .form
            .follow_hardware(facts(&self.state, &self.unknown_hardware, self.pasted))
        {
            self.stale.knobs = true;
        }
        if self.form.base != before {
            self.stale.name = true;
        }
        if self
            .trying
            .is_some_and(|model| self.state.model(model).is_none())
        {
            self.trying = None;
        }
        cx.notify();
    }

    fn send(&mut self, command: TrainingCommand, cx: &mut Context<Self>) {
        match cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Training(command))
        {
            Ok(()) => {
                self.pending = true;
                self.state.error = None;
            }
            Err(error) => self.state.error = Some(error.to_string()),
        }
        cx.notify();
    }

    /// Whether the choices are locked: a run or a command is in progress, or the
    /// person asked this computer to help someone by hand. Help that started by
    /// itself does not lock anything: it gives way the moment a run is started.
    fn locked(&self) -> bool {
        let helping_by_hand = self
            .state
            .helping
            .as_ref()
            .is_some_and(|help| help.phase.active() && help.run.is_none());
        self.pending
            || self.state.phase.busy()
            || self.state.phase == TrainingPhase::Unavailable
            || helping_by_hand
    }

    fn compact(&self) -> bool {
        self.width_class == WidthClass::Compact
    }

    fn problem(&self) -> Option<String> {
        self.form.problem(
            self.typed_name.as_deref().unwrap_or_default(),
            facts(&self.state, &self.unknown_hardware, self.pasted),
        )
    }

    /// Re-derives preset settings after the text changed.
    fn settings_follow(&mut self, cx: &mut Context<Self>) {
        if self
            .form
            .refresh(facts(&self.state, &self.unknown_hardware, self.pasted))
        {
            self.stale.knobs = true;
        }
        cx.notify();
    }

    fn choose_source(&mut self, source: SourceChoice, cx: &mut Context<Self>) {
        self.file_notice = None;
        if self.form.choose_source(
            source,
            facts(&self.state, &self.unknown_hardware, self.pasted),
        ) {
            self.stale.knobs = true;
        }
        cx.notify();
    }

    /// Opens the operating system's file dialog. The path is only passed on in
    /// the plan; the runtime is what reads the file.
    fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use this text".into()),
        });
        self.picker = Some(cx.spawn_in(window, async move |this, cx| {
            let result = selection.await;
            let _ = this.update_in(cx, |view, _, cx| {
                match result {
                    Ok(Ok(Some(paths))) => view.file_chosen(paths.into_iter().next(), cx),
                    Ok(Ok(None)) => {}
                    Ok(Err(_)) | Err(_) => {
                        view.file_notice = Some(
                            "The file picker couldn't open. Please try Use a text file again."
                                .into(),
                        );
                    }
                }
                cx.notify();
            });
        }));
    }

    fn file_chosen(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(path) = path else {
            return;
        };
        if let Err(error) = TrainingSource::File(path.clone()).validate() {
            self.file_notice = Some(error.to_string());
            return;
        }
        self.form.file = Some(path);
        self.choose_source(SourceChoice::File, cx);
    }

    fn choose_base(&mut self, base: TrainingBase, cx: &mut Context<Self>) {
        if self.form.choose_base(
            base,
            facts(&self.state, &self.unknown_hardware, self.pasted),
        ) {
            self.stale.knobs = true;
        }
        self.stale.name = true;
        cx.notify();
    }

    fn choose_preset(&mut self, preset: TrainingPreset, cx: &mut Context<Self>) {
        self.form.choose_preset(
            preset,
            facts(&self.state, &self.unknown_hardware, self.pasted),
        );
        self.stale.knobs = true;
        cx.notify();
    }

    fn reset_settings(&mut self, cx: &mut Context<Self>) {
        self.form
            .reset(facts(&self.state, &self.unknown_hardware, self.pasted));
        self.stale.knobs = true;
        cx.notify();
    }

    /// Reads every expert field as typed, with the given named choices.
    fn settings_edited(
        &mut self,
        schedule: LrSchedule,
        optimizer: LocalOptimizer,
        cx: &mut Context<Self>,
    ) {
        let texts: Vec<String> = self
            .knobs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect();
        self.form.edited(
            &texts,
            schedule,
            optimizer,
            facts(&self.state, &self.unknown_hardware, self.pasted),
        );
        cx.notify();
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        let text = self.text.clone();
        let plan = self.form.plan(
            self.typed_name.as_deref().unwrap_or_default(),
            || text.read(cx).value().to_string(),
            facts(&self.state, &self.unknown_hardware, self.pasted),
        );
        match plan {
            Ok(plan) => self.send(TrainingCommand::Start(Box::new(plan)), cx),
            Err(problem) => {
                self.state.error = Some(problem);
                cx.notify();
            }
        }
    }

    fn run_trial(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.trying else {
            return;
        };
        if self.pending || self.state.trying() {
            return;
        }
        let prompt = self.try_prompt.read(cx).value().to_string();
        let command = TrainingCommand::Try { model, prompt };
        match command.validate() {
            Ok(()) => self.send(command, cx),
            Err(error) => {
                self.state.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    /// Rewrites the input controls the form's choices have moved on from. Typed
    /// text is never replaced while the person is the one changing it.
    fn write_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stale.knobs {
            self.stale.knobs = false;
            for knob in Knob::ALL {
                let shown = knob.shown(&self.form.settings);
                if let Some(input) = self.knobs.get(knob.index()) {
                    input.update(cx, |input, cx| input.set_value(shown, window, cx));
                }
            }
        }
        if self.stale.name {
            self.stale.name = false;
            if self.typed_name.is_none() {
                let name = self.form.default_name(&self.state.models);
                self.name
                    .update(cx, |input, cx| input.set_value(name, window, cx));
            }
        }
    }
}

impl Render for TrainingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.write_inputs(window, cx);
        let t = design::tokens(cx);
        let running = matches!(
            self.state.phase,
            TrainingPhase::Preparing
                | TrainingPhase::Running
                | TrainingPhase::Pausing
                | TrainingPhase::Paused
        );
        let has_run = self.state.plan.is_some() || self.state.progress.total_rounds > 0;
        let mut column = v_flex()
            .relative()
            .w_full()
            .min_w_0()
            .max_w(t.layout.reading_width)
            .gap(t.space.large)
            .child(design::width_probe(cx, |view: &mut Self| {
                &mut view.width_class
            }))
            .child(design::page_header(
                "Training",
                "Teach your own small AI model from text you choose, on your own computers.",
                cx,
            ))
            .child(parts::body(
                "A model learns the style of the text you give it and then writes more in that style. These are small models you teach yourself. They do not replace your chat AI, and large ready-made models cannot be brought in yet. Nothing is downloaded and nothing leaves your computers.",
                cx,
            ));
        if let Some(notice) = &self.state.notice {
            column = column.child(parts::status("training-notice", notice.clone(), cx));
        }
        if let Some(error) = &self.state.error {
            column = column.child(parts::alert("training-error", error.clone(), cx));
        }
        if self.state.phase == TrainingPhase::Unavailable {
            column = column.child(parts::status(
                "training-unavailable",
                "Training is not available in this session. Restart Sovra to try again.",
                cx,
            ));
        }
        // A run in progress comes first so pressing Start shows it learning
        // without scrolling; a finished run sits below the choices for the next.
        if has_run && running {
            column = column.child(self.live_card(cx));
        }
        column = column
            .child(self.hardware_card(cx))
            .child(self.source_card(cx))
            .child(self.base_card(cx))
            .child(self.preset_card(cx))
            .child(self.start_card(cx))
            .child(self.advanced_card(cx));
        if has_run && !running {
            column = column.child(self.live_card(cx));
        }
        column = column
            .child(self.library_card(cx))
            .child(self.helping_card(cx));
        div()
            .id("training-page")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .p(if self.compact() {
                        t.space.medium
                    } else {
                        t.space.large
                    })
                    .child(column),
            )
    }
}
