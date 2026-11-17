//! Where finished models go besides Your models: a storage bucket the person
//! owns, and the Ollama on this computer. Two switches and one small form; the
//! secret key is typed into a masked field and only ever leaves in a command.

use super::{TrainingView, parts};
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use gpui_kit::component::{Disableable as _, Selectable as _, h_flex, input::Input, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::training::{
    BucketSettings, CopyState, LibraryModel, ModelOrigin, Outlet, RepeatLevel, TrainingCommand,
    ollama_refusal,
};

/// The bucket fields in the order they are shown: id, label, hint.
pub(super) const BUCKET_FIELDS: [(&str, &str, &str); 6] = [
    (
        "training-bucket-endpoint",
        "Storage address",
        "https://s3.eu-west-2.amazonaws.com",
    ),
    (
        "training-bucket-region",
        "Region",
        "us-east-1, or auto for Cloudflare R2",
    ),
    ("training-bucket-name", "Bucket", "my-models"),
    (
        "training-bucket-folder",
        "Folder inside it (optional)",
        "sovra",
    ),
    ("training-bucket-key-id", "Access key ID", "The key's ID"),
    (
        "training-bucket-secret",
        "Secret access key",
        "The secret key",
    ),
];

/// The bucket form's inputs and what it is doing.
pub(super) struct BucketForm {
    /// One input per field, in [`BUCKET_FIELDS`] order.
    pub(super) fields: Vec<gpui_kit::Entity<gpui_kit::component::input::InputState>>,
    /// The form is open over a saved bucket.
    pub(super) editing: bool,
    /// The form should show the saved bucket on the next frame.
    pub(super) fill: bool,
    /// The typed secret key should be wiped on the next frame.
    pub(super) clear_secret: bool,
}

impl BucketForm {
    pub(super) fn new(
        fields: Vec<gpui_kit::Entity<gpui_kit::component::input::InputState>>,
    ) -> Self {
        Self {
            fields,
            editing: false,
            fill: false,
            clear_secret: false,
        }
    }
}

/// One on/off choice on the card.
#[derive(Clone, Copy)]
struct Switch {
    id: &'static str,
    title: &'static str,
    detail: &'static str,
    on: bool,
    /// Whether it may be turned on now. It may always be turned off.
    allowed: bool,
    change: fn(bool) -> TrainingCommand,
}

/// What one copy of a model says on its row.
fn copy_words(outlet: Outlet, copy: Option<&CopyState>) -> Option<String> {
    let what = match outlet {
        Outlet::Bucket => "Backup",
        Outlet::Ollama => "Ollama",
    };
    Some(match copy? {
        CopyState::Working => match outlet {
            Outlet::Bucket => "Backing up to your bucket…".to_owned(),
            Outlet::Ollama => "Adding to Ollama…".to_owned(),
        },
        CopyState::Done { place, .. } => match outlet {
            Outlet::Bucket => format!("Backed up to {place}"),
            Outlet::Ollama => format!("In Ollama as {place}. Run it with: ollama run {place}"),
        },
        CopyState::Failed(why) => format!("{what} did not work: {why}"),
    })
}

impl TrainingView {
    /// The bucket typed into the form, as it would be saved.
    fn typed_bucket(&self, cx: &Context<Self>) -> (BucketSettings, Option<String>) {
        let field = |index: usize| -> String {
            self.bucket
                .fields
                .get(index)
                .map(|input| input.read(cx).value().to_string())
                .unwrap_or_default()
        };
        let secret = field(5);
        (
            BucketSettings {
                endpoint: field(0),
                region: field(1),
                bucket: field(2),
                folder: field(3),
                access_key_id: field(4),
            }
            .tidied(),
            (!secret.trim().is_empty()).then(|| secret.trim().to_owned()),
        )
    }

    fn save_bucket(&mut self, cx: &mut Context<Self>) {
        let (settings, secret) = self.typed_bucket(cx);
        self.bucket.editing = false;
        self.bucket.clear_secret = true;
        self.send(TrainingCommand::SaveBucket { settings, secret }, cx);
    }

    fn bucket_form(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        let mut form = v_flex().w_full().min_w_0().gap(t.space.small);
        for (index, (id, label, _)) in BUCKET_FIELDS.into_iter().enumerate() {
            let Some(input) = self.bucket.fields.get(index) else {
                continue;
            };
            form = form.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.hair)
                    .child(parts::caption(label, cx))
                    .child(
                        Input::new(input)
                            .id(id)
                            .aria_label(label)
                            .disabled(self.pending),
                    ),
            );
        }
        let kept = if self.state.outlets.bucket_key_saved {
            "Leave the secret key empty to keep the one already saved."
        } else {
            "The secret key is kept in this computer's keychain, never in a file."
        };
        let mut actions = h_flex().min_w_0().flex_wrap().gap(t.space.small).child(
            design::primary_button("training-bucket-save", "Save bucket", cx)
                .disabled(self.pending)
                .on_click(cx.listener(|view, _, _, cx| view.save_bucket(cx))),
        );
        if self.state.outlets.bucket.is_some() {
            actions = actions.child(
                design::button("training-bucket-cancel", "Cancel", cx).on_click(cx.listener(
                    |view, _, _, cx| {
                        view.bucket.editing = false;
                        cx.notify();
                    },
                )),
            );
        }
        form.child(parts::caption(kept, cx)).child(actions)
    }

    /// One on/off choice, said in words with one button to change it.
    fn outlet_switch(&self, switch: Switch, cx: &mut Context<Self>) -> gpui_kit::Div {
        let Switch {
            id,
            title,
            detail,
            on,
            allowed,
            change,
        } = switch;
        let t = design::tokens(cx);
        let state = if on { "On" } else { "Off" };
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .gap(t.space.small)
            .p(t.space.compact)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(t.space.hair)
                    .child(parts::body(title, cx))
                    .child(parts::caption(detail, cx)),
            )
            .child(
                design::button(id, if on { "Turn off" } else { "Turn on" }, cx)
                    .accessibility_label(format!(
                        "{title}: {}. Turn {}",
                        state.to_lowercase(),
                        if on { "off" } else { "on" }
                    ))
                    .selected(on)
                    .disabled(self.pending || (!on && !allowed))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.send(change(!on), cx);
                    })),
            )
    }

    pub(super) fn outlets_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let outlets = &self.state.outlets;
        let ready = outlets.bucket.is_some() && outlets.bucket_key_saved;
        let mut card = parts::card(self.compact(), cx)
            .child(parts::title("Send finished models", cx))
            .child(parts::caption(
                "Keep a copy of every model you finish in your own storage bucket, and run it in Ollama. Your text file never leaves, but a model can repeat parts of what it learned, so each model shows a privacy check. A model that repeats your text is sent only when you press its button.",
                cx,
            ));
        card = card.child(self.outlet_switch(
            Switch {
                id: "training-outlet-ollama",
                title: "Add finished models to Ollama",
                detail: "Each Small, Medium or Large model you finish here is added to Ollama on this computer, ready for: ollama run. Needs Ollama 0.40.15 or newer.",
                on: outlets.ollama_automatically,
                allowed: true,
                change: TrainingCommand::SetAutomaticOllama,
            },
            cx,
        ));
        card = card.child(self.outlet_switch(
            Switch {
                id: "training-outlet-backup",
                title: "Back up finished models",
                detail: "Each model you finish here is copied to your bucket with a version Ollama and llama.cpp can read, and a short description.",
                on: outlets.backup_automatically,
                allowed: ready,
                change: TrainingCommand::SetAutomaticBackup,
            },
            cx,
        ));
        let bucket = match (&outlets.bucket, self.bucket.editing) {
            (Some(bucket), false) => {
                let place = if bucket.folder.is_empty() {
                    bucket.bucket.clone()
                } else {
                    format!("{}/{}", bucket.bucket, bucket.folder)
                };
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .child(parts::status(
                        "training-bucket-saved",
                        format!("Your bucket: {place} at {}", bucket.endpoint),
                        cx,
                    ))
                    .child(
                        h_flex()
                            .min_w_0()
                            .flex_wrap()
                            .gap(t.space.small)
                            .child(
                                design::button("training-bucket-change", "Change bucket", cx)
                                    .disabled(self.pending)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.bucket.editing = true;
                                        view.bucket.fill = true;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                design::button("training-bucket-forget", "Forget bucket", cx)
                                    .disabled(self.pending)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.send(TrainingCommand::ForgetBucket, cx);
                                    })),
                            ),
                    )
            }
            _ => v_flex()
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .child(parts::body("Your storage bucket", cx))
                .child(parts::caption(
                    "Amazon S3, Cloudflare R2, Backblaze B2, Wasabi, MinIO and other services that work the same way. Give a key that may only write to this bucket.",
                    cx,
                ))
                .child(self.bucket_form(cx)),
        };
        card.child(bucket).id("training-outlets").test_support()
    }

    /// The copy lines and buttons a library model gets on its row.
    pub(super) fn model_outlets(
        &self,
        index: usize,
        model: &LibraryModel,
        cx: &mut Context<Self>,
    ) -> (Vec<gpui_kit::AnyElement>, Vec<gpui_kit::AnyElement>) {
        let outlets = &self.state.outlets;
        let id = model.card.id;
        let copies = outlets.copies(id);
        let mut lines = Vec::new();
        let privacy = match outlets.repeats(id) {
            Some(check) => Some((check.sentence(), check.level() != RepeatLevel::NotFound)),
            None if model.card.origin == ModelOrigin::TrainedHere => Some((
                "Privacy check: not measured for this model, which was made before Sovra checked. A small model can repeat parts of the text it learned.".to_owned(),
                false,
            )),
            None => None,
        };
        if let Some((text, repeats)) = privacy {
            let text = SharedString::from(text);
            lines.push(
                div()
                    .id(("training-model-privacy", index))
                    .test_support()
                    .role(if repeats { Role::Alert } else { Role::Status })
                    .aria_label(text.clone())
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .child(text)
                    .into_any_element(),
            );
        }
        for (outlet, name, copy) in [
            (
                Outlet::Bucket,
                "training-model-backup-state",
                copies.and_then(|copies| copies.bucket.as_ref()),
            ),
            (
                Outlet::Ollama,
                "training-model-ollama-state",
                copies.and_then(|copies| copies.ollama.as_ref()),
            ),
        ] {
            if let Some(text) = copy_words(outlet, copy) {
                let text = SharedString::from(text);
                let role = if matches!(copy, Some(CopyState::Failed(_))) {
                    Role::Alert
                } else {
                    Role::Status
                };
                lines.push(
                    div()
                        .id((name, index))
                        .test_support()
                        .role(role)
                        .aria_label(text.clone())
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Caption, cx)
                        .child(text)
                        .into_any_element(),
                );
            }
        }
        let mut buttons = Vec::new();
        if outlets.bucket.is_some() && outlets.bucket_key_saved {
            buttons.push(
                design::button(("training-model-backup", index), "Back up now", cx)
                    .accessibility_label(format!("Back up {} to your bucket", model.card.name))
                    .disabled(self.pending || outlets.working(id, Outlet::Bucket))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.send(TrainingCommand::BackUp(id), cx);
                    }))
                    .into_any_element(),
            );
        }
        if ollama_refusal(model.card.architecture).is_none() {
            buttons.push(
                design::button(("training-model-ollama", index), "Add to Ollama", cx)
                    .accessibility_label(format!("Add {} to Ollama", model.card.name))
                    .disabled(self.pending || outlets.working(id, Outlet::Ollama))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.send(TrainingCommand::AddToOllama(id), cx);
                    }))
                    .into_any_element(),
            );
        }
        (lines, buttons)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_is_described_in_plain_words() {
        assert_eq!(copy_words(Outlet::Bucket, None), None);
        assert_eq!(
            copy_words(Outlet::Bucket, Some(&CopyState::Working)).unwrap(),
            "Backing up to your bucket…"
        );
        let done = CopyState::Done {
            place: "sovra-notes-1".into(),
            at_unix_ms: 1,
        };
        assert_eq!(
            copy_words(Outlet::Ollama, Some(&done)).unwrap(),
            "In Ollama as sovra-notes-1. Run it with: ollama run sovra-notes-1"
        );
        assert_eq!(
            copy_words(
                Outlet::Bucket,
                Some(&CopyState::Failed("No network.".into()))
            )
            .unwrap(),
            "Backup did not work: No network."
        );
    }
}
