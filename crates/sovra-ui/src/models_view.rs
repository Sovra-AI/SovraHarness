//! Native model library: virtualized discovery, composable filters and model details.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, Theme, ThemeMode,
    button::ButtonVariants as _,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    searchable_list::{SearchableListItem, SearchableVec},
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollStrategy, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, TestSupportExt as _, UniformListScrollHandle, Window, div,
    uniform_list,
};
use sovra_core::{
    Command,
    models::{
        Appearance, CatalogFacets, CatalogScope, ModelCommand, ModelEvent, ModelFilter, ModelKey,
        ModelKind, ModelSort, ModelsState, PriceFilter, ProviderKind, SetupStage,
    },
};

use crate::model_connection_editor::{ConnectionForm, ManualModelForm};
use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, TextRole},
};

#[derive(Debug, Clone)]
pub(crate) struct Choice {
    pub(crate) value: String,
    pub(crate) title: String,
}
impl Choice {
    pub(crate) fn new(value: &str, title: &str) -> Self {
        Self {
            value: value.into(),
            title: title.into(),
        }
    }
}
impl SearchableListItem for Choice {
    type Value = String;
    fn title(&self) -> SharedString {
        self.title.clone().into()
    }
    fn value(&self) -> &String {
        &self.value
    }
}
pub(crate) type Selector = SelectState<SearchableVec<Choice>>;

/// "Show" picker value for every model type at once.
const ALL_KINDS: &str = "all";

pub(crate) struct GoToChat;
impl gpui_kit::EventEmitter<GoToChat> for ModelsView {}

pub(crate) struct StartSystemTour;
impl gpui_kit::EventEmitter<StartSystemTour> for ModelsView {}

/// The shell opens the Connections tab. `keep_editor` leaves a form the view already prepared.
#[derive(Clone, Copy)]
pub(crate) struct OpenConnections {
    pub(crate) kind: Option<sovra_core::CatalogueKind>,
    pub(crate) keep_editor: bool,
}
impl gpui_kit::EventEmitter<OpenConnections> for ModelsView {}

/// Leave Connections and open the on-this-computer AI setup in Models.
pub(crate) struct ShowLocalAi;
impl gpui_kit::EventEmitter<ShowLocalAi> for ModelsView {}

/// Leave Connections and show the model home.
pub(crate) struct ShowModels;
impl gpui_kit::EventEmitter<ShowModels> for ModelsView {}

/// Settings points to the Groups tab.
pub(crate) struct OpenGroups;
impl gpui_kit::EventEmitter<OpenGroups> for ModelsView {}

#[allow(
    clippy::struct_excessive_bools,
    reason = "independent view flags; a state enum would be artificial"
)]
pub struct ModelsView {
    pub(crate) state: ModelsState,
    pub(crate) home_open: bool,
    /// Measured from this pane by `design::width_probe`; starts single-pane.
    pub(crate) width_class: design::WidthClass,
    pub(crate) pending_activation: Option<ModelKey>,
    /// The selected-model card is showing the loaded models you can switch to.
    pub(crate) changing_active: bool,
    /// Filters that change list by the name, price, or plain "good for" sentence.
    pub(crate) change_search: Entity<InputState>,
    /// The open key form was reached from a setup step, so saving it moves the user
    /// on to the next step. Keys managed from Settings stay on their form.
    pub(crate) setup_flow: bool,
    pub(crate) choosing_purpose: Option<sovra_core::files::ModelPurpose>,
    pub(crate) filter: ModelFilter,
    rows: Vec<ModelKey>,
    pub(crate) selected: Option<ModelKey>,
    search: Entity<InputState>,
    author: Entity<Selector>,
    connection: Entity<Selector>,
    input_modality: Entity<Selector>,
    kind: Entity<Selector>,
    context: Entity<Selector>,
    price: Entity<Selector>,
    sort: Entity<Selector>,
    facets: CatalogFacets,
    pub(crate) controls_dirty: bool,
    filters_open: bool,
    pub(crate) download_open: bool,
    pub(crate) connections_open: bool,
    pub(crate) settings_open: bool,
    /// The Connections tab is showing this view.
    pub(crate) connections_page: bool,
    /// The Groups tab is showing this view (it owns the group state and form).
    pub(crate) groups_page: bool,
    /// The card the person opened. `None` is the first-screen catalogue.
    pub(crate) open_kind: Option<sovra_core::CatalogueKind>,
    /// Google status for the catalogue. The shell copies each Google event here.
    pub(crate) google: sovra_core::google::GoogleConnection,
    pub(crate) connection_form: Option<ConnectionForm>,
    pub(crate) manual_form: Option<ManualModelForm>,
    pub(crate) form_subscriptions: Vec<Subscription>,
    pub(crate) pending_save: Option<String>,
    pub(crate) saved_form_refresh: Option<String>,
    pub(crate) pending_context_sync: Option<String>,
    pub(crate) last_saved_previews: std::collections::BTreeMap<String, String>,
    pub(crate) always_context_input: Entity<InputState>,
    pub(crate) delete_confirmation: Option<String>,
    pub(crate) download_name: Entity<InputState>,
    pub(crate) download_connection: Entity<Selector>,
    pub(crate) compare: Vec<ModelKey>,
    pub(crate) compare_open: bool,
    pub(crate) connections: sovra_core::connections::ConnectionsState,
    pub(crate) whatsapp_form: crate::connections_view::WhatsAppForm,
    pub(crate) development: sovra_core::development::DevelopmentState,
    /// Terminal settings for the Settings page (host and saved rules only).
    pub(crate) terminal: sovra_core::terminal::TerminalState,
    /// The Google card on the Connections tab. The shell owns the view.
    pub(crate) google_card: Option<Entity<crate::google_card::GoogleCard>>,
    pub(crate) services: sovra_core::mcp::McpState,
    pub(crate) service_name: Entity<InputState>,
    pub(crate) service_address: Entity<InputState>,
    pub(crate) service_secret: Entity<InputState>,
    pub(crate) service_client_id: Entity<InputState>,
    pub(crate) service_client_secret: Entity<InputState>,
    /// Form state for the connected services screen.
    pub(crate) services_screen: crate::service_connect_view::ServicesScreen,
    /// The Groups tab: this computer's group, folded from runtime events.
    pub(crate) groups: sovra_core::group::GroupsView,
    pub(crate) groups_form: crate::groups_view::GroupsForm,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ModelsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelsView")
            .field("models", &self.rows.len())
            .finish_non_exhaustive()
    }
}

pub(crate) fn selector(
    choices: Vec<Choice>,
    window: &mut Window,
    cx: &mut Context<ModelsView>,
) -> Entity<Selector> {
    cx.new(|cx| SelectState::new(SearchableVec::new(choices), None, window, cx).searchable(true))
}

impl ModelsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search by name or maker…"));
        let change_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Name, price, or what it is good for")
        });
        let author = selector(vec![Choice::new("", "Any maker")], window, cx);
        let connection = selector(vec![Choice::new("", "Any service")], window, cx);
        let input_modality = selector(vec![Choice::new("", "Reads anything")], window, cx);
        let mut kinds: Vec<_> = ModelKind::ALL
            .into_iter()
            .map(|kind| Choice::new(kind.value(), kind.label()))
            .collect();
        kinds.push(Choice::new(ALL_KINDS, "Every type"));
        let kind = selector(kinds, window, cx);
        kind.update(cx, |select, cx| {
            select.set_selected_value(&ModelKind::Chat.value().to_owned(), window, cx);
        });
        let context = selector(
            choices(&[
                ("0", "Any memory size"),
                ("32000", "Long chats (32K+)"),
                ("128000", "Long documents (128K+)"),
                ("200000", "Very long documents (200K+)"),
                ("1000000", "Whole books (1M+)"),
            ]),
            window,
            cx,
        );
        let price = selector(
            choices(&[("any", "Any price"), ("free", "Free"), ("paid", "Paid")]),
            window,
            cx,
        );
        let sort = selector(
            choices(&[
                ("newest", "Newest first"),
                ("name", "Name A–Z"),
                ("context", "Biggest memory"),
                ("input", "Cheapest to read your messages"),
                ("output", "Cheapest to write replies"),
            ]),
            window,
            cx,
        );
        let download_connection = selector(Vec::new(), window, cx);
        let download_name = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Model name from ollama.com, e.g. qwen3:8b")
        });
        let always_context_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                "e.g. Explain things simply without technical jargon. Use short bullet points.",
            )
        });
        let whatsapp_form = crate::connections_view::WhatsAppForm::new(window, cx);
        let service_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("Name, if you want one"));
        let service_address = cx.new(|cx| InputState::new(window, cx).placeholder("https://"));
        let service_secret = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Only if the service gave you one")
                .masked(true)
        });
        let service_client_id =
            cx.new(|cx| InputState::new(window, cx).placeholder("Name from the service"));
        let service_client_secret = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password from the service, if it gave you one")
                .masked(true)
        });
        let services_screen = crate::service_connect_view::ServicesScreen::new(window, cx);
        let groups_form = crate::groups_view::GroupsForm::new(window, cx);
        let mut subscriptions = vec![
            cx.subscribe_in(&search, window, |view, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    view.read_filters(cx);
                }
            }),
            cx.subscribe_in(&change_search, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &service_address,
                window,
                |_, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                },
            ),
        ];
        // Search filters as you type; the program field enables its button.
        for live in services_screen.live_inputs() {
            subscriptions.push(cx.subscribe_in(
                &live,
                window,
                |_, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                },
            ));
        }
        for control in [
            &author,
            &connection,
            &input_modality,
            &kind,
            &context,
            &price,
            &sort,
        ] {
            subscriptions.push(cx.subscribe_in(
                control,
                window,
                |view, _, _: &SelectEvent<SearchableVec<Choice>>, _, cx| view.read_filters(cx),
            ));
        }
        Self {
            state: ModelsState::default(),
            home_open: true,
            width_class: design::WidthClass::Compact,
            pending_activation: None,
            changing_active: false,
            change_search,
            setup_flow: false,
            choosing_purpose: None,
            filter: ModelFilter {
                kind: Some(ModelKind::Chat),
                sort: ModelSort::Newest,
                ..ModelFilter::default()
            },
            rows: Vec::new(),
            selected: None,
            search,
            author,
            connection,
            input_modality,
            kind,
            context,
            price,
            sort,
            facets: CatalogFacets::default(),
            controls_dirty: true,
            filters_open: false,
            download_open: false,
            connections_open: false,
            settings_open: false,
            connections_page: false,
            groups_page: false,
            open_kind: None,
            google: sovra_core::google::GoogleConnection::default(),
            connection_form: None,
            manual_form: None,
            form_subscriptions: Vec::new(),
            pending_save: None,
            saved_form_refresh: None,
            pending_context_sync: None,
            last_saved_previews: std::collections::BTreeMap::new(),
            always_context_input,
            delete_confirmation: None,
            download_name,
            download_connection,
            compare: Vec::new(),
            compare_open: false,
            connections: sovra_core::connections::ConnectionsState::default(),
            whatsapp_form,
            development: sovra_core::development::DevelopmentState::default(),
            terminal: sovra_core::terminal::TerminalState::default(),
            google_card: None,
            services: sovra_core::mcp::McpState::default(),
            service_name,
            service_address,
            service_secret,
            service_client_id,
            service_client_secret,
            services_screen,
            groups: sovra_core::group::GroupsView::default(),
            groups_form,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn state(&self) -> &ModelsState {
        &self.state
    }
    pub fn visible_models(&self) -> &[ModelKey] {
        &self.rows
    }

    pub(crate) fn active_key_preview(
        &self,
        connection: &sovra_core::models::ModelConnection,
    ) -> Option<String> {
        if !connection.key_preview.is_empty() {
            Some(connection.key_preview.clone())
        } else if connection.has_credential {
            self.last_saved_previews
                .get(&connection.id)
                .cloned()
                .or_else(|| connection.active_key_display())
        } else if connection.environment_credential {
            connection.active_key_display()
        } else {
            None
        }
    }

    pub(crate) fn apply(&mut self, event: &ModelEvent, cx: &mut Context<Self>) {
        // Captured before apply, because a confirmed switch replaces the active model.
        let was_ready = self.state.has_active_llm();
        self.state.apply(event);
        if let (Some(purpose), Some(key)) =
            (self.choosing_purpose, self.pending_activation.as_ref())
            && matches!(event, ModelEvent::PreferencesChanged(preferences) if preferences.purposes.get(&purpose) == Some(key))
        {
            self.pending_activation = None;
            self.choosing_purpose = None;
            self.home_open = true;
            self.selected = None;
            cx.emit(GoToChat);
        }

        if self.choosing_purpose.is_none()
            && self.pending_activation.is_some()
            && matches!(event, ModelEvent::PreferencesChanged(preferences)
                if preferences.active.as_ref() == self.pending_activation.as_ref())
        {
            // The first choice finishes setup and opens chat. A later switch stays
            // on the selected-model card so the new name is visible there.
            self.pending_activation = None;
            self.changing_active = false;
            self.home_open = true;
            self.selected = None;
            if !was_ready {
                cx.emit(GoToChat);
            }
        }
        if matches!(event, ModelEvent::Error(_)) {
            self.pending_activation = None;
        }
        if let ModelEvent::ConnectionSaved { id } = event
            && self.pending_save.as_ref() == Some(id)
        {
            let stage = self.state.setup_stage();
            if self.choosing_purpose.is_some() {
                self.settings_open = false;
                self.connections_open = false;
                self.connection_form = None;
                self.form_subscriptions.clear();
                self.saved_form_refresh = None;
            } else if self.setup_flow
                && matches!(
                    stage,
                    SetupStage::Ready { .. } | SetupStage::NeedsModelSelection { .. }
                )
            {
                // A key saved during setup leads straight on: to the chat when a model
                // was already chosen, otherwise to picking one.
                self.connection_form = None;
                self.form_subscriptions.clear();
                self.saved_form_refresh = None;
                self.setup_flow = false;
                self.connections_page = false;
                self.open_kind = None;
                self.open_model_home(cx);
                if matches!(stage, SetupStage::Ready { .. }) {
                    cx.emit(GoToChat);
                } else {
                    cx.emit(ShowModels);
                }
            } else if (self.settings_open || self.connections_page)
                && self.connection_form.is_some()
            {
                // Stay on the open card and show the saved key without the secret.
                self.saved_form_refresh = Some(id.clone());
            } else {
                self.connection_form = None;
                self.form_subscriptions.clear();
            }
            self.manual_form = None;
            self.pending_save = None;
        }
        if matches!(event, ModelEvent::Error(_)) {
            self.pending_save = None;
        }
        if let ModelEvent::ConnectionDeleted { id } = event {
            self.last_saved_previews.remove(id);
        }
        if let ModelEvent::Loaded { preferences, .. }
        | ModelEvent::PreferencesChanged(preferences) = event
        {
            self.pending_context_sync = Some(preferences.agent_settings.always_context.clone());
            let mode = match preferences.appearance {
                Appearance::Dark => ThemeMode::Dark,
                Appearance::Light => ThemeMode::Light,
            };
            if cx.theme().is_dark() != mode.is_dark() {
                Theme::change(mode, None, cx);
            }
            design::set_interface_scale(preferences.interface_scale, cx);
            design::set_ui_language(preferences.language.clone(), cx);
        }
        if matches!(
            event,
            ModelEvent::Loaded { .. }
                | ModelEvent::CatalogLoaded { .. }
                | ModelEvent::DetailsLoaded { .. }
                | ModelEvent::ConnectionDeleted { .. }
                | ModelEvent::PreferencesChanged(_)
        ) {
            self.controls_dirty = true;
            self.rebuild_rows();
        }
        cx.notify();
    }

    pub(crate) fn send(&mut self, command: ModelCommand, cx: &mut Context<Self>) {
        if let Err(error) = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Models(command))
        {
            self.state.apply(&ModelEvent::Error(error.to_string()));
        }
        cx.notify();
    }

    pub(crate) fn rebuild_rows(&mut self) {
        self.rows = self.state.filtered(&self.filter);
        if let Some(purpose) = self.choosing_purpose {
            self.rows.retain(|key| {
                self.state.model(key).is_some_and(|model| {
                    self.state
                        .connections
                        .iter()
                        .find(|connection| connection.id == key.connection_id)
                        .is_some_and(|connection| purpose.accepts(model, connection.kind))
                })
            });
        }
    }

    pub fn open_for_purpose(
        &mut self,
        purpose: sovra_core::files::ModelPurpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let details: Vec<_> = self
            .state
            .connections
            .iter()
            .filter(|connection| connection.kind == ProviderKind::Ollama)
            .flat_map(|connection| {
                self.state
                    .catalogs
                    .get(&connection.id)
                    .into_iter()
                    .flat_map(move |catalog| {
                        catalog
                            .snapshot
                            .models
                            .iter()
                            .filter(|model| !model.details_loaded)
                            .map(|model| model.key(&connection.id))
                    })
            })
            .take(32)
            .collect();
        for key in details {
            self.send(ModelCommand::FetchDetails { key }, cx);
        }
        self.choosing_purpose = Some(purpose);
        self.home_open = false;
        self.settings_open = false;
        self.connections_open = false;
        self.selected = None;
        self.compare_open = false;
        self.filter = ModelFilter {
            kind: Some(ModelKind::Chat),
            ..ModelFilter::default()
        };
        self.set_search("", window, cx);
        self.controls_dirty = true;
        cx.notify();
    }

    /// Shows one type of model (`None` shows every type), as the "Show" picker does.
    pub fn show_kind(
        &mut self,
        kind: Option<ModelKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = kind.map_or(ALL_KINDS, ModelKind::value).to_owned();
        self.kind.update(cx, |select, cx| {
            select.set_selected_value(&value, window, cx);
        });
        self.filter.kind = kind;
        self.rebuild_rows();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    pub fn set_search(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| {
            input.set_value(query.to_owned(), window, cx);
        });
        query.clone_into(&mut self.filter.search);
        self.rebuild_rows();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn read_filters(&mut self, cx: &mut Context<Self>) {
        let value = |control: &Entity<Selector>| {
            control
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_default()
        };
        self.filter.search = self.search.read(cx).value().to_string();
        self.filter.author = nonempty(value(&self.author));
        self.filter.connection_id = nonempty(value(&self.connection));
        self.filter.input_modality = nonempty(value(&self.input_modality));
        self.filter.kind = match value(&self.kind).as_str() {
            ALL_KINDS => None,
            other => Some(ModelKind::from_value(other).unwrap_or(ModelKind::Chat)),
        };
        self.filter.min_context = value(&self.context).parse().unwrap_or(0);
        self.filter.price = match value(&self.price).as_str() {
            "free" => PriceFilter::Free,
            "paid" => PriceFilter::Paid,
            _ => PriceFilter::Any,
        };
        self.filter.sort = match value(&self.sort).as_str() {
            "name" => ModelSort::Name,
            "context" => ModelSort::Context,
            "input" => ModelSort::InputPrice,
            "output" => ModelSort::OutputPrice,
            _ => ModelSort::Newest,
        };
        self.rebuild_rows();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn sync_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.controls_dirty {
            return;
        }
        let facets = self.state.facets(self.filter.scope);
        for (control, before, after, placeholder) in [
            (
                &self.author,
                &self.facets.authors,
                &facets.authors,
                "Any maker",
            ),
            (
                &self.input_modality,
                &self.facets.input_modalities,
                &facets.input_modalities,
                "Reads anything",
            ),
        ] {
            if before != after {
                let mut items = vec![Choice::new("", placeholder)];
                items.extend(after.iter().map(|value| Choice::new(value, value)));
                control.update(cx, |select, cx| {
                    select.set_items(SearchableVec::new(items), window, cx);
                });
            }
        }
        self.facets = facets;
        let mut connections = vec![Choice::new("", "Any service")];
        connections.extend(
            self.state
                .connections
                .iter()
                .map(|c| Choice::new(&c.id, &c.name)),
        );
        self.connection.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(connections), window, cx);
        });
        let servers: Vec<_> = self
            .state
            .connections
            .iter()
            .filter(|c| c.kind == ProviderKind::Ollama)
            .map(|c| Choice::new(&c.id, &c.name))
            .collect();
        self.download_connection.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(servers.clone()), window, cx);
            if select.selected_value().is_none()
                && let Some(first) = servers.first()
            {
                select.set_selected_value(&first.value, window, cx);
            }
        });
        self.controls_dirty = false;
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter = ModelFilter {
            scope: self.filter.scope,
            kind: Some(ModelKind::Chat),
            sort: ModelSort::Newest,
            ..ModelFilter::default()
        };
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        for control in [
            &self.author,
            &self.connection,
            &self.input_modality,
            &self.context,
            &self.price,
            &self.sort,
        ] {
            control.update(cx, |select, cx| select.set_selected_index(None, window, cx));
        }
        self.kind.update(cx, |select, cx| {
            select.set_selected_value(&ModelKind::Chat.value().to_owned(), window, cx);
        });
        self.rebuild_rows();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    pub(crate) fn select_model(&mut self, key: ModelKey, cx: &mut Context<Self>) {
        self.selected = Some(key.clone());
        self.send(ModelCommand::FetchDetails { key }, cx);
    }

    pub(crate) fn refresh_visible(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<_> = self
            .state
            .connections
            .iter()
            .filter(|c| match self.filter.scope {
                CatalogScope::OpenRouter => c.kind == ProviderKind::OpenRouter,
                CatalogScope::Ollama => c.kind == ProviderKind::Ollama,
                CatalogScope::Custom => matches!(
                    c.kind,
                    ProviderKind::OpenAiCompatible | ProviderKind::Anthropic
                ),
                CatalogScope::All | CatalogScope::Favorites => true,
            })
            .map(|c| c.id.clone())
            .collect();
        for connection_id in ids {
            self.send(ModelCommand::Refresh { connection_id }, cx);
        }
    }

    fn active_filter_count(&self) -> usize {
        usize::from(!self.filter.search.trim().is_empty())
            + usize::from(self.filter.author.is_some())
            + usize::from(self.filter.connection_id.is_some())
            + usize::from(self.filter.input_modality.is_some())
            + usize::from(self.filter.kind != Some(ModelKind::Chat))
            + usize::from(self.filter.min_context > 0)
            + usize::from(self.filter.price != PriceFilter::Any)
            + usize::from(self.filter.tools)
            + usize::from(self.filter.reasoning)
            + usize::from(self.filter.structured_output)
    }

    fn scope_guide(&self) -> (&'static str, &'static str) {
        match self.filter.scope {
            CatalogScope::All => (
                "All connected sources",
                "Browse every AI model available from your cloud and local connections.",
            ),
            CatalogScope::OpenRouter => (
                "Online · OpenRouter Cloud",
                "Access 300+ frontier models (Claude, GPT, Gemini, DeepSeek, Llama) with one API key.",
            ),
            CatalogScope::Ollama => (
                "On my computer · Ollama",
                "Private, offline models running directly on your hardware for free.",
            ),
            CatalogScope::Custom => (
                "Other services · Custom Servers",
                "Connect LM Studio, vLLM, llama.cpp, Anthropic, or any OpenAI-compatible server.",
            ),
            CatalogScope::Favorites => (
                "★ Your Favorites",
                "Quick access to the models you've starred for everyday work.",
            ),
        }
    }

    fn render_active_model_banner(
        &self,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let active_key = self.state.preferences.active.clone();
        let active_model = active_key.as_ref().and_then(|k| self.state.model(k));
        let active_conn = active_key.as_ref().and_then(|k| {
            self.state
                .connections
                .iter()
                .find(|c| c.id == k.connection_id)
        });

        let (headline, summary, status_text, needs_key) =
            match (active_key.as_ref(), active_model, active_conn) {
                (Some(_), Some(model), Some(conn)) => {
                    let missing_key = conn.authentication
                        != sovra_core::models::Authentication::None
                        && !conn.has_credential
                        && !conn.environment_credential;
                    let status = if missing_key {
                        format!("{} · Needs API key to chat", conn.name)
                    } else {
                        format!("{} · Ready to answer in Chat", conn.name)
                    };
                    let summary = format!(
                        "Memory {} · {} · {}",
                        compact_count(model.context_length),
                        if model.supports("tools") {
                            "Can use tools"
                        } else {
                            "Standard chat"
                        },
                        price_summary(model)
                    );
                    (model.name.clone(), summary, status, missing_key)
                }
                (Some(key), _, _) => (
                    key.model_id.clone(),
                    "Selected for your Personal AI. Refresh the catalog to load full details."
                        .to_owned(),
                    format!("Connection: {}", key.connection_id),
                    false,
                ),
                (None, _, _) => (
                    "Choose an AI before chatting".to_owned(),
                    "Open a model to see its price and choose it. Setup can help you get started."
                        .to_owned(),
                    "No model selected".to_owned(),
                    false,
                ),
            };

        let openrouter_key_pill = self
            .state
            .openrouter_connection()
            .and_then(|c| self.active_key_preview(c))
            .map_or_else(
                || "OpenRouter key: Not added".to_owned(),
                |preview| format!("OpenRouter key: {preview}"),
            );

        let mut banner = h_flex()
            .id("models-active-banner")
            .test_support()
            .w_full()
            .flex_wrap()
            .justify_between()
            .items_center()
            .gap(t.space.small)
            .px(t.space.medium)
            .py(t.space.small)
            .sovra_surface(design::Surface::Subtle, cx)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(t.space.hair)
                    .child(
                        h_flex()
                            .flex_wrap()
                            .items_center()
                            .gap(t.space.compact)
                            .child(
                                div()
                                    .sovra_text(TextRole::Caption, cx)
                                    .text_color(cx.theme().link)
                                    .child("CURRENTLY SET FOR YOUR AI"),
                            )
                            .child(pill(status_text, cx))
                            .child(
                                div()
                                    .id("models-active-key-status")
                                    .test_support()
                                    .child(pill(openrouter_key_pill, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("models-active-name")
                            .test_support()
                            .sovra_text(TextRole::SectionTitle, cx)
                            .child(headline),
                    )
                    .children((!narrow).then(|| caption(summary, cx))),
            );

        let mut actions = h_flex().items_center().gap(t.space.compact);
        if let Some(inspect_key) = active_key {
            actions = actions.child(
                design::button("models-active-inspect", "View active setup", cx)
                    .small()
                    .ghost()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.select_model(inspect_key.clone(), cx);
                    })),
            );
            if !needs_key {
                actions = actions.child(
                    design::primary_button("models-active-go-chat", "Go to Chat →", cx)
                        .small()
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(GoToChat))),
                );
            }
        }
        actions = actions.child(
            design::button("models-open-settings", "AI Keys & Settings", cx)
                .small()
                .ghost()
                .on_click(cx.listener(|view, _, _, cx| {
                    view.settings_open = true;
                    cx.notify();
                })),
        );
        if needs_key {
            let conn = active_conn.cloned();
            actions = actions.child(
                design::primary_button("models-active-connect", "Add API key", cx)
                    .small()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.open_connection(conn.clone(), window, cx);
                    })),
            );
        }
        if !self.compare.is_empty() {
            actions = actions.child(
                design::button(
                    "models-open-compare",
                    format!("Compare ({}/3)", self.compare.len()),
                    cx,
                )
                .small()
                .on_click(cx.listener(|view, _, _, cx| {
                    view.compare_open = true;
                    cx.notify();
                })),
            );
        }
        banner = banner.child(actions);
        banner
    }

    fn header(&self, narrow: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let total: usize = self
            .state
            .catalogs
            .values()
            .map(|c| {
                c.snapshot
                    .models
                    .iter()
                    .filter(|m| m.kind() == ModelKind::Chat)
                    .count()
            })
            .sum();
        let loading = self.state.catalogs.values().any(|c| c.loading);
        let (scope_title, scope_desc) = self.scope_guide();

        let mut hdr = v_flex()
            .flex_shrink_0()
            .gap(t.space.small)
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .justify_between()
                    .items_center()
                    .child(
                        v_flex()
                            .gap(t.space.hair)
                            .child(
                                h_flex()
                                    .gap(t.space.small)
                                    .items_center()
                                    .child(
                                        div().sovra_text(TextRole::PageTitle, cx).child("Models"),
                                    )
                                    .child(pill(format!("{total} chat models"), cx)),
                            )
                            .children((!narrow).then(|| {
                                caption(
                                    "Explore online and local models. Compare capabilities and choose what fits.",
                                    cx,
                                )
                            })),
                    )
                    .child(
                        h_flex()
                            .gap(t.space.compact)
                            .child(design::button("models-back-home", "Setup", cx)
                                .on_click(cx.listener(|view, _, _, cx| view.open_model_home(cx))))
                            .child(
                                design::button(
                                    "models-refresh",
                                    if loading { "Refreshing…" } else { "Refresh" },
                                    cx,
                                )
                                .disabled(loading)
                                .on_click(cx.listener(|view, _, _, cx| view.refresh_visible(cx))),
                            )
                            .child(
                                design::button("models-connections", "Connections", cx)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(OpenConnections {
                                            kind: None,
                                            keep_editor: false,
                                        });
                                    })),
                            ),
                    ),
            );

        if !narrow
            && !self.connections_open
            && self.connection_form.is_none()
            && self.manual_form.is_none()
        {
            hdr = hdr.child(self.render_active_model_banner(narrow, cx));
        }

        hdr = hdr.child(
            h_flex()
                .id("model-scope-tabs")
                .w_full()
                .min_w_0()
                .overflow_x_scroll()
                .flex_shrink_0()
                .gap(t.space.compact)
                .children(
                    [
                        (CatalogScope::All, "All models"),
                        (CatalogScope::OpenRouter, "Online"),
                        (CatalogScope::Ollama, "On my computer"),
                        (CatalogScope::Custom, "Other services"),
                        (CatalogScope::Favorites, "Favorites"),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (scope, label))| {
                        design::button(("model-scope", index), label, cx)
                            .ghost()
                            .flex_shrink_0()
                            .selected(self.filter.scope == scope)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.filter.scope = scope;
                                view.controls_dirty = true;
                                view.rebuild_rows();
                                view.scroll.scroll_to_item(0, ScrollStrategy::Top);
                                cx.notify();
                            }))
                    }),
                ),
        );

        if !narrow
            && !self.connections_open
            && self.connection_form.is_none()
            && self.manual_form.is_none()
        {
            hdr = hdr.child(
                h_flex()
                    .items_center()
                    .gap(t.space.compact)
                    .child(pill(scope_title, cx))
                    .child(caption(scope_desc, cx)),
            );
        }

        if let Some(purpose) = self.choosing_purpose {
            hdr = hdr.child(v_flex().gap(t.space.small)
                .child(div().sovra_text(TextRole::SectionTitle, cx).child(purpose.label()))
                .child(caption("Only compatible models are shown. Choose one, then return to your waiting message and files.", cx))
                .child(h_flex().flex_wrap().gap(t.space.small)
                    .child(design::button("file-model-back", "Back to my message", cx).ghost().on_click(cx.listener(|_, _, _, cx| cx.emit(GoToChat))))
                    .child(design::button("file-model-connections", "Connect an AI service", cx).on_click(cx.listener(|view, _, window, cx| {
                        view.setup_flow = false;
                        view.open_catalogue_kind(sovra_core::CatalogueKind::OnlineAi, window, cx);
                        cx.emit(OpenConnections {
                            kind: Some(sovra_core::CatalogueKind::OnlineAi),
                            keep_editor: true,
                        });
                    })))));
        }
        hdr
    }

    fn filters(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let active_filters = self.active_filter_count();
        let filters_label = if active_filters > 0 {
            format!("Filters ({active_filters})")
        } else {
            "Filters".to_owned()
        };
        let mut controls = v_flex().flex_shrink_0().gap(t.space.small).child(
            h_flex()
                .gap(t.space.small)
                .child(
                    Input::new(&self.search)
                        .id("models-search")
                        .aria_label("Search models")
                        .cleanable(true)
                        .flex_1(),
                )
                .child(filter_select(&self.kind, "Show", "Chat models", cx))
                .child(
                    design::button("models-toggle-filters", filters_label, cx)
                        .selected(self.filters_open || active_filters > 0)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.filters_open = !view.filters_open;
                            cx.notify();
                        })),
                ),
        );
        if self.filters_open {
            controls = controls
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.compact)
                        .child(filter_select(
                            &self.connection,
                            "Service",
                            "Any service",
                            cx,
                        ))
                        .child(filter_select(&self.author, "Made by", "Any maker", cx))
                        .child(filter_select(
                            &self.input_modality,
                            "What it understands",
                            "Reads anything",
                            cx,
                        ))
                        .child(filter_select(
                            &self.context,
                            "Memory size",
                            "Any memory size",
                            cx,
                        ))
                        .child(filter_select(&self.price, "Price", "Any price", cx)),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap(t.space.medium)
                        .child(
                            Checkbox::new("models-filter-tools")
                                .cursor_pointer()
                                .label("Can use tools")
                                .checked(self.filter.tools)
                                .on_click(cx.listener(|view, checked, _, cx| {
                                    view.filter.tools = *checked;
                                    view.rebuild_rows();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Checkbox::new("models-filter-reasoning")
                                .cursor_pointer()
                                .label("Thinks step by step")
                                .checked(self.filter.reasoning)
                                .on_click(cx.listener(|view, checked, _, cx| {
                                    view.filter.reasoning = *checked;
                                    view.rebuild_rows();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Checkbox::new("models-filter-json")
                                .cursor_pointer()
                                .label("Can fill in forms")
                                .checked(self.filter.structured_output)
                                .on_click(cx.listener(|view, checked, _, cx| {
                                    view.filter.structured_output = *checked;
                                    view.rebuild_rows();
                                    cx.notify();
                                })),
                        )
                        .child(
                            design::button("models-clear-filters", "Clear filters", cx)
                                .ghost()
                                .small()
                                .on_click(cx.listener(Self::clear_filters_click)),
                        ),
                );
        }
        controls.child(
            h_flex()
                .justify_between()
                .items_center()
                .gap(t.space.small)
                .child(caption(
                    format!(
                        "Showing {} model{} · Click any row to inspect or activate",
                        self.rows.len(),
                        if self.rows.len() == 1 { "" } else { "s" }
                    ),
                    cx,
                ))
                .child(filter_select(&self.sort, "Sort models", "Newest first", cx)),
        )
    }

    fn clear_filters_click(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset_filters(window, cx);
    }

    fn row(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let key = self.rows[index].clone();
        let model = self
            .state
            .model(&key)
            .expect("rows derive from the current catalog");
        let favorite = self.state.preferences.favorites.contains(&key);
        let active = self.choosing_purpose.map_or_else(
            || self.state.preferences.active.as_ref() == Some(&key),
            |purpose| self.state.preferences.purposes.get(&purpose) == Some(&key),
        );
        let selected = self.selected.as_ref() == Some(&key);
        let favorite_key = key.clone();
        let name = model.name.clone();
        let connection_obj = self
            .state
            .connections
            .iter()
            .find(|c| c.id == key.connection_id);
        let connection = connection_obj.map_or("", |c| c.name.as_str());
        let needs_key = connection_obj.is_some_and(|c| {
            c.authentication != sovra_core::models::Authentication::None
                && !c.has_credential
                && !c.environment_credential
        });
        let mut capability_tags = Vec::new();
        if model.supports("tools") {
            capability_tags.push("Tools");
        }
        if model.supports("reasoning") || model.supports("include_reasoning") {
            capability_tags.push("Step-by-step reasoning");
        }
        if model.input_modalities.iter().any(|m| m == "image") {
            capability_tags.push("Reads pictures");
        }
        let capabilities_str = if capability_tags.is_empty() {
            String::new()
        } else {
            format!("   ·   {}", capability_tags.join(" · "))
        };
        let kind = model.kind();
        let metrics = if kind == ModelKind::Chat {
            format!(
                "Chat   ·   Memory {}   ·   {}{}",
                compact_count(model.context_length),
                price_summary(model),
                capabilities_str,
            )
        } else {
            format!(
                "{}   ·   Not for chat   ·   {}",
                kind.tag(),
                price_summary(model)
            )
        };
        let badge_text = if active {
            "✓ Using Now".to_owned()
        } else if needs_key {
            format!("{connection} · Needs Key")
        } else {
            connection.to_owned()
        };
        let subtitle = if model.author.is_empty() {
            model.id.clone()
        } else {
            format!("By {}   ·   {}", model.author, model.id)
        };
        let can_chat = model.can_chat();
        let use_key = key.clone();
        let edit_conn = connection_obj.cloned();
        let use_label = if self.choosing_purpose.is_some() && !needs_key {
            "Use for files"
        } else if active && !needs_key {
            "✓ Active"
        } else if !can_chat {
            "Non-chat"
        } else if needs_key {
            "Add Key"
        } else {
            "Use Model"
        };
        v_flex()
            .id(("catalog-row", index))
            .test_support()
            .h(t.layout.catalog_row_height)
            .w_full()
            .min_w_0()
            .px(t.space.small)
            .py(t.space.compact)
            .gap(t.space.compact)
            .bg(if selected || active {
                t.nav_active_background
            } else {
                t.canvas
            })
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.space.compact)
                    .child(
                        design::button(("model-row", index), name, cx)
                            .child(div().flex_1())
                            .px(t.space.compact)
                            .accessibility_label(format!("View details for {}", model.id))
                            .ghost()
                            .flex_1()
                            .justify_start()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.select_model(key.clone(), cx);
                            })),
                    )
                    .child(pill(badge_text, cx))
                    .child(
                        design::button(("model-use-row", index), use_label, cx)
                            .small()
                            .disabled(
                                !can_chat
                                    || (active && !needs_key && self.choosing_purpose.is_none()),
                            )
                            .accessibility_label(format!("Use {} in chat", model.id))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                if needs_key {
                                    view.open_connection(edit_conn.clone(), window, cx);
                                } else {
                                    view.choose_model(&use_key, cx);
                                }
                            })),
                    )
                    .child(
                        design::button(
                            ("model-favorite", index),
                            if favorite { "★" } else { "☆" },
                            cx,
                        )
                        .ghost()
                        .accessibility_label(if favorite {
                            "Remove from favorites"
                        } else {
                            "Add to favorites"
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.send(
                                ModelCommand::SetFavorite {
                                    key: favorite_key.clone(),
                                    favorite: !favorite,
                                },
                                cx,
                            );
                        })),
                    ),
            )
            .child(caption(subtitle, cx).truncate())
            .child(caption(metrics, cx).truncate())
    }

    fn catalog(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut content = v_flex()
            .flex_1()
            .h_full()
            .min_h_0()
            .min_w_0()
            .gap(t.space.small)
            .child(self.filters(cx));
        if self.filter.scope == CatalogScope::Ollama {
            content = content.child(
                design::button("ollama-toggle-download", "Download a model", cx).on_click(
                    cx.listener(|view, _, _, cx| {
                        view.download_open = !view.download_open;
                        cx.notify();
                    }),
                ),
            );
            if self.download_open {
                content = content.child(self.render_download(cx));
            } else if let Some(progress) = &self.state.pull_progress {
                content = content.child(caption(progress.clone(), cx));
            }
        }
        let errors: Vec<_> = self
            .state
            .connections
            .iter()
            .filter(|connection| match self.filter.scope {
                CatalogScope::Ollama => connection.kind == ProviderKind::Ollama,
                CatalogScope::OpenRouter => connection.kind == ProviderKind::OpenRouter,
                CatalogScope::Custom => matches!(
                    connection.kind,
                    ProviderKind::OpenAiCompatible | ProviderKind::Anthropic
                ),
                CatalogScope::All | CatalogScope::Favorites => {
                    connection.kind != ProviderKind::Ollama
                        || self
                            .rows
                            .iter()
                            .any(|key| key.connection_id == connection.id)
                }
            })
            .filter_map(|c| {
                self.state
                    .catalogs
                    .get(&c.id)
                    .and_then(|catalog| catalog.error.as_ref())
                    .map(|error| format!("{}: {error}", c.name))
            })
            .collect();
        if !errors.is_empty() {
            content = content.child(caption(errors.join("\n"), cx).text_color(cx.theme().warning));
        }
        if self.rows.is_empty() {
            let loading = !self.state.loaded || self.state.catalogs.values().any(|c| c.loading);
            let title = if loading {
                "Loading your model library…"
            } else {
                "No matching models"
            };
            let help = if self.filter.scope == CatalogScope::Ollama {
                "Nothing here yet. Open the Ollama app on this computer, then press Refresh, or use Download a model."
            } else if self.filter.scope == CatalogScope::Custom {
                "Nothing here yet. Use Connections to add another service, such as LM Studio."
            } else {
                "Try clearing the filters or pressing Refresh. To use online models, add your key in Connections."
            };
            content = content.child(
                v_flex()
                    .id("models-empty")
                    .test_support()
                    .flex_1()
                    .justify_center()
                    .items_center()
                    .p(t.space.large)
                    .gap(t.space.small)
                    .child(div().sovra_text(TextRole::SectionTitle, cx).child(title))
                    .child(caption(help, cx))
                    .child(
                        design::button("models-empty-reset", "Clear filters", cx)
                            .on_click(cx.listener(Self::clear_filters_click)),
                    ),
            );
        } else {
            let entity = cx.entity();
            content = content.child(
                uniform_list("models-list", self.rows.len(), move |range, _, cx| {
                    entity.update(cx, |view, cx| {
                        range.map(|index| view.row(index, cx)).collect::<Vec<_>>()
                    })
                })
                .track_scroll(&self.scroll)
                .flex_1()
                // The open filter panel must not shrink this virtual list to
                // zero, or the first catalog row never enters the frame.
                .min_h(t.layout.catalog_row_height)
                .w_full(),
            );
        }
        content
    }
}

impl Render for ModelsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Measure this pane, not the window: the sidebar and onboarding bar
        // already take part of the width.
        div()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(design::width_probe(cx, |view: &mut Self| {
                &mut view.width_class
            }))
            .child(self.render_page(window, cx))
    }
}

impl ModelsView {
    fn render_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(context_text) = self.pending_context_sync.take() {
            self.always_context_input.update(cx, |input, cx| {
                input.set_value(context_text, window, cx);
            });
        }
        if let Some(id) = self.saved_form_refresh.take()
            && let Some(connection) = self.state.connections.iter().find(|c| c.id == id).cloned()
        {
            // Recreate the masked input only after the runtime acknowledges persistence.
            // This discards the secret draft while keeping management controls visible.
            let notice = self.state.notice.clone();
            let error = self.state.error.clone();
            self.open_connection(Some(connection), window, cx);
            self.state.notice = notice;
            self.state.error = error;
        }
        if self.groups_page {
            return self.render_groups_page(cx);
        }
        if self.connections_page {
            return self.render_connections_page(window, cx);
        }
        if self.settings_open {
            return self.render_settings(cx).into_any_element();
        }
        if self.home_open {
            return self
                .render_model_home(!self.width_class.fits_detail_pane(), cx)
                .into_any_element();
        }
        self.sync_controls(window, cx);
        let t = design::tokens(cx);
        let narrow = !self.width_class.fits_detail_pane();
        let mut page = v_flex()
            .id("sovra-models-view")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .p(if narrow {
                t.space.small
            } else {
                t.space.medium
            })
            .gap(t.space.small)
            .child(self.header(narrow, cx));
        if let Some(error) = &self.state.error {
            page = page.child(caption(error.clone(), cx).text_color(cx.theme().danger));
        } else if let Some(message) = &self.state.notice {
            page = page.child(caption(message.clone(), cx));
        }
        if self.connection_form.is_some() {
            return page
                .child(self.render_connection_form(cx))
                .into_any_element();
        }
        if self.manual_form.is_some() {
            return page.child(self.render_manual_form(cx)).into_any_element();
        }
        if self.connections_open {
            return page.child(self.render_connections(cx)).into_any_element();
        }
        if self.compare_open {
            return page.child(self.render_comparison(cx)).into_any_element();
        }
        if narrow && self.download_open && self.filter.scope == CatalogScope::Ollama {
            return page
                .child(
                    v_flex()
                        .id("ollama-download-panel")
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(self.render_download(cx)),
                )
                .child(
                    design::button("ollama-download-back", "Back to installed models", cx)
                        .flex_shrink_0()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.download_open = false;
                            cx.notify();
                        })),
                )
                .into_any_element();
        }
        if narrow && self.filters_open && self.selected.is_none() {
            return page
                .child(
                    v_flex()
                        .id("models-filter-panel")
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(self.filters(cx)),
                )
                .child(
                    design::primary_button(
                        "models-apply-filters",
                        format!("Show {} models", self.rows.len()),
                        cx,
                    )
                    .flex_shrink_0()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.filters_open = false;
                        cx.notify();
                    })),
                )
                .into_any_element();
        }
        if narrow && self.selected.is_some() {
            return page.child(self.render_model_details(cx)).into_any_element();
        }
        let mut body = h_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap(t.space.medium)
            .child(self.catalog(cx));
        if self.selected.is_some() {
            body = body.child(
                div()
                    .w(t.layout.detail_width)
                    .h_full()
                    .flex_shrink_0()
                    .child(self.render_model_details(cx)),
            );
        }
        page.child(body).into_any_element()
    }
}

pub(crate) fn choices(items: &[(&str, &str)]) -> Vec<Choice> {
    items
        .iter()
        .map(|(value, title)| Choice::new(value, title))
        .collect()
}
fn nonempty(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}
pub(crate) fn caption(value: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
    div()
        .sovra_text(TextRole::Caption, cx)
        .text_color(design::tokens(cx).text_secondary)
        .child(value.into())
}
pub(crate) fn pill(value: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
    let t = design::tokens(cx);
    caption(value, cx)
        .px(t.space.compact)
        .py(t.space.compact)
        .rounded(t.radius.pill)
        .bg(t.subtle)
}
pub(crate) fn filter_select(
    state: &Entity<Selector>,
    label: &'static str,
    placeholder: &'static str,
    cx: &App,
) -> gpui_kit::Div {
    select_control(state, label, placeholder, cx).w(design::tokens(cx).layout.filter_width)
}

pub(crate) fn select_control(
    state: &Entity<Selector>,
    label: &'static str,
    placeholder: &'static str,
    cx: &App,
) -> gpui_kit::Div {
    // The pinned Select fills its outer wrapper; styling Select itself only
    // sizes its inner input. Bound the wrapper so its hitbox cannot cover rows.
    div()
        .w_full()
        .h(design::tokens(cx).layout.control_height)
        .flex_shrink_0()
        .cursor_pointer()
        .child(
            Select::new(state)
                .id(label)
                .accessibility_label(label)
                .placeholder(placeholder)
                .cursor_pointer()
                .size_full(),
        )
}
pub(crate) fn compact_count(value: Option<u64>) -> String {
    match value {
        None => "Unknown".into(),
        Some(v) if v >= 1_000_000 => format!("{}.{}M", v / 1_000_000, (v % 1_000_000) / 100_000),
        Some(v) if v >= 1_000 => format!("{}K", v / 1_000),
        Some(v) => v.to_string(),
    }
}
pub(crate) fn token_price(model: &sovra_core::models::ModelInfo, field: &str) -> String {
    model.token_price(field).map_or_else(
        || "Unknown".into(),
        |price| format!("${:.3}", price * 1_000_000.),
    )
}
pub(crate) fn price_summary(model: &sovra_core::models::ModelInfo) -> String {
    if model.is_free() {
        "Free".into()
    } else if model.output_modalities.iter().any(|m| m == "text")
        && model.typical_message_cost().is_some()
    {
        // Lists compare cost per message; exact token rates live in the details panel.
        model.plain_price()
    } else if model.pricing.is_empty() {
        "Price unknown".into()
    } else {
        "Paid · see details".into()
    }
}
