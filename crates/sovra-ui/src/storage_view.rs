//! The Online storage card on the Connections page: the linked places with
//! their status, a small file browser, and the form that adds or edits one.
//!
//! The runtime owns the list, the secrets and every request; this view folds
//! [`sovra_core::storage::StorageEvent`]s into [`StorageBook`] and sends
//! [`StorageCommand`]s.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputState},
    switch::Switch,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div,
};
use sovra_core::Command;
use sovra_core::storage::{
    AddressRule, PlaceStatus, PlaceView, StorageCommand, StorageEntry, StoragePlace,
    StorageProtocol, StorageSecret, StorageService, parent_folder,
};

use crate::RuntimeClient;
use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use crate::models_view::{ModelsView, caption};

/// The add-or-edit form. Inputs are made when it opens, with that service's
/// words as placeholders.
pub(crate) struct StorageForm {
    /// `None` while the person is choosing a service.
    service: Option<StorageService>,
    /// The place being changed; `None` adds a new one.
    editing: Option<String>,
    allow_saving: bool,
    /// Waiting for the runtime's answer to Save.
    pending: bool,
    name: Entity<InputState>,
    endpoint: Entity<InputState>,
    region: Entity<InputState>,
    bucket: Entity<InputState>,
    folder: Entity<InputState>,
    user: Entity<InputState>,
    secret: Entity<InputState>,
}

/// The services offered, grouped the way people think of them.
const SERVICE_GROUPS: [(&str, &[StorageService]); 3] = [
    (
        "Storage buckets",
        &[
            StorageService::AmazonS3,
            StorageService::CloudflareR2,
            StorageService::BackblazeB2,
            StorageService::Wasabi,
            StorageService::DigitalOceanSpaces,
            StorageService::GoogleCloudStorage,
            StorageService::Hetzner,
            StorageService::Scaleway,
            StorageService::Linode,
            StorageService::MinIo,
            StorageService::OtherS3,
        ],
    ),
    ("Microsoft Azure", &[StorageService::AzureBlob]),
    (
        "File servers",
        &[
            StorageService::Nextcloud,
            StorageService::OwnCloud,
            StorageService::OtherWebDav,
        ],
    ),
];

fn service_icon(service: StorageService) -> IconName {
    match service.protocol() {
        StorageProtocol::S3 => IconName::Database,
        StorageProtocol::AzureBlob => IconName::Cloud,
        StorageProtocol::WebDav => IconName::FolderOpen,
    }
}

/// "1.2 MB" for a size in bytes.
pub(crate) fn readable_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    // Shown to one decimal place, so the precision lost above 2^52 bytes is invisible.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a size shown to one decimal place"
    )]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn status_color(status: &PlaceStatus, cx: &Context<ModelsView>) -> Hsla {
    match status {
        PlaceStatus::Works => cx.theme().success,
        PlaceStatus::Problem(_) => cx.theme().danger,
        PlaceStatus::Unchecked | PlaceStatus::Checking => design::tokens(cx).text_secondary,
    }
}

fn labelled(
    id: &'static str,
    label: &'static str,
    input: &Entity<InputState>,
    cx: &Context<ModelsView>,
) -> AnyElement {
    let t = design::tokens(cx);
    v_flex()
        .prefer_width(t.layout.filter_width)
        .gap(t.space.hair)
        .child(caption(label, cx))
        .child(Input::new(input).id(id).aria_label(label).small())
        .into_any_element()
}

impl ModelsView {
    fn send_storage(&mut self, command: StorageCommand, cx: &mut Context<Self>) {
        self.storage.notice = None;
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Storage(command));
        cx.notify();
    }

    /// Opens the form: choosing a service for a new place, or filled in from `existing`.
    pub(crate) fn open_storage_form(
        &mut self,
        service: Option<StorageService>,
        existing: Option<&StoragePlace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = existing.cloned().or_else(|| service.map(StoragePlace::new));
        let service = existing.map(|place| place.service).or(service);
        let field =
            |value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
                let value = value.to_owned();
                let placeholder = placeholder.to_owned();
                cx.new(move |cx| {
                    let mut state = InputState::new(window, cx).placeholder(placeholder);
                    state.set_value(value, window, cx);
                    state
                })
            };
        let place = start.unwrap_or_default();
        let example = service.map_or("", StorageService::address_example);
        let user_label = service.map_or("", StorageService::user_label);
        let secret_label = service.map_or("", StorageService::secret_label);
        let endpoint_value = match service.map(StorageService::address) {
            // The address follows the region or account; leave it blank so it
            // keeps following them, unless the person typed their own.
            Some(AddressRule::FromRegion(_) | AddressRule::FromUser(_))
                if existing.is_none_or(|existing| {
                    service.and_then(|s| s.suggested_address(&existing.region, &existing.user))
                        == Some(existing.endpoint.clone())
                }) =>
            {
                String::new()
            }
            _ => place.endpoint.clone(),
        };
        let secret_placeholder = if existing.is_some() {
            "Leave empty to keep the saved one".to_owned()
        } else {
            format!(
                "Paste the {}",
                sovra_core::storage::in_sentence(secret_label)
            )
        };
        self.storage.refused = None;
        self.storage.saved = None;
        self.storage_form = Some(StorageForm {
            service,
            editing: existing.map(|place| place.id.clone()),
            allow_saving: place.allow_saving,
            pending: false,
            name: field(&place.name, "Family photos", window, cx),
            endpoint: field(&endpoint_value, example, window, cx),
            region: field(
                &place.region,
                service.map_or("", StorageService::default_region),
                window,
                cx,
            ),
            bucket: field(&place.bucket, "my-bucket", window, cx),
            folder: field(&place.folder, "Optional, such as sovra", window, cx),
            user: field(&place.user, user_label, window, cx),
            secret: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(secret_placeholder)
                    .masked(true)
            }),
        });
        cx.notify();
    }

    fn save_storage_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.storage_form.as_mut() else {
            return;
        };
        let Some(service) = form.service else {
            return;
        };
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let place = StoragePlace {
            id: form.editing.clone().unwrap_or_default(),
            name: text(&form.name),
            service,
            endpoint: text(&form.endpoint),
            region: text(&form.region),
            bucket: text(&form.bucket),
            folder: text(&form.folder),
            user: text(&form.user),
            allow_saving: form.allow_saving,
        };
        let secret = StorageSecret::new(text(&form.secret));
        form.pending = true;
        self.storage.refused = None;
        self.send_storage(StorageCommand::Save { place, secret }, cx);
    }

    /// Called by the shell after each storage event: a finished save closes the form.
    pub(crate) fn storage_changed(&mut self) {
        if let Some(form) = &mut self.storage_form {
            if self.storage.saved.take().is_some() {
                self.storage_form = None;
            } else if self.storage.refused.is_some() {
                form.pending = false;
            }
        }
    }

    pub(crate) fn render_storage_section(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = design::tokens(cx);
        let mut body = v_flex()
            .id("storage-section")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.medium);
        body = body.child(
            h_flex()
                .w_full()
                .min_w_0()
                .gap(t.space.medium)
                .items_center()
                .child(crate::skill_library::tile(
                    IconName::Cloud,
                    t.tiles.categories[3],
                    cx,
                ))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(t.space.hair)
                        .child(
                            div()
                                .id("storage-title")
                                .test_support()
                                .sovra_text(TextRole::PageTitle, cx)
                                .child("Online storage"),
                        )
                        .child(caption(
                            "Link a bucket or file server and Sovra can find, read and (if you allow it) save files there. Your sign-in stays in this computer's keychain.",
                            cx,
                        )),
                ),
        );
        if let Some(notice) = self.storage.notice.clone() {
            body = body.child(
                div()
                    .id("storage-notice")
                    .test_support()
                    .aria_label(notice.clone())
                    .w_full()
                    .p(t.space.small)
                    .sovra_surface(Surface::Subtle, cx)
                    .sovra_text(TextRole::Caption, cx)
                    .child(notice),
            );
        }
        if self.storage_form.is_some() {
            return body.child(self.render_storage_form(cx)).into_any_element();
        }
        if !self.storage.loaded {
            body = body.child(caption("Looking for your storage places…", cx));
        } else if self.storage.places.is_empty() {
            body = body.child(
                h_flex()
                    .id("storage-empty")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .p(t.space.medium)
                    .gap(t.space.medium)
                    .items_center()
                    .sovra_surface(Surface::Subtle, cx)
                    .child(
                        Icon::new(IconName::Cloud)
                            .size(t.layout.icon_medium)
                            .text_color(t.text_secondary),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(t.space.hair)
                            .child(
                                div()
                                    .sovra_text(TextRole::SectionTitle, cx)
                                    .child("No storage linked yet"),
                            )
                            .child(caption("Buckets, Azure, Nextcloud or a home NAS.", cx)),
                    ),
            );
        } else {
            let places = self.storage.places.clone();
            for (idx, view) in places.iter().enumerate() {
                body = body.child(self.render_place_row(idx, view, cx));
            }
        }
        if self.storage.browsing.is_some() {
            body = body.child(self.render_browser(cx));
        }
        body.child(
            h_flex().w_full().justify_start().child(
                design::primary_button("storage-add", "Add storage", cx)
                    .icon(IconName::Plus)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.storage_remove_confirm = None;
                        view.open_storage_form(None, None, window, cx);
                    })),
            ),
        )
        .into_any_element()
    }

    fn render_place_row(&self, idx: usize, view: &PlaceView, cx: &mut Context<Self>) -> AnyElement {
        let t = design::tokens(cx);
        let place = &view.place;
        let status = if view.has_secret {
            self.storage.status_of(&place.id).clone()
        } else {
            PlaceStatus::Problem(
                "Sovra has no sign-in for this place. Press Edit and enter it again.".into(),
            )
        };
        let color = status_color(&status, cx);
        let status_icon = match &status {
            PlaceStatus::Works => IconName::CircleCheck,
            PlaceStatus::Problem(_) => IconName::CircleAlert,
            PlaceStatus::Checking => IconName::LoaderCircle,
            PlaceStatus::Unchecked => IconName::Circle,
        };
        let id = place.id.clone();
        let confirming = self.storage_remove_confirm.as_deref() == Some(place.id.as_str());
        let mut row = v_flex()
            .id(("storage-place", idx))
            .test_support()
            .w_full()
            .min_w_0()
            .p(t.space.medium)
            .gap(t.space.small)
            .sovra_surface(Surface::Matte, cx)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.medium)
                    .items_center()
                    .child(crate::skill_library::tile(
                        service_icon(place.service),
                        t.tiles.categories[3],
                        cx,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(t.space.hair)
                            .child(
                                div()
                                    .id(("storage-place-name", idx))
                                    .test_support()
                                    .aria_label(place.name.clone())
                                    .min_w_0()
                                    .truncate()
                                    .sovra_text(TextRole::SectionTitle, cx)
                                    .child(place.name.clone()),
                            )
                            .child(caption(place.summary(), cx).min_w_0().truncate()),
                    )
                    .child(
                        h_flex()
                            .id(("storage-place-status", idx))
                            .test_support()
                            .aria_label(status.label().to_owned())
                            .flex_shrink_0()
                            .gap(t.space.compact)
                            .items_center()
                            .text_color(color)
                            .child(Icon::new(status_icon).size(t.layout.icon_small))
                            .child(
                                div()
                                    .sovra_text(TextRole::Caption, cx)
                                    .child(status.label().to_owned()),
                            ),
                    ),
            );
        if let PlaceStatus::Problem(message) = &status {
            row = row.child(
                caption(message.clone(), cx)
                    .id(("storage-place-problem", idx))
                    .test_support()
                    .text_color(cx.theme().danger),
            );
        }
        let saving = if place.allow_saving {
            "Sovra may save files here"
        } else {
            "Read only: Sovra will not save files here"
        };
        let (browse_id, check_id, edit_id) = (id.clone(), id.clone(), id.clone());
        row.child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(t.space.small)
                .child(caption(saving, cx).min_w_0())
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.compact)
                        .child(
                            design::button(("storage-browse", idx), "Browse files", cx)
                                .small()
                                .icon(IconName::FolderOpen)
                                .disabled(!view.has_secret)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.storage.browse(&browse_id, "");
                                    view.send_storage(
                                        StorageCommand::Browse {
                                            id: browse_id.clone(),
                                            folder: String::new(),
                                        },
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            design::button(("storage-check", idx), "Check", cx)
                                .small()
                                .ghost()
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.send_storage(
                                        StorageCommand::Check {
                                            id: check_id.clone(),
                                        },
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            design::button(("storage-edit", idx), "Edit", cx)
                                .small()
                                .ghost()
                                .icon(IconName::Pencil)
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    let existing =
                                        view.storage.place(&edit_id).map(|view| view.place.clone());
                                    view.open_storage_form(None, existing.as_ref(), window, cx);
                                })),
                        )
                        .child(
                            design::button(
                                ("storage-remove", idx),
                                if confirming {
                                    "Press again to unlink"
                                } else {
                                    "Unlink"
                                },
                                cx,
                            )
                            .small()
                            .ghost()
                            .icon(IconName::Trash)
                            .tooltip("Sovra forgets this place. The files in it are not touched.")
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    if view.storage_remove_confirm.as_deref() == Some(id.as_str()) {
                                        view.storage_remove_confirm = None;
                                        view.send_storage(
                                            StorageCommand::Remove { id: id.clone() },
                                            cx,
                                        );
                                    } else {
                                        view.storage_remove_confirm = Some(id.clone());
                                        cx.notify();
                                    }
                                },
                            )),
                        ),
                ),
        )
        .into_any_element()
    }

    fn render_browser(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = design::tokens(cx);
        let Some(browsing) = self.storage.browsing.clone() else {
            return div().into_any_element();
        };
        let name = self
            .storage
            .place(&browsing.id)
            .map_or_else(String::new, |view| view.place.name.clone());
        let location = if browsing.folder.is_empty() {
            name
        } else {
            format!("{name} / {}", browsing.folder.replace('/', " / "))
        };
        let id = browsing.id.clone();
        let parent = parent_folder(&browsing.folder);
        let mut panel = v_flex()
            .id("storage-browser")
            .test_support()
            .w_full()
            .min_w_0()
            .p(t.space.medium)
            .gap(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(t.space.small)
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap(t.space.compact)
                            .items_center()
                            .child(Icon::new(IconName::FolderOpen).size(t.layout.icon_small))
                            .child(
                                div()
                                    .id("storage-browser-location")
                                    .test_support()
                                    .aria_label(location.clone())
                                    .min_w_0()
                                    .truncate()
                                    .sovra_text(TextRole::SectionTitle, cx)
                                    .child(location),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(t.space.compact)
                            .children((!browsing.folder.is_empty()).then(|| {
                                let id = id.clone();
                                design::button("storage-browser-up", "Up", cx)
                                    .small()
                                    .ghost()
                                    .icon(IconName::ArrowUp)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.storage.browse(&id, &parent);
                                        view.send_storage(
                                            StorageCommand::Browse {
                                                id: id.clone(),
                                                folder: parent.clone(),
                                            },
                                            cx,
                                        );
                                    }))
                            }))
                            .child(
                                design::button("storage-browser-close", "Close", cx)
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.storage.browsing = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        match &browsing.listing {
            None => panel = panel.child(caption("Opening…", cx)),
            Some(Err(message)) => {
                panel = panel.child(
                    caption(message.clone(), cx)
                        .id("storage-browser-error")
                        .test_support()
                        .text_color(cx.theme().danger),
                );
            }
            Some(Ok(listing)) if listing.entries.is_empty() => {
                panel = panel.child(caption("This folder is empty.", cx));
            }
            Some(Ok(listing)) => {
                for (idx, entry) in listing.entries.iter().enumerate() {
                    panel = panel.child(Self::render_entry(idx, &browsing.id, entry, cx));
                }
                if listing.more {
                    panel = panel.child(caption(
                        "There are more files here than Sovra shows at once. Ask Sovra in chat to look for a particular one.",
                        cx,
                    ));
                }
            }
        }
        panel.into_any_element()
    }

    fn render_entry(
        idx: usize,
        id: &str,
        entry: &StorageEntry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = design::tokens(cx);
        if entry.folder {
            let id = id.to_owned();
            let folder = entry.path.clone();
            let hover = t.subtle;
            // The same row shape as a file, so names line up; the whole row opens it.
            return h_flex()
                .id(("storage-entry", idx))
                .test_support()
                .role(gpui_kit::Role::Button)
                .aria_label(format!("Open folder {}", entry.name))
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .px(t.space.small)
                .py(t.space.compact)
                .items_center()
                .rounded(t.radius.input)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .child(
                    Icon::new(IconName::FolderOpen)
                        .size(t.layout.icon_small)
                        .text_color(cx.theme().link),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .sovra_text(TextRole::Body, cx)
                        .child(entry.name.clone()),
                )
                .child(
                    Icon::new(IconName::ChevronRight)
                        .size(t.layout.icon_small)
                        .text_color(t.text_secondary),
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.storage.browse(&id, &folder);
                    view.send_storage(
                        StorageCommand::Browse {
                            id: id.clone(),
                            folder: folder.clone(),
                        },
                        cx,
                    );
                }))
                .into_any_element();
        }
        let details = [
            entry.size.map(readable_size),
            entry.modified.as_ref().map(|text| {
                // "2026-11-01T10:00:00.000Z" reads better as its date.
                text.get(..10)
                    .filter(|d| d.as_bytes().get(4) == Some(&b'-'))
                    .map_or_else(|| text.clone(), str::to_owned)
            }),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        h_flex()
            .id(("storage-entry", idx))
            .test_support()
            .aria_label(entry.name.clone())
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .px(t.space.small)
            .py(t.space.compact)
            .items_center()
            .child(
                Icon::new(IconName::File)
                    .size(t.layout.icon_small)
                    .text_color(t.text_secondary),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .sovra_text(TextRole::Body, cx)
                    .child(entry.name.clone()),
            )
            .child(caption(details, cx).flex_shrink_0())
            .into_any_element()
    }

    fn render_storage_form(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = design::tokens(cx);
        let Some(form) = self.storage_form.as_ref() else {
            return div().into_any_element();
        };
        let Some(service) = form.service else {
            return self.render_service_choice(cx);
        };
        let editing = form.editing.is_some();
        let pending = form.pending;
        let allow_saving = form.allow_saving;
        let value = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let typed_endpoint = value(&form.endpoint);
        let address = if typed_endpoint.trim().is_empty() {
            service.suggested_address(&value(&form.region), &value(&form.user))
        } else {
            Some(typed_endpoint)
        };
        let mut fields = h_flex().w_full().min_w_0().flex_wrap().gap(t.space.medium);
        fields = fields.child(labelled(
            "storage-field-name",
            "A name for this place",
            &form.name,
            cx,
        ));
        if service.has_region() {
            fields = fields.child(labelled("storage-field-region", "Region", &form.region, cx));
        }
        if let Some(label) = service.bucket_label() {
            fields = fields.child(labelled("storage-field-bucket", label, &form.bucket, cx));
        }
        fields = fields
            .child(labelled(
                "storage-field-user",
                service.user_label(),
                &form.user,
                cx,
            ))
            .child(labelled(
                "storage-field-secret",
                service.secret_label(),
                &form.secret,
                cx,
            ))
            .child(labelled(
                "storage-field-endpoint",
                match service.address() {
                    AddressRule::Typed(_) => "Storage address",
                    AddressRule::Fixed(_)
                    | AddressRule::FromRegion(_)
                    | AddressRule::FromUser(_) => "Storage address (filled in for you)",
                },
                &form.endpoint,
                cx,
            ))
            .child(labelled(
                "storage-field-folder",
                "Only this folder (optional)",
                &form.folder,
                cx,
            ));
        let mut panel = v_flex()
            .id("storage-form")
            .test_support()
            .w_full()
            .min_w_0()
            .p(t.space.medium)
            .gap(t.space.medium)
            .sovra_surface(Surface::Matte, cx)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .items_center()
                    .child(crate::skill_library::tile(
                        service_icon(service),
                        t.tiles.categories[3],
                        cx,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .id("storage-form-title")
                                    .test_support()
                                    .aria_label(service.label())
                                    .sovra_text(TextRole::SectionTitle, cx)
                                    .child(service.label()),
                            )
                            .child(caption(service.sentence(), cx)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .p(t.space.small)
                    .items_start()
                    .sovra_surface(Surface::Subtle, cx)
                    .child(
                        Icon::new(IconName::Info)
                            .size(t.layout.icon_small)
                            .text_color(cx.theme().link),
                    )
                    .child(
                        caption(service.keys_help(), cx)
                            .id("storage-keys-help")
                            .test_support()
                            .flex_1()
                            .min_w_0(),
                    ),
            )
            .child(fields);
        if let Some(address) = address.filter(|address| !address.trim().is_empty()) {
            panel = panel.child(
                caption(format!("Sovra will connect to {}", address.trim()), cx)
                    .id("storage-address-preview")
                    .test_support(),
            );
        }
        panel = panel.child(
            Switch::new("storage-allow-saving")
                .checked(allow_saving)
                .label("Let Sovra save files here when I ask")
                .on_click(cx.listener(|view, checked: &bool, _, cx| {
                    if let Some(form) = view.storage_form.as_mut() {
                        form.allow_saving = *checked;
                    }
                    cx.notify();
                })),
        );
        if let Some(refused) = self.storage.refused.clone() {
            panel = panel.child(
                caption(refused.clone(), cx)
                    .id("storage-form-error")
                    .test_support()
                    .aria_label(refused)
                    .text_color(cx.theme().danger),
            );
        }
        panel
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::primary_button(
                            "storage-save",
                            if pending {
                                "Checking…"
                            } else if editing {
                                "Check and save"
                            } else {
                                "Check and link"
                            },
                            cx,
                        )
                        .icon(IconName::Check)
                        .disabled(pending)
                        .on_click(cx.listener(|view, _, _, cx| view.save_storage_form(cx))),
                    )
                    .children((!editing).then(|| {
                        design::button("storage-form-other", "Choose another service", cx)
                            .ghost()
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.open_storage_form(None, None, window, cx);
                            }))
                    }))
                    .child(
                        design::button("storage-form-cancel", "Cancel", cx)
                            .ghost()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.storage_form = None;
                                view.storage.refused = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_service_choice(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = design::tokens(cx);
        let columns = crate::skill_library::columns(self.width_class);
        let mut panel = v_flex()
            .id("storage-choose-service")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.medium)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Where are your files?"),
            );
        for (title, services) in SERVICE_GROUPS {
            let cells = services
                .iter()
                .map(|service| {
                    let service = *service;
                    let hover = t.subtle;
                    // A card rather than a button: long names and their
                    // sentence wrap instead of being cut off.
                    h_flex()
                        .id(SharedString::from(format!(
                            "storage-service-{}",
                            service.slug()
                        )))
                        .test_support()
                        .role(gpui_kit::Role::Button)
                        .aria_label(format!("{}: {}", service.label(), service.sentence()))
                        .size_full()
                        .min_w_0()
                        .p(t.space.small)
                        .gap(t.space.small)
                        .items_start()
                        .cursor_pointer()
                        .sovra_surface(Surface::Matte, cx)
                        .hover(move |style| style.bg(hover))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.open_storage_form(Some(service), None, window, cx);
                        }))
                        .child(crate::skill_library::tile(
                            service_icon(service),
                            t.tiles.categories[3],
                            cx,
                        ))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(t.space.hair)
                                .child(
                                    div()
                                        .min_w_0()
                                        .whitespace_normal()
                                        .sovra_text(TextRole::Body, cx)
                                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                        .child(service.label()),
                                )
                                .child(caption(service.sentence(), cx).whitespace_normal()),
                        )
                        .into_any_element()
                })
                .collect();
            panel = panel
                .child(caption(title, cx).text_color(t.text_secondary))
                .child(crate::skill_library::grid(cells, columns, cx));
        }
        panel
            .child(
                design::button("storage-form-cancel", "Cancel", cx)
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.storage_form = None;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}
