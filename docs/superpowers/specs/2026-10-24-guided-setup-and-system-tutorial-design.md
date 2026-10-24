# Guided Setup, State-Aware Model Selection & Interactive System Tutorial — Design Spec

**Date:** 2026-10-24
**Scope:** `sovra-core`, `sovra-runtime`, `sovra-ui`

## 1. Problem Statement & UX Goals

First-time and non-technical users opening Sovra or attempting to send a chat message encounter friction and ambiguity:
1. **Redundant setup options when a key is already configured:** When a user has already saved an API key (e.g., OpenRouter), the in-chat setup card and setup screens still dump all connection options instead of showing **only** the next required action (selecting a model).
2. **Unclear model selection:** In the Models catalog, clicking a row only opens a side inspection panel rather than offering a direct one-click activation button, and the guided setup screen only suggests a single free model without curated recommendations or clear status progression.
3. **No guided first-run walkthrough or system tutorial:** A user with zero prior experience needs a step-by-step onboarding wizard that walks them through getting a key (with a one-click browser link to `https://openrouter.ai/keys`), picking a recommended model, and taking a complete interactive tour of Sovra's core capabilities (`Chat`, `My AI's`, `Knowledge`, `Schedules`, and `Settings`).

### Design Goals
- **Strict State-Aware Progressive Disclosure ("Show Only What's Next"):** Never ask the user to do something they have already completed.
- **One-Click External Helper & Inline Guidance:** Provide a direct browser launcher button (`Open openrouter.ai/keys in Browser`) and clear 1-2-3 visual instructions.
- **Crystal-Clear Model Recommendations & 1-Click Row Activation:** Present up to 3 curated recommendation cards (*Best Free Starter*, *Best Everyday All-Rounder*, *Best Deep Thinker*) on the setup screen AND an explicit `Use Model` / `✓ Active` action button on every row in the full catalog.
- **Interactive 5-Step System Tutorial & Spotlight Coach Bar:** Guide users step-by-step through the live application tabs with a persistent top walkthrough bar, sidebar spotlight indicators, and a one-click replay option.

---

## 2. Domain Model & State Machine (`sovra-core`)

### 2.1 `SetupStage` (Pure State Derivation in `sovra-core/src/models.rs`)
`ModelsState::setup_stage()` deterministically computes what the user needs to do next:
- `SetupStage::Loading`: Models state has not yet loaded from the runtime.
- `SetupStage::NeedsConnection`: No connection has valid credentials (`has_credential || environment_credential`) and no local unauthenticated catalog has models loaded.
- `SetupStage::NeedsModelKey { model_id, connection_id, connection_name }`: A model is selected (`preferences.active = Some(..)`), but its connection requires authentication and has no key saved.
- `SetupStage::NeedsModelSelection`: At least one connection is ready (e.g., OpenRouter key is saved or Ollama has models), but `preferences.active` is `None` (or points to a removed connection).
- `SetupStage::Ready { active: ModelKey }`: `has_active_llm()` is `true`.

### 2.2 State-Aware In-Chat Setup Surface (`sovra-core/src/agui.rs`)
Replace the static 3-card dump in `default_llm_setup_widget` with a state-aware builder `llm_setup_widget_for_stage(stage: &SetupStage, recommended: Option<&ModelInfo>, key_preview: Option<&str>) -> AgUiWidget`:
- **When `SetupStage::NeedsConnection`:**
  - Shows **Step 1 of 2: Connect Your AI Service**.
  - Primary card: **Option 1 · Online with OpenRouter (Recommended for Beginners)** — explains what OpenRouter is in plain English, lists 3 numbered steps, includes a `setup:open-openrouter-web` button (`Get Free Key at openrouter.ai/keys ↗`) and primary `setup:openrouter-key` button (`Paste OpenRouter Key`).
  - Secondary compact row: `Use Free Local AI (Ollama)` (`setup:ollama`) and `Connect Custom OpenAI URL` (`setup:openai-url`).
- **When `SetupStage::NeedsModelSelection` (Key is ALREADY set!):**
  - Shows **✓ Step 1 Complete: OpenRouter Key Connected (`sk-or-••••`)**.
  - Shows **Step 2 of 2: Pick Your AI Model to Start Chatting** — hides all connection setup options completely.
  - If a recommended model is available, provides a primary `setup:activate-recommended` button (`Use Recommended Model & Start Chatting`) plus `setup:choose-model` (`See Recommended Models`).
- **When `SetupStage::NeedsModelKey { .. }`:**
  - Shows **Almost Ready: Add Your API Key for `<connection_name>`** with a single primary button to paste the key for that specific connection.

### 2.3 Curated Model Recommendations (`sovra-core/src/models.rs`)
Add `RecommendedModelCard` and `ModelsState::recommended_model_cards() -> Vec<RecommendedModelCard>`:
- Categorizes available chat-capable models from connected providers into up to 3 clear tiers:
  1. `RecommendationKind::FreeStarter` — **"★ Best Free Starter"** (`$0 · Free to try right now for everyday questions`). Prefers `openrouter/free` or top zero-price text model.
  2. `RecommendationKind::EverydaySmart` — **"⚡ Best Everyday All-Rounder"** (`Fast, smart & affordable for daily writing, email, and work`). Prefers well-known frontier flash/sonnet/mini models or the highest-context standard chat model.
  3. `RecommendationKind::DeepReasoning` — **"🧠 Best Step-by-Step Thinker"** (`Thinks through hard problems, math, and coding step-by-step`). Prefers models supporting `reasoning` / `include_reasoning`.
- If the user only has local Ollama models, returns installed local models as recommended cards.

### 2.4 Interactive Tutorial State (`TourStep` & `ModelPreferences`)
Add `onboarding_completed: bool` (with `#[serde(default)]`) to `ModelPreferences` and `ModelCommand::SetOnboardingCompleted { completed: bool }`.
Add `TourStep` enum in `sovra-core/src/workspace.rs`:
- `TourStep::WelcomeChat` (Step 1 of 5 — `NavTab::Chat`): Try your first message & starter chips.
- `TourStep::AutomaticRouting` (Step 2 of 5 — `NavTab::MyAis`): How Sovra picks the right AI brain automatically.
- `TourStep::KnowledgeMemory` (Step 3 of 5 — `NavTab::Knowledge`): How Sovra remembers facts across chats without losing history.
- `TourStep::AutomatedSchedules` (Step 4 of 5 — `NavTab::Schedules`): Putting recurring tasks and daily briefings on autopilot.
- `TourStep::PersonalizeSettings` (Step 5 of 5 — `NavTab::Settings`): Setting your Always-On Context ("sticky note") and reply tone.

---

## 3. UI Components & User Flows (`sovra-ui`)

### 3.1 Top Guided Setup & Tour Coach Bar (`crates/sovra-ui/src/onboarding_bar.rs`)
Rendered at the top of the main workspace in `Shell` whenever:
- Setup is incomplete (`!state.has_active_llm` and `state.models_loaded`), OR
- The Interactive System Tour is active (`shell.tour_step.is_some()`).

**Visual Structure:**
- Uses `Surface::Elevated` / `Surface::Subtle` with Sovra's violet accent border and tokens (`design::tokens(cx)`).
- **When Setup is Incomplete:**
  - Stepper indicator:
    - If no key: **`[1. Connect AI Service]`** `→ 2. Pick Model → 3. Quick Tour`
    - If key is saved: **`✓ 1. Key Connected`** `→` **`[2. Pick Your Model]`** `→ 3. Quick Tour`
  - Direct primary action button on the right:
    - When `NeedsConnection`: **`Step 1: Connect OpenRouter Key →`** (plus `Get Key at openrouter.ai ↗` and `Skip to Tour`).
    - When `NeedsModelSelection`: **`Step 2: Choose Recommended AI →`** (or 1-click **`Activate Free AI Now`**).
- **When Interactive Tour is Active (`TourStep` 1..=5):**
  - Shows `GUIDED SYSTEM TOUR · STEP X OF 5` badge + step title + plain-English explanation of what is on screen and what to try.
  - Action buttons:
    - Step-specific helper action (e.g., on Step 1: **`Insert Sample Prompt`**; on Step 2: **`Test Route Simulator`**; on Step 5: **`Use Friendly Simple Style`**).
    - **`← Previous`** (when `step > 1`), **`Next Step: <Title> →`** (or **`Finish Tutorial & Start Chatting ✓`** on Step 5), and **`End Tour`**.

### 3.2 Sidebar Spotlight Badge (`crates/sovra-ui/src/sidebar.rs`)
- When a `TourStep` is active or setup points to `Models`/`Settings`, the relevant `NavTab` in the left sidebar displays a crisp pill indicator (`Step X` or `Setup`) so the user can see where they are in the system.
- Includes a subtle **`? Guided Tour`** button in the sidebar footer above the user profile so the user can launch or replay the walkthrough from anywhere in 1 click.

### 3.3 Step-by-Step Setup Home (`crates/sovra-ui/src/model_home.rs`)
Redesigned to be an unmistakable, zero-confusion wizard:
- **Progress Stepper Header:** Shows `Step 1: Connect AI` → `Step 2: Pick Model` → `Step 3: Start Chatting` with green checkmarks on completed steps.
- **State 1 — Needs Connection (`!connected`):**
  - Hero card: **Step 1 of 2 · Connect OpenRouter (Recommended)**
  - Plain-English explanation of what OpenRouter is ("One free account gives Sovra access to 300+ top AI models like Claude, ChatGPT, Gemini, and free starter models").
  - Two clear buttons side-by-side:
    1. **`1. Get a Free Key at openrouter.ai ↗`** (`cx.open_url("https://openrouter.ai/keys")`)
    2. **`2. Paste Your OpenRouter Key →`** (opens the inline/dedicated key editor)
  - Optional collapsible/secondary footer for local power users: `Use Free Local AI (Ollama)` or `Connect Custom OpenAI Server`.
- **State 2 — Key Connected, Needs Model (`connected && active.is_none()`):**
  -Step 1 collapses into a compact green confirmation banner: **`✓ Step 1 Complete · OpenRouter Key Connected (`sk-or-••••`)`** with a small `Change Key` ghost button.
  - Hero section: **Step 2 of 2 · Pick Your AI Model (Recommended for You)**
  - Renders the Curated Recommendation Cards (`FreeStarter`, `EverydaySmart`, `DeepReasoning`), each showing:
    - Badge (`★ BEST FREE STARTER`, `⚡ BEST ALL-ROUNDER`, `🧠 BEST DEEP THINKER`)
    - Model friendly name & author
    - Plain-English explanation of why to pick it + transparent price (`Free ($0)` or `$X / 1M tokens`)
    - Prominent primary button: **`Use This Model →`**
  - Below the cards: **`Browse All 300+ Models in Catalog`** button.
- **State 3 — Model Active (`active.is_some()`):**
  - Shows **`✓ Setup Complete! Your AI is Ready`** with the active model's name, connection, and key preview.
  - Two clear actions:
    1. Primary button: **`Start Chatting Now →`** (switches to `NavTab::Chat` and focuses composer).
    2. Secondary button: **`Take 1-Minute Interactive System Tour`** (starts `TourStep::WelcomeChat`).
  - Also displays the Curated Recommendation Cards below under **"Switch to Another Recommended Model"** so changing models later is still 1 click.

### 3.4 Catalog Row Direct Activation & Details Panel (`models_view.rs`, `model_details.rs`)
- **Every row in `models_view.rs`** gets an explicit right-aligned action control alongside the favorite star:
  - If `active`: pill badge **`✓ Active`**
  - If `needs_key`: ghost button **`Add Key`** (opens key form)
  - If `!model.can_chat()`: pill badge **`Non-chat`**
  - Otherwise: button **`Use Model`** (`id: ("model-use-row", index)`) — clicking it immediately activates the model (`choose_model`)!
- **In `model_details.rs`:**
  - When a model is active, shows a primary **`Go to Chat with This Model →`** button right next to `✓ Currently in use`.

### 3.5 OpenRouter Key Editor Enhancements (`settings_view.rs`)
- In `render_key_form`, add a direct **`Open openrouter.ai/keys in Browser ↗`** button right beside Step 1 so the user doesn't even have to type the URL into their browser.
- When a key is saved and no model is selected yet, highlight the **`Next Step: Pick Your AI Model →`** button as the primary CTA (`design::primary_button`).

---

## 4. Error Handling & Edge Cases
- **Minimum Window Bounds (`640x420`):** The top coach bar and setup cards use compact padding and flex wrapping on narrow viewports so primary action buttons (`models-home-connect`, `models-browse-all`, etc.) remain visible without clipping.
- **Dark & Light Themes:** All surfaces, badges, and buttons use `cx.theme()` and `design::tokens(cx)` exclusively (verified by `cargo xtask check-styles`).
- **Offline / Empty Catalog with Key Saved:** If a key is saved but the catalog is still loading or empty, Step 2 shows a clear loading/refresh state and a fallback 1-click button to activate `openrouter/free` or browse the catalog.

## 5. Testing Strategy
1. **`sovra-core` Unit Tests:**
   - Verify `ModelsState::setup_stage()` across all combinations (unconnected, key saved without model, model selected without key, fully ready).
   - Verify `llm_setup_widget_for_stage` only emits the remaining step when a key is already saved.
   - Verify `recommended_model_cards()` selects free, everyday, and reasoning models deterministically.
   - Verify `WorkspaceState` tour step progression (`start_tour`, `next_tour_step`, `prev_tour_step`, `end_tour`) and tab synchronization.
2. **`sovra-ui` Headless GPUI Tests:**
   - Test that when an OpenRouter key is set, Chat setup card and Models setup screen only show Step 2 (model selection) and curates recommendation cards.
   - Test clicking `Use Model` directly on a catalog row activates the model.
   - Test the 5-step interactive system tour navigates across `Chat` → `My AI's` → `Knowledge` → `Schedules` → `Settings` and completes cleanly.
   - Verify all existing tests in `tests/models.rs`, `tests/shell.rs`, and `tests/knowledge.rs` pass, plus `cargo xtask ci`.
