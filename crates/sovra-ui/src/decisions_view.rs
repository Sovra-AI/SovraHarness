//! Decision library and forms. Saved state is folded by the shell; inputs remain
//! here until the runtime confirms persistence, including on failed saves.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _,
    button::ButtonVariants as _,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    TestSupportExt as _, WeakEntity, Window, div,
};
use sovra_core::decisions::{
    DECISION_PAGE_SIZE, DecisionBook, DecisionCommand, DecisionDraft, DecisionEvent,
    DecisionEvidence, DecisionPerson, DecisionRecord, DecisionStop,
};
use sovra_core::{Command, Timestamp};

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
    shell::Shell,
};

struct EvidenceInput {
    reference: Entity<InputState>,
    passage: Entity<TextareaState>,
}
struct StopInput {
    condition: Entity<InputState>,
    cleared: bool,
    _events: Subscription,
}
struct Editor {
    id: Option<String>,
    revision: u64,
    question: Entity<InputState>,
    decision: Entity<TextareaState>,
    criteria: Entity<TextareaState>,
    review_on: Entity<InputState>,
    ends_when: Entity<TextareaState>,
    evidence: Vec<EvidenceInput>,
    stops: Vec<StopInput>,
    chat: Option<DecisionEvidence>,
}

enum Page {
    List,
    Detail,
    Edit,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirmation {
    Discard,
    Accept,
    End,
}

pub(crate) struct DecisionsView {
    shell: WeakEntity<Shell>,
    page: Page,
    editor: Option<Editor>,
    end_reason: Entity<TextareaState>,
    reset_reason: bool,
    confirmation: Option<Confirmation>,
    pending: bool,
    error: Option<String>,
    history: bool,
    scroll: gpui_kit::ScrollHandle,
    _review_tick: Task<()>,
}

fn now() -> Timestamp {
    Timestamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| {
                i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
            }),
    )
}

fn input(value: &str, window: &mut Window, cx: &mut Context<DecisionsView>) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx);
        state.set_value(value.to_owned(), window, cx);
        state
    })
}
fn textarea(
    value: &str,
    window: &mut Window,
    cx: &mut Context<DecisionsView>,
) -> Entity<TextareaState> {
    cx.new(|cx| {
        let mut state = TextareaState::new(window, cx).auto_grow(3, 10);
        state.set_value(value.to_owned(), window, cx);
        state
    })
}

fn stop_input(
    value: &str,
    cleared: bool,
    window: &mut Window,
    cx: &mut Context<DecisionsView>,
) -> StopInput {
    let condition = input(value, window, cx);
    let events = cx.subscribe(&condition, |view, input, event, cx| {
        if matches!(event, InputEvent::Change)
            && let Some(editor) = &mut view.editor
        {
            for stop in &mut editor.stops {
                if stop.condition.entity_id() == input.entity_id() {
                    stop.cleared = false;
                }
            }
            cx.notify();
        }
    });
    StopInput {
        condition,
        cleared,
        _events: events,
    }
}

impl Editor {
    fn new(
        id: Option<String>,
        revision: u64,
        draft: DecisionDraft,
        window: &mut Window,
        cx: &mut Context<DecisionsView>,
    ) -> Self {
        let evidence = if draft.evidence.is_empty() {
            vec![DecisionEvidence {
                reference: String::new(),
                passage: String::new(),
            }]
        } else {
            draft.evidence
        };
        Self {
            id,
            revision,
            question: input(&draft.question, window, cx),
            decision: textarea(&draft.decision, window, cx),
            criteria: textarea(&draft.criteria, window, cx),
            review_on: input(&draft.review_on, window, cx),
            ends_when: textarea(&draft.ends_when, window, cx),
            evidence: evidence
                .into_iter()
                .map(|item| EvidenceInput {
                    reference: input(&item.reference, window, cx),
                    passage: textarea(&item.passage, window, cx),
                })
                .collect(),
            stops: draft
                .hard_stops
                .into_iter()
                .map(|stop| stop_input(&stop.condition, stop.cleared, window, cx))
                .collect(),
            chat: draft.chat,
        }
    }

    fn draft(&self, cx: &App) -> DecisionDraft {
        DecisionDraft {
            question: self.question.read(cx).value().trim().into(),
            decision: self.decision.read(cx).value().to_string(),
            criteria: self.criteria.read(cx).value().to_string(),
            review_on: self.review_on.read(cx).value().trim().into(),
            ends_when: self.ends_when.read(cx).value().to_string(),
            evidence: self
                .evidence
                .iter()
                .map(|item| DecisionEvidence {
                    reference: item.reference.read(cx).value().to_string(),
                    passage: item.passage.read(cx).value().to_string(),
                })
                .collect(),
            hard_stops: self
                .stops
                .iter()
                .map(|stop| DecisionStop {
                    condition: stop.condition.read(cx).value().to_string(),
                    cleared: stop.cleared,
                })
                .collect(),
            chat: self.chat.clone(),
        }
    }
}

impl DecisionsView {
    pub(crate) fn new(
        shell: WeakEntity<Shell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            shell,
            page: Page::List,
            editor: None,
            end_reason: textarea("", window, cx),
            reset_reason: false,
            confirmation: None,
            pending: false,
            error: None,
            history: false,
            scroll: gpui_kit::ScrollHandle::new(),
            _review_tick: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(60))
                        .await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }),
        }
    }

    fn book(&self, cx: &App) -> DecisionBook {
        self.shell
            .upgrade()
            .map(|shell| shell.read(cx).state.decisions.clone())
            .unwrap_or_default()
    }
    fn person(&self, cx: &App) -> Option<DecisionPerson> {
        self.shell.upgrade().and_then(|shell| {
            shell
                .read(cx)
                .state
                .auth
                .session()
                .map(DecisionPerson::from)
        })
    }

    fn send(&mut self, command: DecisionCommand, cx: &mut Context<Self>) {
        self.error = None;
        match cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Decisions(command))
        {
            Ok(()) => self.pending = true,
            Err(error) => self.error = Some(format!("Your changes are still here. {error}")),
        }
        cx.notify();
    }

    pub(crate) fn show(&mut self, cx: &mut Context<Self>) {
        if self.editor.is_none() && !self.pending {
            self.page = Page::List;
            self.send(DecisionCommand::Load { offset: 0 }, cx);
        }
        cx.notify();
    }

    pub(crate) fn start(
        &mut self,
        draft: DecisionDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.is_some() || self.pending {
            self.error = Some("Finish or discard the decision already open before starting another. Your draft is still here.".into());
        } else {
            self.editor = Some(Editor::new(None, 0, draft, window, cx));
            self.page = Page::Edit;
            self.error = None;
            self.confirmation = None;
            self.scroll.set_offset(gpui_kit::Point::default());
        }
        cx.notify();
    }

    pub(crate) fn apply(&mut self, event: &DecisionEvent, cx: &mut Context<Self>) {
        self.pending = false;
        self.confirmation = None;
        match event {
            DecisionEvent::Saved(_) => {
                self.reset_reason = true;
                self.editor = None;
                self.page = Page::Detail;
                self.error = None;
                self.history = false;
                self.scroll.set_offset(gpui_kit::Point::default());
            }
            DecisionEvent::Opened(_) => {
                self.reset_reason = true;
                self.page = Page::Detail;
                self.error = None;
                self.history = false;
                self.scroll.set_offset(gpui_kit::Point::default());
            }
            DecisionEvent::Listed { .. } => self.error = None,
            DecisionEvent::Error(message) => self.error = Some(message.clone()),
        }
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        if self.editor.is_some() {
            self.confirmation = Some(Confirmation::Discard);
        } else {
            self.page = Page::List;
            self.send(DecisionCommand::Load { offset: 0 }, cx);
        }
        self.scroll.set_offset(gpui_kit::Point::default());
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else { return };
        let draft = editor.draft(cx);
        if let Err(error) = draft.validate() {
            self.error = Some(error.to_string());
            self.scroll.set_offset(gpui_kit::Point::default());
            cx.notify();
            return;
        }
        self.send(
            DecisionCommand::Save {
                id: editor.id.clone(),
                expected_revision: editor.revision,
                draft: Box::new(draft),
            },
            cx,
        );
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        match self.confirmation.take() {
            Some(Confirmation::Discard) => {
                self.editor = None;
                self.page = Page::List;
                self.send(DecisionCommand::Load { offset: 0 }, cx);
            }
            Some(Confirmation::Accept) => {
                if let Some(record) = self.book(cx).open {
                    self.send(
                        DecisionCommand::Accept {
                            id: record.id,
                            expected_revision: record.revision,
                        },
                        cx,
                    );
                }
            }
            Some(Confirmation::End) => {
                if let Some(record) = self.book(cx).open {
                    self.send(
                        DecisionCommand::End {
                            id: record.id,
                            expected_revision: record.revision,
                            reason: self.end_reason.read(cx).value().to_string(),
                        },
                        cx,
                    );
                }
            }
            None => {}
        }
        cx.notify();
    }

    fn list(&self, book: &DecisionBook, cx: &mut Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        let mut list = v_flex().w_full().min_w_0().gap(t.space.medium).child(
            design::button("decision-new", "New decision", cx)
                .disabled(self.pending)
                .on_click(cx.listener(|view, _, window, cx| {
                    view.start(DecisionDraft::default(), window, cx);
                })),
        );
        if book.records.is_empty() {
            list = list.child("No decisions on this page. Save one here or choose Record a decision under a chat reply.");
        }
        for (index, record) in book.records.iter().enumerate() {
            let id = record.id.clone();
            list = list.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .p(t.space.medium)
                    .sovra_surface(Surface::Matte, cx)
                    .child(
                        div()
                            .min_w_0()
                            .sovra_text(TextRole::SectionTitle, cx)
                            .child(record.question.clone()),
                    )
                    .child(format!(
                        "{} · Review on {} (UTC)",
                        record.status(now()).label(),
                        record.review_on
                    ))
                    .child(
                        design::button(("decision-open", index), "Open decision", cx)
                            .disabled(self.pending)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.send(DecisionCommand::Open { id: id.clone() }, cx);
                            })),
                    ),
            );
        }
        let previous = book.offset.saturating_sub(DECISION_PAGE_SIZE);
        let next = book.offset.saturating_add(DECISION_PAGE_SIZE);
        list.child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("decision-previous", "Previous page", cx)
                        .disabled(self.pending || book.offset == 0)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.send(DecisionCommand::Load { offset: previous }, cx);
                        })),
                )
                .child(
                    design::button("decision-next", "Next page", cx)
                        .disabled(self.pending || !book.more)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.send(DecisionCommand::Load { offset: next }, cx);
                        })),
                ),
        )
    }

    fn form(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        let Some(editor) = &self.editor else {
            return div();
        };
        let mut form = v_flex().w_full().min_w_0().gap(t.space.medium)
            .child("Saving keeps a draft. A second person must sign in with their own account and accept it before it becomes the company position.")
            .child(input_field("Question", "decision-question", &editor.question, self.pending, cx))
            .child(text_field("Decision", "decision-outcome", &editor.decision, self.pending, cx))
            .child(text_field("Our criteria", "decision-criteria", &editor.criteria, self.pending, cx))
            .child(input_field("Review on (YYYY-MM-DD, UTC)", "decision-review", &editor.review_on, self.pending, cx))
            .child(text_field("What would end this decision?", "decision-ends-when", &editor.ends_when, self.pending, cx))
            .child("Cited passages — include the file or source, page or lines, and the wording you relied on. These are the quotations you enter; Sovra does not certify them.");
        for (index, evidence) in editor.evidence.iter().enumerate() {
            form = form.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .p(t.space.medium)
                    .sovra_surface(Surface::Matte, cx)
                    .child(format!("Source reference {}", index + 1))
                    .child(
                        Input::new(&evidence.reference)
                            .id(("decision-source", index))
                            .aria_label(format!("Source reference {}", index + 1))
                            .disabled(self.pending),
                    )
                    .child(format!("Cited passage {}", index + 1))
                    .child(
                        Textarea::new(&evidence.passage)
                            .accessibility_id(format!("decision-passage-{index}"))
                            .aria_label(format!("Cited passage {}", index + 1))
                            .disabled(self.pending),
                    )
                    .child(
                        design::button(("decision-remove-evidence", index), "Remove passage", cx)
                            .disabled(self.pending || editor.evidence.len() == 1)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if let Some(editor) = &mut view.editor {
                                    editor.evidence.remove(index);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        form = form.child(design::button("decision-add-evidence", "Add cited passage", cx).disabled(self.pending || editor.evidence.len() >= 16).on_click(cx.listener(|view, _, window, cx| {
            let item = EvidenceInput { reference: input("", window, cx), passage: textarea("", window, cx) };
            if let Some(editor) = &mut view.editor { editor.evidence.push(item); } cx.notify();
        }))).child("Hard stops — optional conditions your company requires before acceptance. Leave a condition unchecked until it has been met. Only these written checks block acceptance.");
        for (index, stop) in editor.stops.iter().enumerate() {
            form = form.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .child(format!("Hard stop {}", index + 1))
                    .child(
                        Input::new(&stop.condition)
                            .id(("decision-stop", index))
                            .aria_label(format!("Hard stop {}", index + 1))
                            .disabled(self.pending),
                    )
                    .child(
                        Checkbox::new(("decision-stop-cleared", index))
                            .cursor_pointer()
                            .label("This condition has been met")
                            .checked(stop.cleared)
                            .disabled(self.pending)
                            .on_click(cx.listener(move |view, checked, _, cx| {
                                if let Some(editor) = &mut view.editor {
                                    editor.stops[index].cleared = *checked;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        design::button(("decision-remove-stop", index), "Remove hard stop", cx)
                            .disabled(self.pending)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if let Some(editor) = &mut view.editor {
                                    editor.stops.remove(index);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        form = form.child(
            design::button("decision-add-stop", "Add hard stop", cx)
                .disabled(self.pending || editor.stops.len() >= 32)
                .on_click(cx.listener(|view, _, window, cx| {
                    let stop = stop_input("", false, window, cx);
                    if let Some(editor) = &mut view.editor {
                        editor.stops.push(stop);
                    }
                    cx.notify();
                })),
        );
        if let Some(chat) = &editor.chat {
            form = form.child(reading(
                "Original chat reply",
                &format!("{}\n{}", chat.reference, chat.passage),
                cx,
            ));
        }
        form.child(
            design::button(
                "decision-save",
                if self.pending {
                    "Saving…"
                } else {
                    "Save decision draft"
                },
                cx,
            )
            .primary()
            .disabled(self.pending)
            .on_click(cx.listener(|view, _, _, cx| view.save(cx))),
        )
    }

    fn detail(&self, record: &DecisionRecord, cx: &mut Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        let Ok(version) = record.current() else {
            return div().child("This record has no saved version.");
        };
        let status = record.summary().map_or("Could not read status", |summary| {
            summary.status(now()).label()
        });
        let acceptance_error = self.person(cx).map_or_else(
            || Some("Sign in to accept a decision.".into()),
            |by| {
                record
                    .can_accept(&by, now())
                    .err()
                    .map(|error| error.to_string())
            },
        );
        let edit = record.clone();
        let copy = record.clone();
        let mut detail = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.medium)
            .child(
                div()
                    .id("decision-status")
                    .aria_label(status)
                    .test_support()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child(status),
            )
            .child(version_view(version, cx))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::button("decision-edit", "Save a new version", cx)
                            .disabled(self.pending)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                if let Ok(current) = edit.current() {
                                    view.editor = Some(Editor::new(
                                        Some(edit.id.clone()),
                                        edit.revision,
                                        current.draft.clone(),
                                        window,
                                        cx,
                                    ));
                                    view.page = Page::Edit;
                                    view.confirmation = None;
                                    view.error = None;
                                    view.scroll.set_offset(gpui_kit::Point::default());
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        design::button("decision-copy", "Copy record and history", cx).on_click(
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.text(now())));
                            },
                        ),
                    )
                    .child(
                        design::button(
                            "decision-history",
                            if self.history {
                                "Hide earlier versions"
                            } else {
                                "Earlier versions"
                            },
                            cx,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.history = !view.history;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                design::button("decision-accept", "Accept as company position", cx)
                    .disabled(self.pending || acceptance_error.is_some())
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.confirmation = Some(Confirmation::Accept);
                        view.scroll.set_offset(gpui_kit::Point::default());
                        cx.notify();
                    })),
            );
        if let Some(message) = acceptance_error {
            detail = detail.child(message);
        }
        if version.ended.is_none() {
            detail = detail
                .child(text_field(
                    "Why is this decision no longer valid?",
                    "decision-end-reason",
                    &self.end_reason,
                    self.pending,
                    cx,
                ))
                .child(
                    design::button("decision-end", "End this decision", cx)
                        .disabled(self.pending)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.confirmation = Some(Confirmation::End);
                            view.scroll.set_offset(gpui_kit::Point::default());
                            cx.notify();
                        })),
                );
        }
        if self.history {
            for (index, earlier) in record.versions.iter().enumerate().rev().skip(1) {
                detail = detail.child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap(t.space.small)
                        .p(t.space.medium)
                        .sovra_surface(Surface::Matte, cx)
                        .child(format!(
                            "Earlier version {} — historical, not the current position",
                            index + 1
                        ))
                        .child(version_view(earlier, cx)),
                );
            }
            if record.versions.len() == 1 {
                detail = detail.child("There are no earlier versions.");
            }
        }
        detail
    }
}

fn reading(label: &str, value: &str, cx: &App) -> gpui_kit::Div {
    v_flex()
        .w_full()
        .min_w_0()
        .gap(design::tokens(cx).space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(label.to_owned()),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .child(value.to_owned()),
        )
}

fn version_view(version: &sovra_core::decisions::DecisionVersion, cx: &App) -> gpui_kit::Div {
    let draft = &version.draft;
    let mut view = v_flex()
        .w_full()
        .min_w_0()
        .gap(design::tokens(cx).space.medium)
        .child(reading("Question", &draft.question, cx))
        .child(reading("Decision", &draft.decision, cx))
        .child(reading("Our criteria", &draft.criteria, cx))
        .child(reading("Review on (UTC)", &draft.review_on, cx))
        .child(reading(
            "What would end this decision",
            &draft.ends_when,
            cx,
        ))
        .child(format!(
            "Drafted by {} · {}",
            version.drafted_by.name,
            sovra_core::decisions::decision_time(version.saved_at)
        ));
    for evidence in &draft.evidence {
        view = view.child(reading(&evidence.reference, &evidence.passage, cx));
    }
    for stop in &draft.hard_stops {
        view = view.child(format!(
            "{}: {}",
            if stop.cleared {
                "Condition met"
            } else {
                "Hard stop still blocks acceptance"
            },
            stop.condition
        ));
    }
    if let Some(chat) = &draft.chat {
        view = view.child(reading(
            "Original chat reply",
            &format!("{}\n{}", chat.reference, chat.passage),
            cx,
        ));
    }
    if let Some(accepted) = &version.accepted {
        view = view.child(format!(
            "Accepted by {} · {}",
            accepted.by.name,
            sovra_core::decisions::decision_time(accepted.at)
        ));
    }
    if let Some(ended) = &version.ended {
        view = view.child(format!(
            "Ended by {} · {}: {}",
            ended.by.name,
            sovra_core::decisions::decision_time(ended.at),
            ended.reason
        ));
    }
    view
}

fn input_field(
    label: &'static str,
    id: &'static str,
    state: &Entity<InputState>,
    disabled: bool,
    cx: &App,
) -> gpui_kit::Div {
    v_flex()
        .w_full()
        .min_w_0()
        .gap(design::tokens(cx).space.small)
        .child(label)
        .child(
            Input::new(state)
                .id(id)
                .aria_label(label)
                .disabled(disabled),
        )
}
fn text_field(
    label: &'static str,
    id: &'static str,
    state: &Entity<TextareaState>,
    disabled: bool,
    cx: &App,
) -> gpui_kit::Div {
    v_flex()
        .w_full()
        .min_w_0()
        .gap(design::tokens(cx).space.small)
        .child(label)
        .child(
            Textarea::new(state)
                .accessibility_id(id)
                .aria_label(label)
                .disabled(disabled),
        )
}

impl Render for DecisionsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.reset_reason) {
            self.end_reason
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        let t = design::tokens(cx);
        let book = self.book(cx);
        let body = match self.page {
            Page::List => self.list(&book, cx),
            Page::Edit => self.form(cx),
            Page::Detail => book.open.as_ref().map_or_else(
                || div().child("Open a decision to see it here."),
                |record| self.detail(record, cx),
            ),
        };
        let mut content = v_flex().w_full().min_w_0().max_w(t.layout.reading_width).mx_auto().gap(t.space.medium)
            .child(design::page_header("Decisions", "Keep the reasons, evidence and people behind a company decision.", cx))
            .child("Records stay on this computer and are shared by its local accounts. Acceptance records the signed-in account; name-only accounts are not proof of identity.");
        if !matches!(self.page, Page::List) {
            content = content.child(
                design::button("decision-back", "Back to decisions", cx)
                    .disabled(self.pending)
                    .on_click(cx.listener(|view, _, _, cx| view.back(cx))),
            );
        }
        if let Some(confirmation) = self.confirmation {
            let (message, label) = match confirmation {
                Confirmation::Discard => (
                    "Discard the unsaved changes to this decision?",
                    "Discard unsaved changes",
                ),
                Confirmation::Accept => (
                    "Accept this exact saved version as the company's position? Check the evidence and review date first.",
                    "Confirm acceptance",
                ),
                Confirmation::End => (
                    "Mark this decision as no longer valid? Its wording, acceptance and history will be kept.",
                    "Confirm end decision",
                ),
            };
            content = content.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .p(t.space.medium)
                    .sovra_surface(Surface::Matte, cx)
                    .child(message)
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(t.space.small)
                            .child(
                                design::button("decision-confirm", label, cx)
                                    .disabled(self.pending)
                                    .on_click(cx.listener(|view, _, _, cx| view.confirm(cx))),
                            )
                            .child(
                                design::button("decision-keep-editing", "Go back", cx)
                                    .disabled(self.pending)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.confirmation = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .id("decision-error")
                    .aria_label(error.clone())
                    .test_support()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        v_flex()
            .id("sovra-decisions-view")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p(t.space.large)
            .child(content.child(body))
    }
}
