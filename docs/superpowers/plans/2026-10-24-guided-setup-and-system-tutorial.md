# Guided Setup, State-Aware Model Selection & Interactive System Tutorial Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a zero-confusion, state-aware AI setup flow (that only asks for what is still missing), curated 1-click model recommendations with direct catalog row activation, and a 5-step interactive system tutorial with a live cross-tab coach bar and sidebar spotlight indicators.

**Architecture:**
1. Pure domain types (`SetupStage`, `RecommendationKind`, `RecommendedModelCard`, `TourStep`, and state-aware `llm_setup_widget_for_stage`) live in `sovra-core` and are unit-tested without IO or UI.
2. `sovra-runtime` persists `onboarding_completed` in `ModelPreferences` when `ModelCommand::SetOnboardingCompleted` is dispatched.
3. `sovra-ui` renders a top walkthrough/coach bar (`onboarding_bar.rs`), sidebar spotlight badges (`sidebar.rs`), a progressive 3-step `model_home.rs` with curated recommendation cards, direct `Use Model` row buttons in `models_view.rs`, and a 1-click browser launcher for `openrouter.ai/keys` in `settings_view.rs` and `native_surface.rs`.

**Tech Stack:** Rust 1.99.1 (edition 2024), `sovra-core`, `sovra-runtime`, `sovra-ui` (`gpui-kit = 0.8.0`).

---

### Task 1: State-Aware `SetupStage`, Curated Recommendations & `TourStep` in `sovra-core`

**Files:**
- Modify: `crates/sovra-core/src/models.rs`
- Modify: `crates/sovra-core/src/models/tests.rs`
- Modify: `crates/sovra-core/src/agui.rs`
- Modify: `crates/sovra-core/src/workspace.rs`
- Modify: `crates/sovra-core/src/lib.rs`

- [ ] **Step 1: Write failing unit tests in `crates/sovra-core/src/models/tests.rs` and `crates/sovra-core/src/workspace.rs`**
  - Test `ModelsState::setup_stage()` returns `NeedsConnection` when no key is set, `NeedsModelSelection` when an OpenRouter key is set (`has_credential = true`) and no model is active, `NeedsModelKey` when a model is selected on a keyless connection, and `Ready` when both are satisfied.
  - Test `ModelsState::recommended_model_cards()` returns up to 3 curated cards (`FreeStarter`, `EverydaySmart`, `DeepReasoning`).
  - Test `llm_setup_widget_for_stage` when in `NeedsModelSelection` does NOT emit Step 1 connection setup cards (`setup:openrouter-key`, `setup:ollama`, `setup:openai-url`) and instead only shows Step 2 model selection actions.
  - Test `WorkspaceState` tour step lifecycle (`start_tour`, `next_tour_step`, `prev_tour_step`, `end_tour`) updates `active_tab` to match each `TourStep`.

- [ ] **Step 2: Run `cargo test -p sovra-core` to verify tests fail**

- [ ] **Step 3: Implement domain types and pure reducers in `sovra-core` and `sovra-runtime`**
  - Add `SetupStage`, `RecommendationKind`, `RecommendedModelCard`, `ModelsState::setup_stage()`, `ModelsState::recommended_model_cards()`, `ModelPreferences::onboarding_completed`, and `ModelCommand::SetOnboardingCompleted { completed: bool }` in `crates/sovra-core/src/models.rs`.
  - Handle `ModelCommand::SetOnboardingCompleted` in `crates/sovra-runtime/src/models.rs`.
  - Add `llm_setup_widget_for_stage` in `crates/sovra-core/src/agui.rs` and wire `WorkspaceState::submit_prompt` in `crates/sovra-core/src/workspace.rs` to use the current `SetupStage` so users who already set a key only see the model selection step in chat.
  - Add `TourStep` enum (`WelcomeChat`, `AutomaticRouting`, `KnowledgeMemory`, `AutomatedSchedules`, `PersonalizeSettings`) and tour navigation methods on `WorkspaceState`.

- [ ] **Step 4: Run `cargo test -p sovra-core -p sovra-runtime` and verify all tests pass**

---

### Task 2: State-Aware Setup Home, Curated Recommendation Cards & Direct Catalog Row Activation in `sovra-ui`

**Files:**
- Modify: `crates/sovra-ui/src/model_home.rs`
- Modify: `crates/sovra-ui/src/models_view.rs`
- Modify: `crates/sovra-ui/src/model_details.rs`
- Modify: `crates/sovra-ui/src/settings_view.rs`

- [ ] **Step 1: Write failing UI tests in `crates/sovra-ui/tests/models.rs`**
  - Test that when an OpenRouter key is saved (`has_credential = true`), `models-setup-home` hides the "Connect an AI service" Step 1 card, shows a completed Step 1 badge (`models-home-step1-done`), and displays curated recommendation cards (`models-home-rec-0`, etc.).
  - Test that in the catalog (`sovra-models-view`), clicking `("model-use-row", index)` directly activates a chat model without opening the details panel first.
  - Test that `settings-key-form` has an `openrouter-open-browser` button to launch `https://openrouter.ai/keys`.

- [ ] **Step 2: Implement UI enhancements in `model_home.rs`, `models_view.rs`, `model_details.rs`, and `settings_view.rs`**
  - Update `model_home.rs` with a clear 3-step visual progress bar (`1. Connect AI` → `2. Pick Model` → `3. Start Chatting`), state-aware cards (collapsing Step 1 when connected), a 1-click `Open openrouter.ai/keys` button, curated recommendation cards, and a `Start Interactive System Tour` button.
  - Update `models_view.rs` `row()` to include a direct `Use Model` / `✓ Active` / `Add Key` action button (`("model-use-row", index)`) on every row, and add a `Go to Chat` button in `render_active_model_banner`.
  - Update `model_details.rs` to show a `Go to Chat` button when the inspected model is active.
  - Update `settings_view.rs` to include a 1-click `Open openrouter.ai/keys in Browser` button in `render_key_form` and highlight `Next: choose an AI` as primary once the key is saved.

- [ ] **Step 3: Run `cargo test -p sovra-ui --test models` and verify all tests pass (including minimum window 640x420 checks)**

---

### Task 3: Top Guided Setup & Interactive 5-Step System Tutorial Coach Bar (`onboarding_bar.rs`)

**Files:**
- Create: `crates/sovra-ui/src/onboarding_bar.rs`
- Modify: `crates/sovra-ui/src/lib.rs`
- Modify: `crates/sovra-ui/src/shell.rs`
- Modify: `crates/sovra-ui/src/sidebar.rs`
- Modify: `crates/sovra-ui/src/native_surface.rs`
- Modify: `crates/sovra-ui/tests/shell.rs`

- [ ] **Step 1: Write failing UI tests in `crates/sovra-ui/tests/shell.rs`**
  - Test that when a user has set an OpenRouter key (`has_credential = true`) but has not picked a model, sending a message in Chat renders a state-aware setup card that ONLY shows Step 2 (Pick Model) and does NOT show Step 1 (`surface-setup-openrouter-key` is absent, while `surface-setup-choose-model` is visible).
  - Test the 5-step interactive system tour (`start_system_tour`, clicking `tour-next-btn` across all 5 steps: `Chat` → `MyAis` → `Knowledge` → `Schedules` → `Settings`, and completing the tour).

- [ ] **Step 2: Implement `onboarding_bar.rs`, `sidebar.rs` spotlight badges, and `shell.rs` integration**
  - Create `crates/sovra-ui/src/onboarding_bar.rs` rendering:
    - The 5-step Interactive System Tour coach bar when `state.tour_step.is_some()`, with step badge (`STEP X OF 5`), title, plain-English walkthrough copy, step-specific helper CTA (e.g. `Try a Sample Prompt` on Step 1), `Previous`, `Next Step →` / `Finish Tour`, and `Skip Tour`.
    - A compact, non-intrusive Guided Setup Strip when `!state.has_active_llm && state.models_loaded` showing the exact next step (`Step 1 of 2: Connect Key` or `✓ Key Saved · Step 2 of 2: Pick a Model`) with a 1-click button to complete it or start the tour.
  - Add a spotlight indicator badge on the active tour tab in `crates/sovra-ui/src/sidebar.rs` plus a `Guided Tour` button in the sidebar footer.
  - Handle new `setup:` actions (`setup:open-openrouter-web`, `setup:activate-recommended`, `setup:start-tour`) in `crates/sovra-ui/src/shell.rs` and `crates/sovra-ui/src/native_surface.rs`.

- [ ] **Step 3: Run `cargo xtask ci` and update `CHANGELOG.md` and `docs/models.md`**
  - Verify formatting, clippy, layering (`check-deps`), styling (`check-styles`), and all unit/integration/UI tests pass.
