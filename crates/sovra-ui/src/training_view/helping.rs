//! Helping your group: what this computer is doing for someone else's training.

use super::{TrainingView, parts, words};
use crate::design;
use gpui_kit::component::{Disableable as _, h_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    TestSupportExt as _,
};
use sovra_core::training::{HelpingPhase, TrainingCommand};

impl TrainingView {
    pub(super) fn helping_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut card =
            parts::card(self.compact(), cx).child(parts::title("Helping your group", cx));
        if let Some(help) = &self.state.helping {
            let phase = match help.phase {
                HelpingPhase::Working => "Helping",
                HelpingPhase::Waiting => "Waiting to help",
                HelpingPhase::Stopping => "Stopping after this piece of work",
                HelpingPhase::Stopped => "Stopped helping",
                HelpingPhase::Finished => "That training has finished",
            };
            let rounds = words::count(help.accepted_rounds.into());
            let status = format!("{phase} {} · {rounds} rounds given", help.peer.name);
            card = card.child(parts::status("training-helper-status", status, cx));
            let progress = help.progress.as_ref();
            let what = match (&help.run, progress) {
                (Some(run), Some(progress)) => Some(format!(
                    "{}, a {} model: round {} of {}.",
                    run.name,
                    run.base.info().title,
                    words::count(progress.rounds.into()),
                    words::count(progress.total_rounds.into())
                )),
                (Some(run), None) => Some(format!(
                    "{}, a {} model: round {} of {}.",
                    run.name,
                    run.base.info().title,
                    words::count(run.rounds.into()),
                    words::count(run.total_rounds.into())
                )),
                (None, Some(progress)) if progress.total_rounds > 0 => Some(format!(
                    "Their training is at round {} of {}.",
                    words::count(progress.rounds.into()),
                    words::count(progress.total_rounds.into())
                )),
                (None, _) => None,
            };
            if let Some(what) = what {
                card = card.child(parts::body(what, cx));
            }
            if let Some(notice) = &help.notice {
                card = card.child(parts::caption(notice.clone(), cx));
            }
            card = card.child(
                h_flex().min_w_0().flex_wrap().child(
                    design::button("training-stop-help", "Stop helping", cx)
                        .disabled(
                            self.pending
                                || !help.phase.active()
                                || help.phase == HelpingPhase::Stopping,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send(TrainingCommand::StopHelping, cx);
                        })),
                ),
            );
        } else {
            card = card.child(parts::status(
                "training-helper-status",
                "This computer is not helping anyone's training right now.",
                cx,
            ));
        }
        card.child(parts::caption(
            "Whether this computer may help is your choice in Groups, under Help the group train. When it is on, this computer joins a group member's training by itself while it is idle. It only sends back what it learned. Your own text, chats and files are never sent.",
            cx,
        ))
        .id("training-helping")
        .test_support()
    }
}
