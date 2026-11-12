//! User-facing language: identity, world catalog, chrome lookup, RTL, and the
//! agent reply-language clause. English is the product default, so a new person
//! can always read the first screen; Arabic and every Settings language are one
//! choice away (also on the sign-in screen).

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// BCP-47 tag identifying the UI and agent reply language.
///
/// Unknown tags stay choosable so the agent can still reply in that language.
/// Tags outside the Settings catalog fall back to English chrome so the screen
/// never shows empty text or raw keys. Every catalog language has its own chrome.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UiLanguage(String);

impl Default for UiLanguage {
    fn default() -> Self {
        Self::english()
    }
}

impl UiLanguage {
    /// Complete chrome catalog, written right to left.
    #[must_use]
    pub fn arabic() -> Self {
        Self("ar".into())
    }

    /// Product default and complete chrome catalog.
    #[must_use]
    pub fn english() -> Self {
        Self("en".into())
    }

    /// Parse a saved or chosen tag. Empty input becomes English. Catalog tags are
    /// canonicalized; anything else is kept in lowercase so the agent can name it.
    #[must_use]
    pub fn from_tag(tag: impl AsRef<str>) -> Self {
        let raw = tag.as_ref().trim().replace('_', "-");
        if raw.is_empty() {
            return Self::english();
        }
        if let Some(info) = world_languages()
            .iter()
            .find(|info| info.tag.eq_ignore_ascii_case(&raw))
        {
            return Self(info.tag.to_owned());
        }
        Self(raw.to_ascii_lowercase())
    }

    #[must_use]
    pub fn tag(&self) -> &str {
        &self.0
    }

    /// Primary subtag (`zh-Hant` → `zh`).
    #[must_use]
    pub fn primary(&self) -> &str {
        self.0
            .split(['-', '_'])
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or("en")
    }

    #[must_use]
    pub fn info(&self) -> Option<&'static LanguageInfo> {
        world_languages()
            .iter()
            .find(|info| info.tag.eq_ignore_ascii_case(&self.0))
    }

    #[must_use]
    pub fn english_name(&self) -> &str {
        self.info().map_or(self.tag(), |info| info.english_name)
    }

    #[must_use]
    pub fn native_name(&self) -> &str {
        self.info().map_or(self.tag(), |info| info.native_name)
    }

    #[must_use]
    pub fn is_rtl(&self) -> bool {
        self.info().map_or_else(
            || {
                matches!(
                    self.primary(),
                    "ar" | "he" | "fa" | "ur" | "ps" | "sd" | "ug" | "yi" | "dv" | "ckb"
                )
            },
            |info| info.rtl,
        )
    }

    /// Standing-instruction clause the model must follow for every reply.
    #[must_use]
    pub fn reply_instruction(&self) -> String {
        format!(
            "Reply language: Always reply in {} ({}). Write every user-visible answer in that language unless the user explicitly asks for a different language.",
            self.english_name(),
            self.native_name()
        )
    }
}

impl Serialize for UiLanguage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for UiLanguage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let tag = String::deserialize(deserializer)?;
        Ok(Self::from_tag(tag))
    }
}

/// One choosable language in the world catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageInfo {
    pub tag: &'static str,
    pub english_name: &'static str,
    pub native_name: &'static str,
    pub rtl: bool,
}

impl LanguageInfo {
    /// Label for the Settings picker: native name, with English in parentheses when they differ.
    #[must_use]
    pub fn picker_label(self) -> String {
        if self.native_name == self.english_name {
            self.english_name.to_owned()
        } else {
            format!("{} ({})", self.native_name, self.english_name)
        }
    }
}

/// User-visible chrome strings, resolved in the chosen language (English by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChromeKey {
    LoginWelcomeTitle,
    LoginWelcomeSubtitle,
    LoginCreateTitle,
    LoginCreateSubtitle,
    LoginEmail,
    LoginPassword,
    LoginSignIn,
    LoginCreateStart,
    LoginSigningIn,
    LoginCreating,
    LoginGoogle,
    LoginHintSignIn,
    LoginHintCreate,
    LoginGoogleWaiting,
    LoginGoogleWaitingHint,
    LoginOpenAgain,
    LoginCancel,
    NavChat,
    NavTeam,
    NavArtifacts,
    NavSchedules,
    NavModels,
    NavMyAis,
    NavKnowledge,
    NavSkills,
    NavSettings,
    NavConnections,
    NavChatTooltip,
    NavTeamTooltip,
    NavArtifactsTooltip,
    NavSchedulesTooltip,
    NavModelsTooltip,
    NavMyAisTooltip,
    NavKnowledgeTooltip,
    NavSkillsTooltip,
    NavSettingsTooltip,
    NavConnectionsTooltip,
    NavGroupAiSetup,
    NavNewChat,
    NavNewChatHint,
    NavTour,
    NavTourHint,
    NavCollapse,
    NavExpand,
    NavSetupBadge,
    SettingsTitle,
    SettingsSubtitle,
    SettingsTour,
    SettingsAppearanceTitle,
    SettingsAppearanceDesc,
    SettingsLight,
    SettingsDark,
    SettingsLightA11y,
    SettingsDarkA11y,
    SettingsLanguageTitle,
    SettingsLanguageDesc,
    SettingsTextSize,
    SettingsTextSmaller,
    SettingsTextLarger,
    SettingsConnectionsTitle,
    SettingsConnectionsDesc,
    SettingsOpenConnections,
    ChatSend,
    ChatQueue,
    ChatStop,
    ChatAddFiles,
    ChatComposerA11y,
    ChatComposerPlaceholder,
    ChatWelcomeTitle,
    ChatWelcomeGuest,
    ChatWelcomeSubtitle,
    ChatStarterUnderstand,
    ChatStarterUnderstandDraft,
    ChatStarterEmail,
    ChatStarterEmailDraft,
    ChatStarterPlan,
    ChatStarterPlanDraft,
    ChatStarterSummarize,
    ChatStarterSummarizeDraft,
    ChatHintIdle,
    ChatHintGenerating,
    ChatHintFiles,
    ChatHintFilesGenerating,
    LoginEmailPlaceholder,
    LoginPasswordPlaceholder,
    NavGroups,
    NavGroupsTooltip,
    LoginName,
    LoginNamePlaceholder,
}

impl ChromeKey {
    pub const ALL: [Self; 89] = [
        Self::LoginWelcomeTitle,
        Self::LoginWelcomeSubtitle,
        Self::LoginCreateTitle,
        Self::LoginCreateSubtitle,
        Self::LoginEmail,
        Self::LoginPassword,
        Self::LoginSignIn,
        Self::LoginCreateStart,
        Self::LoginSigningIn,
        Self::LoginCreating,
        Self::LoginGoogle,
        Self::LoginHintSignIn,
        Self::LoginHintCreate,
        Self::LoginGoogleWaiting,
        Self::LoginGoogleWaitingHint,
        Self::LoginOpenAgain,
        Self::LoginCancel,
        Self::NavChat,
        Self::NavTeam,
        Self::NavArtifacts,
        Self::NavSchedules,
        Self::NavModels,
        Self::NavMyAis,
        Self::NavKnowledge,
        Self::NavSkills,
        Self::NavSettings,
        Self::NavConnections,
        Self::NavChatTooltip,
        Self::NavTeamTooltip,
        Self::NavArtifactsTooltip,
        Self::NavSchedulesTooltip,
        Self::NavModelsTooltip,
        Self::NavMyAisTooltip,
        Self::NavKnowledgeTooltip,
        Self::NavSkillsTooltip,
        Self::NavSettingsTooltip,
        Self::NavConnectionsTooltip,
        Self::NavGroupAiSetup,
        Self::NavNewChat,
        Self::NavNewChatHint,
        Self::NavTour,
        Self::NavTourHint,
        Self::NavCollapse,
        Self::NavExpand,
        Self::NavSetupBadge,
        Self::SettingsTitle,
        Self::SettingsSubtitle,
        Self::SettingsTour,
        Self::SettingsAppearanceTitle,
        Self::SettingsAppearanceDesc,
        Self::SettingsLight,
        Self::SettingsDark,
        Self::SettingsLightA11y,
        Self::SettingsDarkA11y,
        Self::SettingsLanguageTitle,
        Self::SettingsLanguageDesc,
        Self::SettingsTextSize,
        Self::SettingsTextSmaller,
        Self::SettingsTextLarger,
        Self::SettingsConnectionsTitle,
        Self::SettingsConnectionsDesc,
        Self::SettingsOpenConnections,
        Self::ChatSend,
        Self::ChatQueue,
        Self::ChatStop,
        Self::ChatAddFiles,
        Self::ChatComposerA11y,
        Self::ChatComposerPlaceholder,
        Self::ChatWelcomeTitle,
        Self::ChatWelcomeGuest,
        Self::ChatWelcomeSubtitle,
        Self::ChatStarterUnderstand,
        Self::ChatStarterUnderstandDraft,
        Self::ChatStarterEmail,
        Self::ChatStarterEmailDraft,
        Self::ChatStarterPlan,
        Self::ChatStarterPlanDraft,
        Self::ChatStarterSummarize,
        Self::ChatStarterSummarizeDraft,
        Self::ChatHintIdle,
        Self::ChatHintGenerating,
        Self::ChatHintFiles,
        Self::ChatHintFilesGenerating,
        Self::LoginEmailPlaceholder,
        Self::LoginPasswordPlaceholder,
        Self::NavGroups,
        Self::NavGroupsTooltip,
        Self::LoginName,
        Self::LoginNamePlaceholder,
    ];
}

/// Chrome string for `language`. English and Arabic have their own catalogs.
/// Every other Settings language has its own wording. A tag outside the catalog
/// falls back to English so keys never appear.
#[must_use]
pub fn chrome(language: &UiLanguage, key: ChromeKey) -> &'static str {
    let text = if language.tag() == "en" {
        english(key)
    } else if language.tag() == "ar" {
        arabic(key)
    } else if let Some(text) = chrome_catalog::localized(language.tag(), key) {
        text
    } else {
        english(key)
    };
    if text.is_empty() { english(key) } else { text }
}

#[path = "chrome_catalog.rs"]
mod chrome_catalog;

/// The first thing a new person sees before any AI is connected: one step at a
/// time, in words anyone can follow. `{service}` is replaced with the service name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FirstRunKey {
    StepOne,
    StepTwo,
    ConnectTitle,
    ConnectBody,
    ConnectAction,
    LocalAction,
    PickTitle,
    PickBody,
    PickAction,
    KeyTitle,
    KeyBody,
    KeyAction,
}

impl FirstRunKey {
    pub const ALL: [Self; 12] = [
        Self::StepOne,
        Self::StepTwo,
        Self::ConnectTitle,
        Self::ConnectBody,
        Self::ConnectAction,
        Self::LocalAction,
        Self::PickTitle,
        Self::PickBody,
        Self::PickAction,
        Self::KeyTitle,
        Self::KeyBody,
        Self::KeyAction,
    ];
}

/// First-run guidance for `language`. English and Arabic have their own wording;
/// other languages read English until their packs are translated.
#[must_use]
pub fn first_run_text(language: &UiLanguage, key: FirstRunKey) -> &'static str {
    let arabic = language.primary() == "ar";
    match (key, arabic) {
        (FirstRunKey::StepOne, false) => "Step 1 of 2",
        (FirstRunKey::StepOne, true) => "الخطوة 1 من 2",
        (FirstRunKey::StepTwo, false) => "Step 2 of 2",
        (FirstRunKey::StepTwo, true) => "الخطوة 2 من 2",
        (FirstRunKey::ConnectTitle, false) => "First, connect an AI",
        (FirstRunKey::ConnectTitle, true) => "أولاً، اربط ذكاءً اصطناعياً",
        (FirstRunKey::ConnectBody, false) => {
            "Sovra needs an AI to answer you. It takes about two minutes, and there are free \
             choices. After that, just type your question below."
        }
        (FirstRunKey::ConnectBody, true) => {
            "يحتاج Sovra إلى ذكاء اصطناعي ليجيبك. يستغرق ذلك دقيقتين تقريباً، وهناك خيارات \
             مجانية. بعد ذلك اكتب سؤالك في الأسفل."
        }
        (FirstRunKey::ConnectAction, false) => "Connect an AI",
        (FirstRunKey::ConnectAction, true) => "اربط ذكاءً اصطناعياً",
        (FirstRunKey::LocalAction, false) => "Use a free AI on this computer",
        (FirstRunKey::LocalAction, true) => "استخدم ذكاءً اصطناعياً مجانياً على هذا الحاسوب",
        (FirstRunKey::PickTitle, false) => "Next, pick the AI you want to talk to",
        (FirstRunKey::PickTitle, true) => "بعد ذلك، اختر الذكاء الاصطناعي الذي تريد التحدث إليه",
        (FirstRunKey::PickBody, false) => {
            "{service} is connected. Choose one AI and you can start chatting."
        }
        (FirstRunKey::PickBody, true) => {
            "تم ربط {service}. اختر ذكاءً اصطناعياً واحداً لتبدأ المحادثة."
        }
        (FirstRunKey::PickAction, false) => "Pick your AI",
        (FirstRunKey::PickAction, true) => "اختر الذكاء الاصطناعي",
        (FirstRunKey::KeyTitle, false) => "Last step: add your key",
        (FirstRunKey::KeyTitle, true) => "الخطوة الأخيرة: أضف مفتاحك",
        (FirstRunKey::KeyBody, false) => {
            "Your AI is chosen. Add your {service} key and you can start chatting."
        }
        (FirstRunKey::KeyBody, true) => {
            "تم اختيار الذكاء الاصطناعي. أضف مفتاح {service} لتبدأ المحادثة."
        }
        (FirstRunKey::KeyAction, false) => "Add your key",
        (FirstRunKey::KeyAction, true) => "أضف مفتاحك",
    }
}

/// Every language the Settings picker offers.
#[must_use]
pub fn world_languages() -> &'static [LanguageInfo] {
    CATALOG
}

const CATALOG: &[LanguageInfo] = &[
    LanguageInfo {
        tag: "ar",
        english_name: "Arabic",
        native_name: "العربية",
        rtl: true,
    },
    LanguageInfo {
        tag: "en",
        english_name: "English",
        native_name: "English",
        rtl: false,
    },
    LanguageInfo {
        tag: "fr",
        english_name: "French",
        native_name: "Français",
        rtl: false,
    },
    LanguageInfo {
        tag: "es",
        english_name: "Spanish",
        native_name: "Español",
        rtl: false,
    },
    LanguageInfo {
        tag: "zh",
        english_name: "Chinese",
        native_name: "中文",
        rtl: false,
    },
    LanguageInfo {
        tag: "zh-Hant",
        english_name: "Chinese (Traditional)",
        native_name: "繁體中文",
        rtl: false,
    },
    LanguageInfo {
        tag: "ja",
        english_name: "Japanese",
        native_name: "日本語",
        rtl: false,
    },
    LanguageInfo {
        tag: "hi",
        english_name: "Hindi",
        native_name: "हिन्दी",
        rtl: false,
    },
    LanguageInfo {
        tag: "pt",
        english_name: "Portuguese",
        native_name: "Português",
        rtl: false,
    },
    LanguageInfo {
        tag: "ru",
        english_name: "Russian",
        native_name: "Русский",
        rtl: false,
    },
    LanguageInfo {
        tag: "de",
        english_name: "German",
        native_name: "Deutsch",
        rtl: false,
    },
    LanguageInfo {
        tag: "it",
        english_name: "Italian",
        native_name: "Italiano",
        rtl: false,
    },
    LanguageInfo {
        tag: "ko",
        english_name: "Korean",
        native_name: "한국어",
        rtl: false,
    },
    LanguageInfo {
        tag: "tr",
        english_name: "Turkish",
        native_name: "Türkçe",
        rtl: false,
    },
    LanguageInfo {
        tag: "nl",
        english_name: "Dutch",
        native_name: "Nederlands",
        rtl: false,
    },
    LanguageInfo {
        tag: "pl",
        english_name: "Polish",
        native_name: "Polski",
        rtl: false,
    },
    LanguageInfo {
        tag: "vi",
        english_name: "Vietnamese",
        native_name: "Tiếng Việt",
        rtl: false,
    },
    LanguageInfo {
        tag: "th",
        english_name: "Thai",
        native_name: "ไทย",
        rtl: false,
    },
    LanguageInfo {
        tag: "id",
        english_name: "Indonesian",
        native_name: "Bahasa Indonesia",
        rtl: false,
    },
    LanguageInfo {
        tag: "ms",
        english_name: "Malay",
        native_name: "Bahasa Melayu",
        rtl: false,
    },
    LanguageInfo {
        tag: "uk",
        english_name: "Ukrainian",
        native_name: "Українська",
        rtl: false,
    },
    LanguageInfo {
        tag: "sv",
        english_name: "Swedish",
        native_name: "Svenska",
        rtl: false,
    },
    LanguageInfo {
        tag: "da",
        english_name: "Danish",
        native_name: "Dansk",
        rtl: false,
    },
    LanguageInfo {
        tag: "fi",
        english_name: "Finnish",
        native_name: "Suomi",
        rtl: false,
    },
    LanguageInfo {
        tag: "no",
        english_name: "Norwegian",
        native_name: "Norsk",
        rtl: false,
    },
    LanguageInfo {
        tag: "cs",
        english_name: "Czech",
        native_name: "Čeština",
        rtl: false,
    },
    LanguageInfo {
        tag: "el",
        english_name: "Greek",
        native_name: "Ελληνικά",
        rtl: false,
    },
    LanguageInfo {
        tag: "he",
        english_name: "Hebrew",
        native_name: "עברית",
        rtl: true,
    },
    LanguageInfo {
        tag: "fa",
        english_name: "Persian",
        native_name: "فارسی",
        rtl: true,
    },
    LanguageInfo {
        tag: "ur",
        english_name: "Urdu",
        native_name: "اردو",
        rtl: true,
    },
    LanguageInfo {
        tag: "bn",
        english_name: "Bengali",
        native_name: "বাংলা",
        rtl: false,
    },
    LanguageInfo {
        tag: "ta",
        english_name: "Tamil",
        native_name: "தமிழ்",
        rtl: false,
    },
    LanguageInfo {
        tag: "te",
        english_name: "Telugu",
        native_name: "తెలుగు",
        rtl: false,
    },
    LanguageInfo {
        tag: "mr",
        english_name: "Marathi",
        native_name: "मराठी",
        rtl: false,
    },
    LanguageInfo {
        tag: "gu",
        english_name: "Gujarati",
        native_name: "ગુજરાતી",
        rtl: false,
    },
    LanguageInfo {
        tag: "kn",
        english_name: "Kannada",
        native_name: "ಕನ್ನಡ",
        rtl: false,
    },
    LanguageInfo {
        tag: "ml",
        english_name: "Malayalam",
        native_name: "മലയാളം",
        rtl: false,
    },
    LanguageInfo {
        tag: "pa",
        english_name: "Punjabi",
        native_name: "ਪੰਜਾਬੀ",
        rtl: false,
    },
    LanguageInfo {
        tag: "sw",
        english_name: "Swahili",
        native_name: "Kiswahili",
        rtl: false,
    },
    LanguageInfo {
        tag: "am",
        english_name: "Amharic",
        native_name: "አማርኛ",
        rtl: false,
    },
    LanguageInfo {
        tag: "ha",
        english_name: "Hausa",
        native_name: "Hausa",
        rtl: false,
    },
    LanguageInfo {
        tag: "yo",
        english_name: "Yoruba",
        native_name: "Yorùbá",
        rtl: false,
    },
    LanguageInfo {
        tag: "ig",
        english_name: "Igbo",
        native_name: "Igbo",
        rtl: false,
    },
    LanguageInfo {
        tag: "zu",
        english_name: "Zulu",
        native_name: "isiZulu",
        rtl: false,
    },
    LanguageInfo {
        tag: "af",
        english_name: "Afrikaans",
        native_name: "Afrikaans",
        rtl: false,
    },
    LanguageInfo {
        tag: "ro",
        english_name: "Romanian",
        native_name: "Română",
        rtl: false,
    },
    LanguageInfo {
        tag: "hu",
        english_name: "Hungarian",
        native_name: "Magyar",
        rtl: false,
    },
    LanguageInfo {
        tag: "bg",
        english_name: "Bulgarian",
        native_name: "Български",
        rtl: false,
    },
    LanguageInfo {
        tag: "hr",
        english_name: "Croatian",
        native_name: "Hrvatski",
        rtl: false,
    },
    LanguageInfo {
        tag: "sk",
        english_name: "Slovak",
        native_name: "Slovenčina",
        rtl: false,
    },
    LanguageInfo {
        tag: "sl",
        english_name: "Slovenian",
        native_name: "Slovenščina",
        rtl: false,
    },
    LanguageInfo {
        tag: "lt",
        english_name: "Lithuanian",
        native_name: "Lietuvių",
        rtl: false,
    },
    LanguageInfo {
        tag: "lv",
        english_name: "Latvian",
        native_name: "Latviešu",
        rtl: false,
    },
    LanguageInfo {
        tag: "et",
        english_name: "Estonian",
        native_name: "Eesti",
        rtl: false,
    },
    LanguageInfo {
        tag: "sr",
        english_name: "Serbian",
        native_name: "Српски",
        rtl: false,
    },
    LanguageInfo {
        tag: "bs",
        english_name: "Bosnian",
        native_name: "Bosanski",
        rtl: false,
    },
    LanguageInfo {
        tag: "mk",
        english_name: "Macedonian",
        native_name: "Македонски",
        rtl: false,
    },
    LanguageInfo {
        tag: "sq",
        english_name: "Albanian",
        native_name: "Shqip",
        rtl: false,
    },
    LanguageInfo {
        tag: "ka",
        english_name: "Georgian",
        native_name: "ქართული",
        rtl: false,
    },
    LanguageInfo {
        tag: "hy",
        english_name: "Armenian",
        native_name: "Հայերեն",
        rtl: false,
    },
    LanguageInfo {
        tag: "az",
        english_name: "Azerbaijani",
        native_name: "Azərbaycan",
        rtl: false,
    },
    LanguageInfo {
        tag: "kk",
        english_name: "Kazakh",
        native_name: "Қазақша",
        rtl: false,
    },
    LanguageInfo {
        tag: "uz",
        english_name: "Uzbek",
        native_name: "Oʻzbekcha",
        rtl: false,
    },
    LanguageInfo {
        tag: "mn",
        english_name: "Mongolian",
        native_name: "Монгол",
        rtl: false,
    },
    LanguageInfo {
        tag: "ne",
        english_name: "Nepali",
        native_name: "नेपाली",
        rtl: false,
    },
    LanguageInfo {
        tag: "si",
        english_name: "Sinhala",
        native_name: "සිංහල",
        rtl: false,
    },
    LanguageInfo {
        tag: "my",
        english_name: "Burmese",
        native_name: "မြန်မာ",
        rtl: false,
    },
    LanguageInfo {
        tag: "km",
        english_name: "Khmer",
        native_name: "ខ្មែរ",
        rtl: false,
    },
    LanguageInfo {
        tag: "lo",
        english_name: "Lao",
        native_name: "ລາວ",
        rtl: false,
    },
    LanguageInfo {
        tag: "tl",
        english_name: "Tagalog",
        native_name: "Tagalog",
        rtl: false,
    },
    LanguageInfo {
        tag: "ca",
        english_name: "Catalan",
        native_name: "Català",
        rtl: false,
    },
    LanguageInfo {
        tag: "eu",
        english_name: "Basque",
        native_name: "Euskara",
        rtl: false,
    },
    LanguageInfo {
        tag: "gl",
        english_name: "Galician",
        native_name: "Galego",
        rtl: false,
    },
    LanguageInfo {
        tag: "cy",
        english_name: "Welsh",
        native_name: "Cymraeg",
        rtl: false,
    },
    LanguageInfo {
        tag: "ga",
        english_name: "Irish",
        native_name: "Gaeilge",
        rtl: false,
    },
    LanguageInfo {
        tag: "is",
        english_name: "Icelandic",
        native_name: "Íslenska",
        rtl: false,
    },
    LanguageInfo {
        tag: "mt",
        english_name: "Maltese",
        native_name: "Malti",
        rtl: false,
    },
    LanguageInfo {
        tag: "lb",
        english_name: "Luxembourgish",
        native_name: "Lëtzebuergesch",
        rtl: false,
    },
    LanguageInfo {
        tag: "be",
        english_name: "Belarusian",
        native_name: "Беларуская",
        rtl: false,
    },
    LanguageInfo {
        tag: "ps",
        english_name: "Pashto",
        native_name: "پښتو",
        rtl: true,
    },
    LanguageInfo {
        tag: "sd",
        english_name: "Sindhi",
        native_name: "سنڌي",
        rtl: true,
    },
    LanguageInfo {
        tag: "ku",
        english_name: "Kurdish",
        native_name: "Kurdî",
        rtl: false,
    },
    LanguageInfo {
        tag: "ckb",
        english_name: "Central Kurdish",
        native_name: "کوردیی ناوەندی",
        rtl: true,
    },
    LanguageInfo {
        tag: "yi",
        english_name: "Yiddish",
        native_name: "ייִדיש",
        rtl: true,
    },
    LanguageInfo {
        tag: "dv",
        english_name: "Dhivehi",
        native_name: "ދިވެހި",
        rtl: true,
    },
    LanguageInfo {
        tag: "ug",
        english_name: "Uyghur",
        native_name: "ئۇيغۇرچە",
        rtl: true,
    },
    LanguageInfo {
        tag: "so",
        english_name: "Somali",
        native_name: "Soomaali",
        rtl: false,
    },
    LanguageInfo {
        tag: "rw",
        english_name: "Kinyarwanda",
        native_name: "Kinyarwanda",
        rtl: false,
    },
    LanguageInfo {
        tag: "ht",
        english_name: "Haitian Creole",
        native_name: "Kreyòl ayisyen",
        rtl: false,
    },
    LanguageInfo {
        tag: "jv",
        english_name: "Javanese",
        native_name: "Basa Jawa",
        rtl: false,
    },
];

fn arabic(key: ChromeKey) -> &'static str {
    match key {
        ChromeKey::LoginWelcomeTitle => "مرحباً بعودتك",
        ChromeKey::LoginWelcomeSubtitle => "سجّل الدخول للمتابعة إلى Sovra.",
        ChromeKey::LoginCreateTitle => "أنشئ حسابك",
        ChromeKey::LoginCreateSubtitle => "كل شيء يبقى على هذا الحاسوب — لا حاجة لحساب سحابي.",
        ChromeKey::LoginEmail => "البريد الإلكتروني",
        ChromeKey::LoginPassword => "كلمة المرور",
        ChromeKey::LoginSignIn => "تسجيل الدخول إلى Sovra",
        ChromeKey::LoginCreateStart => "إنشاء الحساب والبدء",
        ChromeKey::LoginSigningIn => "جارٍ تسجيل الدخول…",
        ChromeKey::LoginCreating => "جارٍ إنشاء حسابك…",
        ChromeKey::LoginGoogle => "المتابعة باستخدام Google",
        ChromeKey::LoginHintSignIn => {
            "أول زيارة؟ أدخل اسمك لإنشاء حسابك. البريد وكلمة المرور اختياريان."
        }
        ChromeKey::LoginHintCreate => {
            "اسمك يكفي للبدء. أضف بريداً وكلمة مرور فقط إذا أردت تسجيل الدخول بهما لاحقاً. تبقى كلمة المرور على هذا الحاسوب."
        }
        ChromeKey::LoginGoogleWaiting => "أكمل تسجيل الدخول عبر Google في المتصفح.",
        ChromeKey::LoginGoogleWaitingHint => "وافق هناك ثم عد — تتحدّث هذه الشاشة من تلقاء نفسها.",
        ChromeKey::LoginOpenAgain => "افتح الصفحة مجدداً",
        ChromeKey::LoginCancel => "إلغاء",
        ChromeKey::NavChat => "محادثة",
        ChromeKey::NavTeam => "الفريق",
        ChromeKey::NavArtifacts => "المحفوظات",
        ChromeKey::NavSchedules => "الجداول",
        ChromeKey::NavModels => "النماذج",
        ChromeKey::NavMyAis => "مساعداي",
        ChromeKey::NavKnowledge => "المعرفة",
        ChromeKey::NavSkills => "المهارات",
        ChromeKey::NavSettings | ChromeKey::SettingsTitle => "الإعدادات",
        ChromeKey::NavConnections | ChromeKey::SettingsConnectionsTitle => "الاتصالات",
        ChromeKey::NavChatTooltip => "تحدّث مع Sovra بشكل طبيعي",
        ChromeKey::NavTeamTooltip => "اسأل فريق مساعدي الذكاء الاصطناعي",
        ChromeKey::NavArtifactsTooltip => "المستندات والرمز والجداول والملاحظات المحفوظة",
        ChromeKey::NavSchedulesTooltip => "المهام والإيجازات المحفوظة لتشغيلها في أي وقت",
        ChromeKey::NavModelsTooltip => "اربط واختر نموذج الذكاء الاصطناعي",
        ChromeKey::NavMyAisTooltip => "اختر كيف يختار Sovra أفضل ذكاء لكل مهمة",
        ChromeKey::NavKnowledgeTooltip => "شاهد كيف يتذكّر Sovra الحقائق ويقرأ ملفاتك",
        ChromeKey::NavSkillsTooltip => "اختصارات وسير عمل جاهزة",
        ChromeKey::NavSettingsTooltip => "المظهر وحجم النص وكيف يكلّمك Sovra",
        ChromeKey::NavConnectionsTooltip => {
            "اربط ذكاءً اصطناعياً أو واتساب أو تيليغرام أو Google أو خدمة تستخدمها"
        }
        ChromeKey::NavGroupAiSetup => "ذكاؤك والإعداد",
        ChromeKey::NavNewChat => "محادثة جديدة",
        ChromeKey::NavNewChatHint => "ابدأ محادثة جديدة",
        ChromeKey::NavTour => "خذ جولة إرشادية",
        ChromeKey::NavTourHint => "شرح خطوة بخطوة لكيفية عمل Sovra",
        ChromeKey::NavCollapse => "طي الشريط الجانبي",
        ChromeKey::NavExpand => "توسيع الشريط الجانبي",
        ChromeKey::NavSetupBadge => "إعداد",
        ChromeKey::SettingsSubtitle => "اجعل Sovra يناسبك.",
        ChromeKey::SettingsTour => "خذ جولة",
        ChromeKey::SettingsAppearanceTitle => "المظهر",
        ChromeKey::SettingsAppearanceDesc => "اختر مساحة عمل فاتحة أو داكنة.",
        ChromeKey::SettingsLight => "فاتح",
        ChromeKey::SettingsDark => "داكن",
        ChromeKey::SettingsLightA11y => "مظهر فاتح",
        ChromeKey::SettingsDarkA11y => "مظهر داكن",
        ChromeKey::SettingsLanguageTitle => "اللغة",
        ChromeKey::SettingsLanguageDesc => "اختر لغة الواجهة والردود. تُحفظ مثل المظهر.",
        ChromeKey::SettingsTextSize => "حجم النص",
        ChromeKey::SettingsTextSmaller => "تصغير النص",
        ChromeKey::SettingsTextLarger => "تكبير النص",
        ChromeKey::SettingsConnectionsDesc => {
            "الذكاء الاصطناعي وواتساب وتيليغرام وGoogle والخدمات التي تستخدمها لها صفحة خاصة."
        }
        ChromeKey::SettingsOpenConnections => "فتح الاتصالات",
        ChromeKey::ChatSend => "إرسال الرسالة",
        ChromeKey::ChatQueue => "وضع الرسالة في الانتظار",
        ChromeKey::ChatStop => "إيقاف التوليد",
        ChromeKey::ChatAddFiles => "إضافة ملفات",
        ChromeKey::ChatComposerA11y => "راسل Sovra",
        ChromeKey::ChatComposerPlaceholder => "راسل Sovra…",
        ChromeKey::ChatWelcomeTitle => "ما الذي يدور في بالك، {name}؟",
        ChromeKey::ChatWelcomeGuest => "يا صديقي",
        ChromeKey::ChatWelcomeSubtitle => "سؤال، فكرة، أو مساعدة بسيطة للبدء.",
        ChromeKey::ChatStarterUnderstand => "افهم شيئاً",
        ChromeKey::ChatStarterUnderstandDraft => "اشرح لي هذا ببساطة، كأنني جديد عليه: ",
        ChromeKey::ChatStarterEmail => "اكتب رسالة",
        ChromeKey::ChatStarterEmailDraft => "ساعدني في كتابة رسالة قصيرة وودية عن ",
        ChromeKey::ChatStarterPlan => "خطط لأسبوعي",
        ChromeKey::ChatStarterPlanDraft => "ساعدني في تخطيط أسبوعي. هذا ما أحتاج إنجازه: ",
        ChromeKey::ChatStarterSummarize => "لخّص مستنداً",
        ChromeKey::ChatStarterSummarizeDraft => "لخّص هذا في نقاط موجزة:\n\n",
        ChromeKey::ChatHintIdle => "اكتب / للمهارات · Enter للإرسال · Shift+Enter لسطر جديد",
        ChromeKey::ChatHintGenerating => "اكتب مسبقاً — Enter يضعها في الانتظار حتى ينتهي Sovra",
        ChromeKey::ChatHintFiles => {
            "حتى 8 ملفات · 20 ميغابايت لكل ملف · تُرسل إلى الذكاء الذي اخترته عند الإرسال"
        }
        ChromeKey::ChatHintFilesGenerating => "أرسل هذه الملفات بعد انتهاء هذا الرد",
        ChromeKey::LoginEmailPlaceholder => "you@example.com",
        ChromeKey::LoginPasswordPlaceholder => "أدخل كلمة المرور",
        ChromeKey::NavGroups => "المجموعات",
        ChromeKey::NavGroupsTooltip => "شارك مع عائلتك أو فريقك",
        ChromeKey::LoginName => "اسمك",
        ChromeKey::LoginNamePlaceholder => "اكتب اسمك",
    }
}

fn english(key: ChromeKey) -> &'static str {
    match key {
        ChromeKey::LoginWelcomeTitle => "Welcome back",
        ChromeKey::LoginWelcomeSubtitle => "Sign in to continue to Sovra.",
        ChromeKey::LoginCreateTitle => "Create your account",
        ChromeKey::LoginCreateSubtitle => {
            "Everything stays on this computer — no cloud account needed."
        }
        ChromeKey::LoginEmail => "Email address",
        ChromeKey::LoginPassword => "Password",
        ChromeKey::LoginSignIn => "Sign in to Sovra",
        ChromeKey::LoginCreateStart => "Create account & start",
        ChromeKey::LoginSigningIn => "Signing in…",
        ChromeKey::LoginCreating => "Creating your account…",
        ChromeKey::LoginGoogle => "Continue with Google",
        ChromeKey::LoginHintSignIn => {
            "First time here? Enter your name to create your account. Email and a password are optional."
        }
        ChromeKey::LoginHintCreate => {
            "Your name is enough to start. Add an email and password only if you want to sign in that way later. A password stays on this computer."
        }
        ChromeKey::LoginGoogleWaiting => "Finish signing in with Google in your browser.",
        ChromeKey::LoginGoogleWaitingHint => {
            "Approve there and come back — this screen updates by itself."
        }
        ChromeKey::LoginOpenAgain => "Open the page again",
        ChromeKey::LoginCancel => "Cancel",
        ChromeKey::NavChat => "Chat",
        ChromeKey::NavTeam => "Team",
        ChromeKey::NavArtifacts => "Artifacts",
        ChromeKey::NavSchedules => "Schedules",
        ChromeKey::NavModels => "Models",
        ChromeKey::NavMyAis => "My AI's",
        ChromeKey::NavKnowledge => "Knowledge",
        ChromeKey::NavSkills => "Skills",
        ChromeKey::NavSettings | ChromeKey::SettingsTitle => "Settings",
        ChromeKey::NavConnections | ChromeKey::SettingsConnectionsTitle => "Connections",
        ChromeKey::NavChatTooltip => "Chat naturally with Sovra",
        ChromeKey::NavTeamTooltip => "Ask your team of AI specialists",
        ChromeKey::NavArtifactsTooltip => "Saved documents, code, tables & notes",
        ChromeKey::NavSchedulesTooltip => "Saved tasks & briefings to run anytime",
        ChromeKey::NavModelsTooltip => "Connect & choose which AI model to use",
        ChromeKey::NavMyAisTooltip => "Choose how Sovra picks the best AI for each task",
        ChromeKey::NavKnowledgeTooltip => "See how Sovra remembers facts & reads your files",
        ChromeKey::NavSkillsTooltip => "Reusable /slash shortcuts & workflows",
        ChromeKey::NavSettingsTooltip => "Appearance, text size, and how Sovra talks to you",
        ChromeKey::NavConnectionsTooltip => {
            "Link an AI, WhatsApp, Telegram, Google, or a service you already use"
        }
        ChromeKey::NavGroupAiSetup => "Your AI & Setup",
        ChromeKey::NavNewChat => "New Chat",
        ChromeKey::NavNewChatHint => "Start a fresh conversation",
        ChromeKey::NavTour => "Take a guided tour",
        ChromeKey::NavTourHint => "Step-by-step walkthrough of how Sovra works",
        ChromeKey::NavCollapse => "Collapse sidebar",
        ChromeKey::NavExpand => "Expand sidebar and chat history",
        ChromeKey::NavSetupBadge => "Setup",
        ChromeKey::SettingsSubtitle => "Make Sovra feel like your own.",
        ChromeKey::SettingsTour => "Take a tour",
        ChromeKey::SettingsAppearanceTitle => "Appearance",
        ChromeKey::SettingsAppearanceDesc => "Choose a light or dark workspace.",
        ChromeKey::SettingsLight => "Light",
        ChromeKey::SettingsDark => "Dark",
        ChromeKey::SettingsLightA11y => "Light appearance",
        ChromeKey::SettingsDarkA11y => "Dark appearance",
        ChromeKey::SettingsLanguageTitle => "Language",
        ChromeKey::SettingsLanguageDesc => {
            "Choose the language for chrome and replies. Saved like appearance."
        }
        ChromeKey::SettingsTextSize => "Text size",
        ChromeKey::SettingsTextSmaller => "Make text smaller",
        ChromeKey::SettingsTextLarger => "Make text larger",
        ChromeKey::SettingsConnectionsDesc => {
            "AI, WhatsApp, Telegram, Google, and services you already use live on their own page."
        }
        ChromeKey::SettingsOpenConnections => "Open Connections",
        ChromeKey::ChatSend => "Send message",
        ChromeKey::ChatQueue => "Queue message",
        ChromeKey::ChatStop => "Stop generating",
        ChromeKey::ChatAddFiles => "Add files",
        ChromeKey::ChatComposerA11y => "Message Sovra",
        ChromeKey::ChatComposerPlaceholder => "Message Sovra…",
        ChromeKey::ChatWelcomeTitle => "What’s on your mind, {name}?",
        ChromeKey::ChatWelcomeGuest => "there",
        ChromeKey::ChatWelcomeSubtitle => "A question, an idea, a little help getting started.",
        ChromeKey::ChatStarterUnderstand => "Understand something",
        ChromeKey::ChatStarterUnderstandDraft => "Explain this to me simply, like I'm new to it: ",
        ChromeKey::ChatStarterEmail => "Draft an email",
        ChromeKey::ChatStarterEmailDraft => "Help me write a short, friendly email about ",
        ChromeKey::ChatStarterPlan => "Plan my week",
        ChromeKey::ChatStarterPlanDraft => "Help me plan my week. Here's what I need to get done: ",
        ChromeKey::ChatStarterSummarize => "Summarize a document",
        ChromeKey::ChatStarterSummarizeDraft => "Summarize this in a few bullet points:\n\n",
        ChromeKey::ChatHintIdle => "Type / for skills · Enter to send · Shift+Enter for a new line",
        ChromeKey::ChatHintGenerating => "Type ahead — Enter queues it for when Sovra is done",
        ChromeKey::ChatHintFiles => {
            "Up to 8 files · 20 MB each · Sent to your chosen AI when you send"
        }
        ChromeKey::ChatHintFilesGenerating => "Send these files after this reply finishes",
        ChromeKey::LoginEmailPlaceholder => "you@example.com",
        ChromeKey::LoginPasswordPlaceholder => "Enter your password",
        ChromeKey::NavGroups => "Groups",
        ChromeKey::NavGroupsTooltip => "Share with your family or team",
        ChromeKey::LoginName => "Your name",
        ChromeKey::LoginNamePlaceholder => "What should we call you?",
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AgentSettings, ModelPreferences};
    use super::*;

    fn has_arabic_script(text: &str) -> bool {
        text.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c))
    }

    #[test]
    fn default_language_is_english_and_ltr_while_arabic_is_rtl() {
        let default = UiLanguage::default();
        assert_eq!(default.tag(), "en");
        assert!(!default.is_rtl());
        assert!(UiLanguage::arabic().is_rtl());
        assert!(UiLanguage::from_tag("he").is_rtl());
        assert!(!UiLanguage::from_tag("fr").is_rtl());
        assert_eq!(UiLanguage::from_tag(""), UiLanguage::english());
        assert_eq!(UiLanguage::from_tag("EN").tag(), "en");
    }

    #[test]
    fn default_chrome_is_english_and_arabic_chrome_is_arabic_script() {
        assert_eq!(
            chrome(&UiLanguage::default(), ChromeKey::SettingsTitle),
            "Settings"
        );
        for key in ChromeKey::ALL {
            assert!(
                !has_arabic_script(chrome(&UiLanguage::default(), key)),
                "default {key:?} must be readable English"
            );
        }
        let arabic = UiLanguage::arabic();
        let settings = chrome(&arabic, ChromeKey::SettingsTitle);
        assert_eq!(settings, "الإعدادات");
        let chat = chrome(&arabic, ChromeKey::NavChat);
        assert!(has_arabic_script(chat), "Arabic Chat label: {chat}");
        let login = chrome(&arabic, ChromeKey::LoginWelcomeTitle);
        assert!(has_arabic_script(login), "Arabic login title: {login}");
        let welcome = chrome(&arabic, ChromeKey::ChatWelcomeSubtitle);
        assert!(has_arabic_script(welcome), "Arabic chat welcome: {welcome}");
        let password = chrome(&arabic, ChromeKey::LoginPasswordPlaceholder);
        assert!(
            has_arabic_script(password),
            "Arabic login password placeholder: {password}"
        );
        assert_eq!(
            chrome(&UiLanguage::english(), ChromeKey::SettingsTitle),
            "Settings"
        );
        assert_eq!(
            chrome(&UiLanguage::english(), ChromeKey::SettingsLightA11y),
            "Light appearance"
        );
    }

    #[test]
    fn world_catalog_includes_arabic_english_and_a_wide_set() {
        let tags: Vec<_> = world_languages().iter().map(|info| info.tag).collect();
        for needed in ["ar", "en", "fr", "es", "zh", "ja", "hi", "pt", "ru", "de"] {
            assert!(
                tags.iter()
                    .any(|tag| *tag == needed || tag.starts_with(&format!("{needed}-"))),
                "catalog missing {needed}: {tags:?}"
            );
        }
        assert!(
            world_languages().len() > 20,
            "catalog is a two-item list: {}",
            world_languages().len()
        );
        assert_eq!(world_languages()[0].tag, "ar");
    }

    #[test]
    fn chrome_never_returns_empty_or_raw_keys_even_without_a_catalog() {
        for key in ChromeKey::ALL {
            for language in [
                UiLanguage::default(),
                UiLanguage::english(),
                UiLanguage::from_tag("fr"),
                UiLanguage::from_tag("ja"),
            ] {
                let text = chrome(&language, key);
                assert!(!text.is_empty(), "{key:?} empty for {}", language.tag());
                assert_ne!(text, format!("{key:?}"), "raw key on screen for {key:?}");
            }
        }
        assert_eq!(
            chrome(&UiLanguage::from_tag("zz"), ChromeKey::SettingsTitle),
            chrome(&UiLanguage::english(), ChromeKey::SettingsTitle),
            "a tag outside the catalog falls back to English"
        );
        assert_ne!(
            chrome(&UiLanguage::from_tag("fr"), ChromeKey::SettingsTitle),
            chrome(&UiLanguage::arabic(), ChromeKey::SettingsTitle),
            "French settings chrome must not reuse Arabic"
        );
    }

    #[test]
    fn turkish_tag_is_tr_turkce_ltr_and_chrome_is_ayarlar_dil_sohbet_iptal_acik_koyu() {
        let turkish = UiLanguage::from_tag("TR");
        assert_eq!(turkish.tag(), "tr");
        assert_eq!(turkish.english_name(), "Turkish");
        assert_eq!(turkish.native_name(), "Türkçe");
        assert!(!turkish.is_rtl());
        let expected = [
            (ChromeKey::SettingsTitle, "Ayarlar"),
            (ChromeKey::SettingsLanguageTitle, "Dil"),
            (ChromeKey::NavChat, "Sohbet"),
            (ChromeKey::LoginCancel, "İptal"),
            (ChromeKey::SettingsLight, "Açık"),
            (ChromeKey::SettingsDark, "Koyu"),
        ];
        for (key, label) in expected {
            let text = chrome(&turkish, key);
            assert_eq!(text, label, "{key:?}");
            assert_ne!(text, chrome(&UiLanguage::arabic(), key), "{key:?}");
            assert_ne!(text, chrome(&UiLanguage::english(), key), "{key:?}");
            assert!(
                !has_arabic_script(text),
                "{key:?} used Arabic script: {text}"
            );
        }
        let own: Vec<_> = ChromeKey::ALL
            .iter()
            .map(|key| chrome(&turkish, *key))
            .collect();
        let english: Vec<_> = ChromeKey::ALL
            .iter()
            .map(|key| chrome(&UiLanguage::english(), *key))
            .collect();
        let arabic: Vec<_> = ChromeKey::ALL
            .iter()
            .map(|key| chrome(&UiLanguage::arabic(), *key))
            .collect();
        assert_ne!(own, english);
        assert_ne!(own, arabic);
    }

    #[test]
    fn first_run_steps_read_in_english_or_arabic_and_never_blank() {
        for key in FirstRunKey::ALL {
            let english = first_run_text(&UiLanguage::default(), key);
            let arabic = first_run_text(&UiLanguage::arabic(), key);
            assert!(
                !english.is_empty() && !has_arabic_script(english),
                "{key:?}"
            );
            assert!(has_arabic_script(arabic), "{key:?} Arabic: {arabic}");
            assert_eq!(first_run_text(&UiLanguage::from_tag("fr"), key), english);
        }
        assert_eq!(
            first_run_text(&UiLanguage::default(), FirstRunKey::ConnectAction),
            "Connect an AI"
        );
    }

    #[test]
    fn missing_or_empty_language_is_english() {
        for tag in ["", "   "] {
            let language = UiLanguage::from_tag(tag);
            assert_eq!(language.tag(), "en");
            assert!(!language.is_rtl());
            assert_eq!(chrome(&language, ChromeKey::SettingsTitle), "Settings");
        }
    }

    #[test]
    fn chooser_languages_have_own_chrome_not_arabic_or_english_and_reply_names() {
        let arabic = UiLanguage::arabic();
        let english = UiLanguage::english();
        let english_set: Vec<_> = ChromeKey::ALL
            .iter()
            .map(|key| chrome(&english, *key))
            .collect();
        let arabic_script_keys: Vec<_> = ChromeKey::ALL
            .iter()
            .copied()
            .filter(|key| has_arabic_script(chrome(&arabic, *key)))
            .collect();
        assert_ne!(
            arabic_script_keys.len(),
            0,
            "Arabic chrome has no Arabic-script strings"
        );
        let sequence = ["ar", "tr", "en", "ar"];
        let mut seen = Vec::new();
        for tag in sequence {
            let language = UiLanguage::from_tag(tag);
            seen.push((
                language.tag().to_owned(),
                language.is_rtl(),
                chrome(&language, ChromeKey::SettingsTitle).to_owned(),
            ));
        }
        assert_eq!(seen[0].0, "ar");
        assert!(seen[0].1);
        assert!(has_arabic_script(&seen[0].2));
        assert_eq!(seen[1].0, "tr");
        assert!(!seen[1].1);
        assert_eq!(seen[1].2, "Ayarlar");
        assert_eq!(seen[2].0, "en");
        assert!(!seen[2].1);
        assert_eq!(seen[2].2, "Settings");
        assert_eq!(seen[3], seen[0]);

        for info in world_languages() {
            let language = UiLanguage::from_tag(info.tag);
            assert_eq!(language.tag(), info.tag);
            assert_eq!(language.english_name(), info.english_name);
            assert_eq!(language.native_name(), info.native_name);
            assert_eq!(language.is_rtl(), info.rtl);
            let reply = language.reply_instruction();
            assert!(
                reply.contains(info.english_name),
                "{} reply missing English name: {reply}",
                info.tag
            );
            assert!(
                reply.contains(info.native_name),
                "{} reply missing native name: {reply}",
                info.tag
            );
            if info.tag != "ar" {
                assert!(
                    !reply.contains("العربية"),
                    "{} reply named Arabic: {reply}",
                    info.tag
                );
            }
            let mut own = Vec::new();
            let uses_arabic_script = has_arabic_script(info.native_name);
            for key in ChromeKey::ALL {
                let text = chrome(&language, key);
                assert!(!text.is_empty(), "{key:?} empty for {}", info.tag);
                assert_ne!(
                    text,
                    format!("{key:?}"),
                    "raw key on screen for {key:?} {}",
                    info.tag
                );
                own.push(text);
                if info.tag != "ar" && arabic_script_keys.contains(&key) {
                    assert_ne!(
                        text,
                        chrome(&arabic, key),
                        "{key:?} for {} reused the Arabic catalog",
                        info.tag
                    );
                    if !uses_arabic_script {
                        assert!(
                            !has_arabic_script(text),
                            "{key:?} for {} contains Arabic script: {text}",
                            info.tag
                        );
                    }
                }
            }
            if info.tag != "en" {
                assert_ne!(
                    own, english_set,
                    "{} chrome catalog is a copy of English",
                    info.tag
                );
            }
        }
    }

    #[test]
    fn standing_instructions_embed_the_chosen_reply_language() {
        let settings = AgentSettings::default();
        let english =
            settings.compose_system_instruction("Base instruction.", &UiLanguage::default());
        assert!(english.contains("Always reply in English"));
        let arabic =
            settings.compose_system_instruction("Base instruction.", &UiLanguage::arabic());
        let french =
            settings.compose_system_instruction("Base instruction.", &UiLanguage::from_tag("fr"));
        assert!(arabic.contains("Base instruction."));
        assert!(arabic.contains("Arabic"));
        assert!(arabic.contains("العربية"));
        assert!(french.contains("French"));
        assert!(french.contains("Français"));
        assert_ne!(arabic, french);
    }

    #[test]
    fn missing_saved_language_is_english_and_a_saved_choice_is_kept() {
        let older: ModelPreferences = serde_json::from_str(r#"{"favorites":[],"active":null}"#)
            .expect("preferences saved before language existed");
        assert_eq!(older.language, UiLanguage::english());
        let chosen: ModelPreferences =
            serde_json::from_str(r#"{"favorites":[],"active":null,"language":"ar"}"#)
                .expect("a saved Arabic choice");
        assert_eq!(chosen.language, UiLanguage::arabic());
        let tagged: UiLanguage = serde_json::from_str("\"en\"").expect("tag");
        assert_eq!(tagged, UiLanguage::english());
        assert_eq!(
            serde_json::to_string(&UiLanguage::default()).expect("ser"),
            "\"en\""
        );
    }
}
