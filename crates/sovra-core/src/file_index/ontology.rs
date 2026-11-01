//! What a file *is* and what it is *about*, in plain words backed by open standards.
//!
//! Three layers, each a small SKOS-shaped concept scheme compiled into the binary:
//!
//! 1. **Format** — the IANA media type the extractor detected (stored per version).
//! 2. **Kind** — [`FileKind`]: what sort of thing the file is for a person ("Photos &
//!    pictures"). Every concept carries `exactMatch`/`closeMatch` links to the DCMI Type
//!    Vocabulary, schema.org and the NEPOMUK file ontologies (NFO/NMO) so the index can
//!    be exported or reasoned over with standard tools.
//! 3. **Life area** — [`LifeArea`]: what the file is about in everyday terms ("Money",
//!    "Health"), with `closeMatch` links to IPTC Media Topics where one fits.
//!
//! Only identifiers are referenced, never copied descriptions: schema.org is CC BY-SA,
//! IPTC Media Topics CC BY 4.0, DCMI and NEPOMUK publish their IRIs openly. GPL data
//! (freedesktop shared-mime-info) is deliberately not embedded.
//!
//! Sources and the design rationale: `docs/file-ontology.md`.

use serde::{Deserialize, Serialize};

/// What sort of thing a file is, at the granularity a non-technical person uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Document,
    Spreadsheet,
    Presentation,
    Photo,
    Video,
    Audio,
    Email,
    Bill,
    Form,
    Book,
    Code,
    Data,
    Archive,
    App,
    Other,
}

/// One SKOS concept in the Kind scheme. IRIs are full so they can be emitted verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindConcept {
    pub kind: FileKind,
    /// `skos:prefLabel` — the only name a person ever sees.
    pub pref_label: &'static str,
    /// Lower-case count noun for sentences: "320 photos".
    pub plural: &'static str,
    pub singular: &'static str,
    /// `skos:altLabel` — words people search with.
    pub alt_labels: &'static [&'static str],
    /// `skos:broader` in the DCMI Type Vocabulary, when one applies.
    pub dcmi_type: Option<&'static str>,
    /// `skos:closeMatch` in schema.org.
    pub schema_org: &'static str,
    /// `skos:closeMatch` in NEPOMUK (NFO / NMO).
    pub nepomuk: &'static str,
}

const DCMI: &str = "http://purl.org/dc/dcmitype/";
const NFO: &str = "http://www.semanticdesktop.org/ontologies/2007/03/22/nfo#";

/// The Kind concept scheme, in the order the UI lists kinds when counts tie.
pub const KINDS: &[KindConcept] = &[
    concept(
        FileKind::Photo,
        "Photos & pictures",
        ("photo", "photos"),
        &["picture", "image", "scan", "screenshot"],
        Some("StillImage"),
        "Photograph",
        "nfo:Image",
    ),
    concept(
        FileKind::Document,
        "Documents & letters",
        ("document", "documents"),
        &["letter", "note", "pdf", "word", "text"],
        Some("Text"),
        "DigitalDocument",
        "nfo:PaginatedTextDocument",
    ),
    concept(
        FileKind::Bill,
        "Bills, receipts & statements",
        ("bill or receipt", "bills and receipts"),
        &["invoice", "receipt", "statement", "payslip", "tax"],
        Some("Text"),
        "Invoice",
        "nfo:Document",
    ),
    concept(
        FileKind::Form,
        "Forms & certificates",
        ("form or certificate", "forms and certificates"),
        &[
            "form",
            "certificate",
            "application",
            "licence",
            "passport",
            "contract",
        ],
        Some("Text"),
        "DigitalDocument",
        "nfo:Document",
    ),
    concept(
        FileKind::Spreadsheet,
        "Spreadsheets & tables",
        ("spreadsheet", "spreadsheets"),
        &["excel", "table", "csv", "numbers", "budget"],
        Some("Dataset"),
        "SpreadsheetDigitalDocument",
        "nfo:Spreadsheet",
    ),
    concept(
        FileKind::Presentation,
        "Slide shows",
        ("slide show", "slide shows"),
        &["presentation", "slides", "powerpoint", "keynote"],
        Some("Text"),
        "PresentationDigitalDocument",
        "nfo:Presentation",
    ),
    concept(
        FileKind::Email,
        "Emails & messages",
        ("email", "emails"),
        &["mail", "message", "inbox"],
        Some("Text"),
        "EmailMessage",
        "nmo:Email",
    ),
    concept(
        FileKind::Audio,
        "Music & recordings",
        ("recording", "recordings"),
        &["music", "song", "voice memo", "podcast", "audio"],
        Some("Sound"),
        "AudioObject",
        "nfo:Audio",
    ),
    concept(
        FileKind::Video,
        "Videos",
        ("video", "videos"),
        &["movie", "film", "clip", "recording"],
        Some("MovingImage"),
        "VideoObject",
        "nfo:Video",
    ),
    concept(
        FileKind::Book,
        "Books & reading",
        ("book", "books"),
        &["ebook", "epub", "novel", "manual"],
        Some("Text"),
        "Book",
        "nfo:PaginatedTextDocument",
    ),
    concept(
        FileKind::Code,
        "Computer code",
        ("code file", "code files"),
        &["program", "script", "source"],
        Some("Software"),
        "SoftwareSourceCode",
        "nfo:SourceCode",
    ),
    concept(
        FileKind::Data,
        "Data files",
        ("data file", "data files"),
        &["database", "settings", "json", "xml"],
        Some("Dataset"),
        "Dataset",
        "nfo:FileDataObject",
    ),
    concept(
        FileKind::Archive,
        "Zipped bundles",
        ("zipped bundle", "zipped bundles"),
        &["zip", "archive", "compressed"],
        Some("Collection"),
        "MediaObject",
        "nfo:Archive",
    ),
    concept(
        FileKind::App,
        "Programs & installers",
        ("program", "programs"),
        &["app", "application", "installer", "software"],
        Some("Software"),
        "SoftwareApplication",
        "nfo:Executable",
    ),
    concept(
        FileKind::Other,
        "Other files",
        ("other file", "other files"),
        &[],
        None,
        "MediaObject",
        "nfo:FileDataObject",
    ),
];

const fn concept(
    kind: FileKind,
    pref_label: &'static str,
    (singular, plural): (&'static str, &'static str),
    alt_labels: &'static [&'static str],
    dcmi_type: Option<&'static str>,
    schema_org: &'static str,
    nepomuk: &'static str,
) -> KindConcept {
    KindConcept {
        kind,
        pref_label,
        plural,
        singular,
        alt_labels,
        dcmi_type,
        schema_org,
        nepomuk,
    }
}

impl FileKind {
    pub fn concept(self) -> &'static KindConcept {
        KINDS
            .iter()
            .find(|c| c.kind == self)
            .expect("every FileKind has a row in KINDS")
    }

    pub fn label(self) -> &'static str {
        self.concept().pref_label
    }

    /// "1 photo" / "320 photos".
    pub fn count_phrase(self, count: u64) -> String {
        let concept = self.concept();
        let noun = if count == 1 {
            concept.singular
        } else {
            concept.plural
        };
        format!("{} {noun}", group_digits(count))
    }

    /// Full DCMI Type IRI (`skos:broader`).
    pub fn dcmi_iri(self) -> Option<String> {
        self.concept().dcmi_type.map(|t| format!("{DCMI}{t}"))
    }

    /// Full schema.org IRI (`skos:closeMatch`).
    pub fn schema_org_iri(self) -> String {
        format!("https://schema.org/{}", self.concept().schema_org)
    }

    /// Full NEPOMUK IRI (`skos:closeMatch`). NMO terms live beside NFO.
    pub fn nepomuk_iri(self) -> String {
        let term = self.concept().nepomuk;
        match term.split_once(':') {
            Some(("nmo", local)) => {
                format!("http://www.semanticdesktop.org/ontologies/2007/03/22/nmo#{local}")
            }
            Some((_, local)) => format!("{NFO}{local}"),
            None => format!("{NFO}{term}"),
        }
    }
}

/// Classify a file from its detected media type and its path. The media type wins;
/// the extension decides only when the type is generic (`application/octet-stream`,
/// `text/plain`) or unknown. Words in the file name then refine documents into bills
/// or forms — a cheap, explainable signal that needs no model.
pub fn classify(media_type: Option<&str>, path: &str) -> FileKind {
    let name = file_name(path);
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    let by_type = media_type
        .map(|m| m.split(';').next().unwrap_or(m).trim().to_ascii_lowercase())
        .and_then(|m| kind_for_media_type(&m));
    let kind = match (by_type, extension.as_deref().and_then(kind_for_extension)) {
        // Plain text is a container for code and data as often as for prose.
        (Some(FileKind::Document), Some(ext @ (FileKind::Code | FileKind::Data))) => ext,
        (Some(kind), _) | (None, Some(kind)) => kind,
        (None, None) => FileKind::Other,
    };
    refine_by_name(kind, &name.to_lowercase())
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn kind_for_media_type(media: &str) -> Option<FileKind> {
    let (top, sub) = media.split_once('/')?;
    Some(match (top, sub) {
        ("image", _) => FileKind::Photo,
        ("video", _) => FileKind::Video,
        ("audio", _) => FileKind::Audio,
        ("message", "rfc822") | ("application", "mbox" | "vnd.ms-outlook") => FileKind::Email,
        ("text", "csv" | "tab-separated-values")
        | (
            "application",
            "vnd.ms-excel"
            | "vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "vnd.oasis.opendocument.spreadsheet"
            | "vnd.apple.numbers",
        ) => FileKind::Spreadsheet,
        (
            "application",
            "vnd.ms-powerpoint"
            | "vnd.openxmlformats-officedocument.presentationml.presentation"
            | "vnd.oasis.opendocument.presentation"
            | "vnd.apple.keynote",
        ) => FileKind::Presentation,
        ("application", "epub+zip" | "x-mobipocket-ebook" | "vnd.amazon.ebook") => FileKind::Book,
        (
            "application",
            "zip" | "gzip" | "x-tar" | "x-7z-compressed" | "vnd.rar" | "x-rar-compressed"
            | "x-bzip2" | "x-xz" | "zstd",
        ) => FileKind::Archive,
        (
            "application",
            "x-mach-binary"
            | "x-executable"
            | "x-msdownload"
            | "vnd.microsoft.portable-executable"
            | "vnd.android.package-archive"
            | "x-apple-diskimage",
        ) => FileKind::App,
        (
            "application",
            "javascript" | "x-sh" | "x-python" | "x-rust" | "typescript" | "x-httpd-php",
        ) => FileKind::Code,
        ("application", sub)
            if matches!(
                sub,
                "json" | "xml" | "yaml" | "toml" | "x-sqlite3" | "vnd.sqlite3"
            ) || sub.ends_with("+json")
                || sub.ends_with("+xml") =>
        {
            FileKind::Data
        }
        ("text", "x-script" | "x-python" | "x-rust" | "x-c" | "x-shellscript" | "javascript") => {
            FileKind::Code
        }
        (
            "application",
            "pdf"
            | "msword"
            | "rtf"
            | "vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "vnd.oasis.opendocument.text"
            | "vnd.apple.pages",
        )
        | ("text", _) => FileKind::Document,
        _ => return None,
    })
}

fn kind_for_extension(ext: &str) -> Option<FileKind> {
    Some(match ext {
        "jpg" | "jpeg" | "png" | "gif" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp"
        | "raw" | "dng" | "cr2" | "nef" | "arw" | "svg" => FileKind::Photo,
        "mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm" | "wmv" => FileKind::Video,
        "mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" | "opus" | "aiff" | "aif" => FileKind::Audio,
        "eml" | "msg" | "mbox" | "emlx" => FileKind::Email,
        "xls" | "xlsx" | "xlsm" | "ods" | "numbers" | "csv" | "tsv" => FileKind::Spreadsheet,
        "ppt" | "pptx" | "odp" | "key" => FileKind::Presentation,
        "epub" | "mobi" | "azw" | "azw3" => FileKind::Book,
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "pages" | "txt" | "md" | "markdown" | "html"
        | "htm" => FileKind::Document,
        "zip" | "gz" | "tgz" | "tar" | "7z" | "rar" | "bz2" | "xz" | "zst" => FileKind::Archive,
        "app" | "exe" | "msi" | "dmg" | "pkg" | "apk" | "deb" | "rpm" | "appimage" => FileKind::App,
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "kt" | "swift" | "c" | "h"
        | "cpp" | "hpp" | "cs" | "rb" | "php" | "sh" | "zsh" | "ps1" | "sql" | "css" | "scss" => {
            FileKind::Code
        }
        "json" | "xml" | "yaml" | "yml" | "toml" | "ini" | "plist" | "sqlite" | "db"
        | "parquet" | "geojson" | "gpx" | "ics" | "vcf" => FileKind::Data,
        _ => return None,
    })
}

// Cue words cover English plus the most common French, German, Spanish, Italian, Dutch
// and Portuguese terms, with and without accents (people often type names without them).
const BILL_WORDS: &[&str] = &[
    "facture",
    "rechnung",
    "factura",
    "fattura",
    "factuur",
    "fatura",
    "reçu",
    "recu",
    "quittung",
    "recibo",
    "ricevuta",
    "kontoauszug",
    "relevé",
    "releve",
    "extracto",
    "estratto",
    "afschrift",
    "nómina",
    "nomina",
    "lohnabrechnung",
    "loonstrook",
    "fiche de paie",
    "busta paga",
    "holerite",
    "invoice",
    "receipt",
    "bill",
    "statement",
    "payslip",
    "pay slip",
    "tax return",
    "p60",
    "w2",
];
const FORM_WORDS: &[&str] = &[
    "formulaire",
    "formular",
    "formulario",
    "formulier",
    "certificat",
    "zertifikat",
    "bescheinigung",
    "certificado",
    "certificato",
    "certificaat",
    "attestation",
    "vertrag",
    "contrat",
    "contrato",
    "contratto",
    "passeport",
    "reisepass",
    "pasaporte",
    "passaporto",
    "paspoort",
    "passaporte",
    "antrag",
    "solicitud",
    "domanda",
    "aanvraag",
    "form",
    "certificate",
    "application",
    "passport",
    "licence",
    "license",
    "contract",
    "agreement",
    "policy",
];

fn refine_by_name(kind: FileKind, lower_name: &str) -> FileKind {
    if !matches!(
        kind,
        FileKind::Document | FileKind::Spreadsheet | FileKind::Photo
    ) {
        return kind;
    }
    let words = words(lower_name);
    let has = |cues: &[&str]| cues.iter().any(|cue| contains_phrase(&words, cue));
    if has(BILL_WORDS) {
        FileKind::Bill
    } else if kind == FileKind::Document && has(FORM_WORDS) {
        FileKind::Form
    } else {
        kind
    }
}

/// What a file is about, in the handful of areas people sort their life into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifeArea {
    Money,
    Health,
    Home,
    Work,
    Family,
    Travel,
    Learning,
    Hobbies,
    Legal,
    Shopping,
    Vehicles,
}

/// One SKOS concept in the Life-area scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AreaConcept {
    pub area: LifeArea,
    pub pref_label: &'static str,
    /// Words in folder or file names that suggest this area (lower case, whole words).
    pub cues: &'static [&'static str],
    /// `skos:closeMatch` IPTC Media Topic code, only where the match is genuine.
    pub iptc_media_topic: Option<&'static str>,
}

pub const AREAS: &[AreaConcept] = &[
    AreaConcept {
        area: LifeArea::Money,
        pref_label: "Money",
        cues: &[
            "banque",
            "steuer",
            "steuern",
            "impôts",
            "impots",
            "impuestos",
            "finanzen",
            "banco",
            "banca",
            "belasting",
            "financiën",
            "bank",
            "banking",
            "finance",
            "finances",
            "tax",
            "taxes",
            "invoice",
            "invoices",
            "receipt",
            "receipts",
            "budget",
            "pension",
            "mortgage",
            "savings",
            "payslip",
            "statement",
            "statements",
            "bills",
            "accounts",
        ],
        iptc_media_topic: Some("medtop:04000000"),
    },
    AreaConcept {
        area: LifeArea::Health,
        pref_label: "Health",
        cues: &[
            "arzt",
            "médecin",
            "medecin",
            "médico",
            "medico",
            "santé",
            "sante",
            "salud",
            "gesundheit",
            "salute",
            "krankenhaus",
            "hôpital",
            "hopital",
            "ospedale",
            "ziekenhuis",
            "saúde",
            "saude",
            "health",
            "medical",
            "doctor",
            "hospital",
            "prescription",
            "dentist",
            "nhs",
            "vaccination",
            "insurance claim",
            "fitness",
        ],
        iptc_media_topic: Some("medtop:07000000"),
    },
    AreaConcept {
        area: LifeArea::Home,
        pref_label: "Home",
        cues: &[
            "haus",
            "maison",
            "wohnung",
            "miete",
            "loyer",
            "alquiler",
            "affitto",
            "huur",
            "hogar",
            "home",
            "house",
            "rent",
            "lease",
            "utilities",
            "electricity",
            "gas",
            "water",
            "council",
            "garden",
            "renovation",
            "appliance",
            "warranty",
        ],
        iptc_media_topic: None,
    },
    AreaConcept {
        area: LifeArea::Work,
        pref_label: "Work",
        cues: &[
            "arbeit",
            "travail",
            "trabajo",
            "lavoro",
            "werk",
            "trabalho",
            "bewerbung",
            "lebenslauf",
            "work",
            "job",
            "cv",
            "resume",
            "office",
            "meeting",
            "meetings",
            "project",
            "projects",
            "client",
            "clients",
            "report",
            "proposal",
        ],
        iptc_media_topic: None,
    },
    AreaConcept {
        area: LifeArea::Family,
        pref_label: "Family & friends",
        cues: &[
            "familie",
            "famille",
            "familia",
            "famiglia",
            "família",
            "hochzeit",
            "mariage",
            "boda",
            "matrimonio",
            "geburtstag",
            "anniversaire",
            "cumpleaños",
            "compleanno",
            "verjaardag",
            "family",
            "kids",
            "children",
            "wedding",
            "birthday",
            "christmas",
            "friends",
            "mum",
            "mom",
            "dad",
            "grandma",
            "grandpa",
            "baby",
        ],
        iptc_media_topic: None,
    },
    AreaConcept {
        area: LifeArea::Travel,
        pref_label: "Travel & trips",
        cues: &[
            "reise",
            "reisen",
            "urlaub",
            "voyage",
            "vacances",
            "viaje",
            "vacaciones",
            "viaggio",
            "vacanze",
            "vakantie",
            "viagem",
            "férias",
            "ferias",
            "flug",
            "vuelo",
            "volo",
            "travel",
            "trip",
            "trips",
            "holiday",
            "holidays",
            "vacation",
            "flight",
            "flights",
            "hotel",
            "boarding pass",
            "itinerary",
            "visa",
        ],
        iptc_media_topic: None,
    },
    AreaConcept {
        area: LifeArea::Learning,
        pref_label: "Learning & school",
        cues: &[
            "schule",
            "école",
            "ecole",
            "escuela",
            "scuola",
            "universität",
            "universitat",
            "université",
            "universite",
            "universidad",
            "università",
            "universita",
            "universiteit",
            "cours",
            "kurs",
            "curso",
            "corso",
            "school",
            "university",
            "college",
            "course",
            "courses",
            "homework",
            "lecture",
            "lectures",
            "study",
            "notes",
            "thesis",
            "exam",
        ],
        iptc_media_topic: Some("medtop:05000000"),
    },
    AreaConcept {
        area: LifeArea::Hobbies,
        pref_label: "Hobbies & fun",
        cues: &[
            "rezepte",
            "recettes",
            "recetas",
            "ricette",
            "recepten",
            "receitas",
            "musik",
            "musique",
            "música",
            "musica",
            "hobby",
            "hobbies",
            "music",
            "recipes",
            "recipe",
            "games",
            "photography",
            "art",
            "knitting",
            "sport",
            "football",
        ],
        iptc_media_topic: Some("medtop:01000000"),
    },
    AreaConcept {
        area: LifeArea::Legal,
        pref_label: "Legal & official papers",
        cues: &[
            "testament",
            "notar",
            "notaire",
            "notario",
            "notaio",
            "notaris",
            "anwalt",
            "avocat",
            "abogado",
            "avvocato",
            "advocaat",
            "advogado",
            "gericht",
            "tribunal",
            "legal",
            "will",
            "contract",
            "contracts",
            "passport",
            "id",
            "certificate",
            "certificates",
            "court",
            "solicitor",
            "lawyer",
            "deed",
        ],
        iptc_media_topic: Some("medtop:02000000"),
    },
    AreaConcept {
        area: LifeArea::Shopping,
        pref_label: "Shopping & orders",
        cues: &[
            "bestellung",
            "commande",
            "pedido",
            "ordine",
            "bestelling",
            "encomenda",
            "einkauf",
            "achat",
            "compra",
            "acquisto",
            "aankoop",
            "order",
            "orders",
            "amazon",
            "shopping",
            "purchase",
            "purchases",
            "delivery",
            "returns",
        ],
        iptc_media_topic: None,
    },
    AreaConcept {
        area: LifeArea::Vehicles,
        pref_label: "Car & transport",
        cues: &[
            "auto",
            "voiture",
            "coche",
            "carro",
            "fahrzeug",
            "véhicule",
            "vehicule",
            "vehículo",
            "vehiculo",
            "veicolo",
            "voertuig",
            "car",
            "vehicle",
            "mot",
            "dvla",
            "garage",
            "parking",
            "bike",
            "motorbike",
        ],
        iptc_media_topic: None,
    },
];

impl LifeArea {
    pub fn concept(self) -> &'static AreaConcept {
        AREAS
            .iter()
            .find(|c| c.area == self)
            .expect("every LifeArea has a row in AREAS")
    }

    pub fn label(self) -> &'static str {
        self.concept().pref_label
    }

    /// Full IPTC Media Topic IRI (`skos:closeMatch`), where one fits.
    pub fn iptc_iri(self) -> Option<String> {
        self.concept()
            .iptc_media_topic
            .and_then(|code| code.strip_prefix("medtop:"))
            .map(|code| format!("http://cv.iptc.org/newscodes/mediatopic/{code}"))
    }
}

/// Guess a life area from folder and file names alone (English plus common French, German,
/// Spanish, Italian, Dutch and Portuguese words). Folder names are how people
/// already sort their lives ("Taxes 2024", "Holiday photos"), so they are a strong,
/// explainable first signal; content-based topics come later from clustering.
/// Ties go to the area listed first in [`AREAS`]; no cue means no guess.
pub fn life_area(path: &str) -> Option<LifeArea> {
    let lower = path.to_lowercase();
    let words = words(&lower);
    AREAS
        .iter()
        .map(|c| {
            let hits = c
                .cues
                .iter()
                .filter(|cue| contains_phrase(&words, cue))
                .count();
            (hits, c.area)
        })
        .filter(|(hits, _)| *hits > 0)
        // max_by_key keeps the last maximum; reverse so the first-listed area wins ties.
        .rev()
        .max_by_key(|(hits, _)| *hits)
        .map(|(_, area)| area)
}

/// Lower-case alphanumeric words; separators (including `-`/`_`) and digit-only runs
/// are dropped, so "passport-renewal-form" yields "passport", "renewal", "form".
fn words(lower: &str) -> Vec<&str> {
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

/// Whole-word (or whole multi-word phrase) match, so "bill" never matches "billboard".
fn contains_phrase(words: &[&str], phrase: &str) -> bool {
    let wanted: Vec<&str> = phrase.split(' ').collect();
    words.windows(wanted.len()).any(|w| w == wanted.as_slice())
}

/// "1,234,567" — counts read better with separators at every size.
pub fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_and_area_has_exactly_one_concept_row() {
        let kinds = [
            FileKind::Document,
            FileKind::Spreadsheet,
            FileKind::Presentation,
            FileKind::Photo,
            FileKind::Video,
            FileKind::Audio,
            FileKind::Email,
            FileKind::Bill,
            FileKind::Form,
            FileKind::Book,
            FileKind::Code,
            FileKind::Data,
            FileKind::Archive,
            FileKind::App,
            FileKind::Other,
        ];
        assert_eq!(KINDS.len(), kinds.len());
        for kind in kinds {
            assert_eq!(KINDS.iter().filter(|c| c.kind == kind).count(), 1);
            assert_ne!(kind.label(), "");
        }
        let mut areas: Vec<_> = AREAS.iter().map(|c| c.area).collect();
        areas.sort();
        areas.dedup();
        assert_eq!(areas.len(), AREAS.len());
    }

    #[test]
    fn labels_are_plain_words_without_jargon() {
        for concept in KINDS {
            for jargon in ["MIME", "mime", "file type", "binary", "metadata", "/"] {
                assert!(
                    !concept.pref_label.contains(jargon),
                    "{} uses jargon",
                    concept.pref_label
                );
            }
        }
    }

    #[test]
    fn standard_identifiers_are_well_formed_iris() {
        assert_eq!(
            FileKind::Photo.dcmi_iri().as_deref(),
            Some("http://purl.org/dc/dcmitype/StillImage")
        );
        assert_eq!(
            FileKind::Spreadsheet.schema_org_iri(),
            "https://schema.org/SpreadsheetDigitalDocument"
        );
        assert_eq!(
            FileKind::Email.nepomuk_iri(),
            "http://www.semanticdesktop.org/ontologies/2007/03/22/nmo#Email"
        );
        assert_eq!(
            FileKind::Archive.nepomuk_iri(),
            "http://www.semanticdesktop.org/ontologies/2007/03/22/nfo#Archive"
        );
        assert_eq!(
            LifeArea::Health.iptc_iri().as_deref(),
            Some("http://cv.iptc.org/newscodes/mediatopic/07000000")
        );
        assert_eq!(LifeArea::Home.iptc_iri(), None);
    }

    #[test]
    fn media_type_decides_and_extension_breaks_generic_types() {
        assert_eq!(
            classify(Some("image/heic"), "IMG_0001.HEIC"),
            FileKind::Photo
        );
        assert_eq!(
            classify(Some("application/pdf"), "a/b/report.pdf"),
            FileKind::Document
        );
        assert_eq!(
            classify(Some("text/plain; charset=utf-8"), "src/main.rs"),
            FileKind::Code
        );
        assert_eq!(classify(Some("text/plain"), "config.yaml"), FileKind::Data);
        assert_eq!(
            classify(Some("application/octet-stream"), "song.flac"),
            FileKind::Audio
        );
        assert_eq!(classify(None, "Holiday.MOV"), FileKind::Video);
        assert_eq!(classify(None, "mystery.blob"), FileKind::Other);
        assert_eq!(
            classify(
                Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
                "x.xlsx"
            ),
            FileKind::Spreadsheet
        );
        assert_eq!(
            classify(Some("message/rfc822"), "mail.eml"),
            FileKind::Email
        );
    }

    #[test]
    fn names_refine_documents_into_bills_and_forms_by_whole_word() {
        assert_eq!(
            classify(Some("application/pdf"), "Money/Invoice 2024-03.pdf"),
            FileKind::Bill
        );
        assert_eq!(
            classify(Some("image/jpeg"), "receipt_tesco.jpg"),
            FileKind::Bill
        );
        assert_eq!(
            classify(Some("application/pdf"), "passport-renewal-form.pdf"),
            FileKind::Form
        );
        assert_eq!(
            classify(Some("application/pdf"), "billboard-design.pdf"),
            FileKind::Document,
            "partial words never match"
        );
        assert_eq!(
            classify(Some("video/mp4"), "receipt.mp4"),
            FileKind::Video,
            "only documents, sheets and pictures are refined"
        );
    }

    #[test]
    fn life_area_reads_folder_names_and_prefers_more_cues() {
        assert_eq!(
            life_area("Documents/Taxes 2024/bank statement.pdf"),
            Some(LifeArea::Money)
        );
        assert_eq!(
            life_area("Photos/Holiday Spain/IMG_1.jpg"),
            Some(LifeArea::Travel)
        );
        assert_eq!(
            life_area("Medical/doctor letter.pdf"),
            Some(LifeArea::Health)
        );
        assert_eq!(life_area("random/IMG_0042.jpg"), None);
        assert_eq!(
            life_area("Bank/holiday.pdf"),
            Some(LifeArea::Money),
            "ties go to the first listed area"
        );
        assert_eq!(
            life_area("scarface/scar.txt"),
            None,
            "whole words only: 'car' is not in 'scarface'"
        );
    }

    #[test]
    fn common_european_words_are_understood_with_or_without_accents() {
        for name in [
            "Rechnung 2024.pdf",
            "facture-edf.pdf",
            "Factura luz.pdf",
            "fattura_enel.pdf",
            "factuur.pdf",
            "Releve bancaire.pdf",
            "Relevé bancaire.pdf",
        ] {
            assert_eq!(
                classify(Some("application/pdf"), name),
                FileKind::Bill,
                "{name}"
            );
        }
        for name in ["Formulaire CAF.pdf", "Vertrag Miete.pdf", "certificado.pdf"] {
            assert_eq!(
                classify(Some("application/pdf"), name),
                FileKind::Form,
                "{name}"
            );
        }
        assert_eq!(
            classify(Some("application/pdf"), "Mietvertrag.pdf"),
            FileKind::Document,
            "compound words are not split: whole words only"
        );
        assert_eq!(
            life_area("Urlaub Spanien/IMG_1.jpg"),
            Some(LifeArea::Travel)
        );
        assert_eq!(life_area("Santé/ordonnance.pdf"), Some(LifeArea::Health));
        assert_eq!(life_area("Sante/ordonnance.pdf"), Some(LifeArea::Health));
        assert_eq!(
            life_area("Steuern 2023/Bescheid.pdf"),
            Some(LifeArea::Money)
        );
        assert_eq!(life_area("Vacaciones/playa.jpg"), Some(LifeArea::Travel));
        assert_eq!(life_area("Famiglia/foto.jpg"), Some(LifeArea::Family));
        assert_eq!(
            life_area("Manual vol 2.pdf"),
            None,
            "ambiguous words are left out"
        );
    }

    #[test]
    fn counts_read_naturally() {
        assert_eq!(FileKind::Photo.count_phrase(1), "1 photo");
        assert_eq!(FileKind::Photo.count_phrase(1_234), "1,234 photos");
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1_000_000), "1,000,000");
    }
}
