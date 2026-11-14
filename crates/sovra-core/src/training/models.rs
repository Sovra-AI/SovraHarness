//! The built-in starting points, the exact shape of their numbers, and the model file.
//!
//! Every model reads and writes plain bytes (a fixed vocabulary of 256), so there is
//! no tokenizer file and any language works. A model is one flat list of `f32`
//! values; [`Architecture::blocks`] is the single definition of how that list is
//! cut into tensors, and the engine builds its tensors from it.

use serde::{Deserialize, Serialize};

use crate::group::ContentHash;

use super::TrainingError;

/// Number of distinct input and output symbols: every byte value.
pub const VOCABULARY: usize = 256;
/// Largest model file this computer will store, send or accept.
pub const MAX_MODEL_BYTES: usize = 16 * 1024 * 1024;
/// Fixed bytes before the values in a model file.
pub const MODEL_HEADER_BYTES: usize = 64;
/// Largest model whose work packet and returned update each fit one group message
/// (8 MiB of JSON, where bytes travel as hex and so cost double) with room for the
/// assignment and the text.
pub const MAX_GROUP_PARAMETERS: usize = 1_000_000;

/// A library model is named by the hash of its file, so every copy is provably
/// the same model.
pub type ModelId = ContentHash;

const MODEL_MAGIC: &[u8; 8] = b"SOVRAMDL";
const MODEL_VERSION: u32 = 1;
const NO_LOSS: u32 = u32::MAX;

/// The built-in starting points a person can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseModel {
    Starter,
    Small,
    Medium,
    Large,
}

impl BaseModel {
    pub const ALL: [Self; 4] = [Self::Starter, Self::Small, Self::Medium, Self::Large];

    /// This model's row in [`BASE_MODELS`].
    pub fn info(self) -> &'static BaseModelInfo {
        match self {
            Self::Starter => &BASE_MODELS[0],
            Self::Small => &BASE_MODELS[1],
            Self::Medium => &BASE_MODELS[2],
            Self::Large => &BASE_MODELS[3],
        }
    }

    pub fn architecture(self) -> Architecture {
        self.info().architecture
    }

    fn code(self) -> u8 {
        match self {
            Self::Starter => 0,
            Self::Small => 1,
            Self::Medium => 2,
            Self::Large => 3,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|model| model.code() == code)
    }
}

/// One row of the starting-point list, in words a beginner can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseModelInfo {
    pub model: BaseModel,
    pub title: &'static str,
    pub summary: &'static str,
    pub architecture: Architecture,
    /// Whether the group's computers can help train it. A model too large for one
    /// group message trains on this computer only; the finished model is still
    /// delivered to the group.
    pub group_capable: bool,
    /// Memory a computer should have for this model to train comfortably.
    pub min_memory_bytes: u64,
    /// Processor threads a computer should have for it to train comfortably.
    pub comfortable_cores: u16,
}

const MIB: u64 = 1024 * 1024;

pub const BASE_MODELS: &[BaseModelInfo] = &[
    BaseModelInfo {
        model: BaseModel::Starter,
        title: "Starter",
        summary: "Learns in seconds. Picks up which letters follow which.",
        architecture: Architecture::STARTER,
        group_capable: true,
        min_memory_bytes: 256 * MIB,
        comfortable_cores: 1,
    },
    BaseModelInfo {
        model: BaseModel::Small,
        title: "Small",
        summary: "Learns words and short phrases. A good first real try on any computer.",
        architecture: Architecture {
            context: 64,
            width: 64,
            layers: 2,
            heads: 4,
        },
        group_capable: true,
        min_memory_bytes: 1024 * MIB,
        comfortable_cores: 2,
    },
    BaseModelInfo {
        model: BaseModel::Medium,
        title: "Medium",
        summary: "Learns sentences and style. Takes longer, and a group makes it quicker.",
        architecture: Architecture {
            context: 128,
            width: 128,
            layers: 4,
            heads: 4,
        },
        group_capable: true,
        min_memory_bytes: 2 * 1024 * MIB,
        comfortable_cores: 4,
    },
    BaseModelInfo {
        model: BaseModel::Large,
        title: "Large",
        summary: "Follows longer passages. Slow without a graphics card, and trains on this computer only.",
        architecture: Architecture {
            context: 256,
            width: 192,
            layers: 6,
            heads: 6,
        },
        group_capable: false,
        min_memory_bytes: 4 * 1024 * MIB,
        comfortable_cores: 8,
    },
];

/// The shape of a model. `layers == 0` is the Starter table; anything else is a
/// byte-level transformer.
///
/// # Parameter layout
///
/// A model is one flat `f32` vector. [`Self::blocks`] lists its pieces in order;
/// each piece is a row-major matrix `[rows, cols]`, and one-row pieces are vectors.
///
/// **Starter table** (`layers == 0`): a single `[256, 256]` block of scores, row =
/// the byte just read, column = the byte that follows. 65,536 values, all zero at
/// the start.
///
/// **Transformer** with context `C`, width `W`, `L` layers and `H` heads
/// (pre-norm, causal self-attention, GELU feed-forward four times the width,
/// learned positions, a separate output layer with a bias). Every linear weight is
/// `[inputs, outputs]`:
///
/// | Order | Block | Shape |
/// |---|---|---|
/// | 1 | byte embedding | `[256, W]` |
/// | 2 | position embedding | `[C, W]` |
/// | per layer | attention norm gain, bias | `[1, W]`, `[1, W]` |
/// | | query/key/value weight, bias (query first, then key, then value) | `[W, 3W]`, `[1, 3W]` |
/// | | attention output weight, bias | `[W, W]`, `[1, W]` |
/// | | feed-forward norm gain, bias | `[1, W]`, `[1, W]` |
/// | | feed-forward in weight, bias | `[W, 4W]`, `[1, 4W]` |
/// | | feed-forward out weight, bias | `[4W, W]`, `[1, W]` |
/// | then | final norm gain, bias | `[1, W]`, `[1, W]` |
/// | last | output weight, bias | `[W, 256]`, `[1, 256]` |
///
/// Total: `256·W + C·W + L·(12·W² + 13·W) + 2·W + 256·W + 256`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Architecture {
    /// How many bytes back the model can look.
    pub context: u16,
    pub width: u16,
    pub layers: u8,
    pub heads: u8,
}

/// What one piece of the flat parameter vector is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// The whole Starter model.
    Table,
    ByteEmbedding,
    PositionEmbedding,
    AttentionNormGain,
    AttentionNormBias,
    QueryKeyValueWeight,
    QueryKeyValueBias,
    AttentionOutWeight,
    AttentionOutBias,
    FeedForwardNormGain,
    FeedForwardNormBias,
    FeedForwardInWeight,
    FeedForwardInBias,
    FeedForwardOutWeight,
    FeedForwardOutBias,
    FinalNormGain,
    FinalNormBias,
    OutputWeight,
    OutputBias,
}

/// One row-major `[rows, cols]` matrix inside the flat parameter vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParameterBlock {
    pub kind: BlockKind,
    /// Which layer it belongs to, counted from zero; `None` outside the layers.
    pub layer: Option<u8>,
    pub offset: usize,
    pub rows: usize,
    pub cols: usize,
}

impl ParameterBlock {
    pub const fn len(&self) -> usize {
        self.rows * self.cols
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn range(&self) -> std::ops::Range<usize> {
        self.offset..self.offset + self.len()
    }
}

impl Architecture {
    /// The Starter table: it looks one byte back.
    pub const STARTER: Self = Self {
        context: 1,
        width: 256,
        layers: 0,
        heads: 0,
    };
    pub const MAX_CONTEXT: u16 = 1024;
    pub const MAX_WIDTH: u16 = 512;
    pub const MAX_LAYERS: u8 = 12;
    pub const MAX_HEADS: u8 = 16;

    pub const fn is_table(&self) -> bool {
        self.layers == 0
    }

    /// Whether this is the Starter table or a transformer inside the supported
    /// bounds whose file fits [`MAX_MODEL_BYTES`]. Checked before any allocation.
    pub fn validate(&self) -> Result<(), TrainingError> {
        let valid = if self.is_table() {
            *self == Self::STARTER
        } else {
            (2..=Self::MAX_CONTEXT).contains(&self.context)
                && (8..=Self::MAX_WIDTH).contains(&self.width)
                && self.layers <= Self::MAX_LAYERS
                && (1..=Self::MAX_HEADS).contains(&self.heads)
                && self.width.is_multiple_of(u16::from(self.heads))
                && self.file_len() <= MAX_MODEL_BYTES
        };
        if valid {
            Ok(())
        } else {
            Err(TrainingError::InvalidModel)
        }
    }

    /// The exact number of values in the flat parameter vector. The engine's real
    /// count must equal this for every model.
    pub const fn parameters(&self) -> usize {
        if self.is_table() {
            return VOCABULARY * VOCABULARY;
        }
        let context = self.context as usize;
        let width = self.width as usize;
        let layers = self.layers as usize;
        VOCABULARY * width
            + context * width
            + layers * (12 * width * width + 13 * width)
            + 2 * width
            + VOCABULARY * width
            + VOCABULARY
    }

    /// The exact length of this model's file.
    pub const fn file_len(&self) -> usize {
        MODEL_HEADER_BYTES + self.parameters() * 4
    }

    /// Whether the group's computers can help train this shape: its work packet
    /// and returned update must each fit one group message.
    pub const fn group_capable(&self) -> bool {
        self.parameters() <= MAX_GROUP_PARAMETERS
    }

    /// How many next-byte guesses one window of a `text_len`-byte packet teaches.
    /// The table learns from every neighbouring pair; a transformer reads up to
    /// its context, or less when the packet is shorter.
    pub fn window_tokens(&self, text_len: usize) -> usize {
        let pairs = text_len.saturating_sub(1);
        if self.is_table() {
            pairs
        } else {
            pairs.min(usize::from(self.context))
        }
    }

    /// Every piece of the flat parameter vector, in order, with no gaps. See the
    /// layout table on [`Architecture`].
    pub fn blocks(&self) -> Vec<ParameterBlock> {
        let mut blocks = Vec::new();
        let mut offset = 0;
        let mut push = |kind, layer, rows: usize, cols: usize| {
            blocks.push(ParameterBlock {
                kind,
                layer,
                offset,
                rows,
                cols,
            });
            offset += rows * cols;
        };
        if self.is_table() {
            push(BlockKind::Table, None, VOCABULARY, VOCABULARY);
            return blocks;
        }
        let context = usize::from(self.context);
        let width = usize::from(self.width);
        push(BlockKind::ByteEmbedding, None, VOCABULARY, width);
        push(BlockKind::PositionEmbedding, None, context, width);
        for layer in 0..self.layers {
            let layer = Some(layer);
            push(BlockKind::AttentionNormGain, layer, 1, width);
            push(BlockKind::AttentionNormBias, layer, 1, width);
            push(BlockKind::QueryKeyValueWeight, layer, width, 3 * width);
            push(BlockKind::QueryKeyValueBias, layer, 1, 3 * width);
            push(BlockKind::AttentionOutWeight, layer, width, width);
            push(BlockKind::AttentionOutBias, layer, 1, width);
            push(BlockKind::FeedForwardNormGain, layer, 1, width);
            push(BlockKind::FeedForwardNormBias, layer, 1, width);
            push(BlockKind::FeedForwardInWeight, layer, width, 4 * width);
            push(BlockKind::FeedForwardInBias, layer, 1, 4 * width);
            push(BlockKind::FeedForwardOutWeight, layer, 4 * width, width);
            push(BlockKind::FeedForwardOutBias, layer, 1, width);
        }
        push(BlockKind::FinalNormGain, None, 1, width);
        push(BlockKind::FinalNormBias, None, 1, width);
        push(BlockKind::OutputWeight, None, width, VOCABULARY);
        push(BlockKind::OutputBias, None, 1, VOCABULARY);
        blocks
    }
}

/// What a model file says about itself. It carries no name, person or text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelHeader {
    /// The starting point this model's family is shown as.
    pub base: BaseModel,
    pub architecture: Architecture,
    /// Rounds and text it has learned from in total, across every continued run.
    pub rounds: u32,
    pub tokens: u64,
    pub loss_milli: Option<u32>,
    pub check_loss_milli: Option<u32>,
}

impl ModelHeader {
    /// A model that has not learned anything yet.
    pub fn fresh(base: BaseModel) -> Self {
        Self {
            base,
            architecture: base.architecture(),
            rounds: 0,
            tokens: 0,
            loss_milli: None,
            check_loss_milli: None,
        }
    }

    pub fn validate(&self) -> Result<(), TrainingError> {
        self.architecture.validate()?;
        if (self.base == BaseModel::Starter) != self.architecture.is_table()
            || self.loss_milli == Some(NO_LOSS)
            || self.check_loss_milli == Some(NO_LOSS)
        {
            return Err(TrainingError::InvalidModel);
        }
        Ok(())
    }
}

/// A decoded model file.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelFile {
    pub header: ModelHeader,
    pub parameters: Vec<f32>,
}

/// What a checked model file is, without its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFacts {
    pub id: ModelId,
    pub header: ModelHeader,
    pub parameters: u64,
    pub bytes: u64,
}

pub fn model_id(file: &[u8]) -> ModelId {
    ContentHash::of(file)
}

/// Writes a model file: a fixed 64-byte header, then every value as a
/// little-endian `f32`.
///
/// | Bytes | Field |
/// |---|---|
/// | 0..8 | `SOVRAMDL` |
/// | 8..12 | format version (1) |
/// | 12, 13, 14, 15 | base model, layers, heads, zero |
/// | 16..18, 18..20 | context, width |
/// | 20..24, 24..32 | rounds, tokens |
/// | 32..36, 36..40 | loss, check loss in thousandths (`u32::MAX` = none) |
/// | 40..48 | number of values |
/// | 48..64 | zero |
///
/// The length is exactly [`Architecture::file_len`], so a file's size can be
/// checked against its card before it is fetched.
pub fn encode_model(header: &ModelHeader, parameters: &[f32]) -> Result<Vec<u8>, TrainingError> {
    header.validate()?;
    let architecture = header.architecture;
    if parameters.len() != architecture.parameters() || parameters.iter().any(|x| !x.is_finite()) {
        return Err(TrainingError::InvalidModel);
    }
    let mut out = Vec::with_capacity(architecture.file_len());
    out.extend_from_slice(MODEL_MAGIC);
    out.extend_from_slice(&MODEL_VERSION.to_le_bytes());
    out.extend_from_slice(&[
        header.base.code(),
        architecture.layers,
        architecture.heads,
        0,
    ]);
    out.extend_from_slice(&architecture.context.to_le_bytes());
    out.extend_from_slice(&architecture.width.to_le_bytes());
    out.extend_from_slice(&header.rounds.to_le_bytes());
    out.extend_from_slice(&header.tokens.to_le_bytes());
    out.extend_from_slice(&header.loss_milli.unwrap_or(NO_LOSS).to_le_bytes());
    out.extend_from_slice(&header.check_loss_milli.unwrap_or(NO_LOSS).to_le_bytes());
    out.extend_from_slice(&(parameters.len() as u64).to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    debug_assert_eq!(out.len(), MODEL_HEADER_BYTES);
    out.extend(parameters.iter().flat_map(|x| x.to_le_bytes()));
    Ok(out)
}

/// Reads and checks only the header and the length. Nothing the size of the model
/// is allocated.
fn decode_header(file: &[u8]) -> Result<ModelHeader, TrainingError> {
    let invalid = TrainingError::InvalidModel;
    if file.len() > MAX_MODEL_BYTES {
        return Err(invalid);
    }
    let head = file
        .first_chunk::<MODEL_HEADER_BYTES>()
        .ok_or(invalid.clone())?;
    let u16_at = |at: usize| u16::from_le_bytes([head[at], head[at + 1]]);
    let u32_at =
        |at: usize| u32::from_le_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    let u64_at = |at: usize| (u64::from(u32_at(at + 4)) << 32) | u64::from(u32_at(at));
    let loss = |at: usize| Some(u32_at(at)).filter(|value| *value != NO_LOSS);
    if head[..8] != *MODEL_MAGIC
        || u32_at(8) != MODEL_VERSION
        || head[15] != 0
        || head[48..].iter().any(|byte| *byte != 0)
    {
        return Err(invalid);
    }
    let header = ModelHeader {
        base: BaseModel::from_code(head[12]).ok_or(invalid.clone())?,
        architecture: Architecture {
            context: u16_at(16),
            width: u16_at(18),
            layers: head[13],
            heads: head[14],
        },
        rounds: u32_at(20),
        tokens: u64_at(24),
        loss_milli: loss(32),
        check_loss_milli: loss(36),
    };
    header.validate()?;
    if u64_at(40) != header.architecture.parameters() as u64
        || file.len() != header.architecture.file_len()
    {
        return Err(invalid);
    }
    Ok(header)
}

/// Reads a model file, refusing a wrong length, an unknown version, a shape
/// outside the supported bounds, and any value that is not a finite number.
pub fn decode_model(file: &[u8]) -> Result<ModelFile, TrainingError> {
    let header = decode_header(file)?;
    let parameters: Vec<f32> = file[MODEL_HEADER_BYTES..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    if parameters.iter().any(|x| !x.is_finite()) {
        return Err(TrainingError::InvalidModel);
    }
    Ok(ModelFile { header, parameters })
}

/// Checks a whole model file and says what it is, without keeping its values.
pub fn inspect_model(file: &[u8]) -> Result<ModelFacts, TrainingError> {
    let header = decode_header(file)?;
    let finite = file[MODEL_HEADER_BYTES..]
        .as_chunks::<4>()
        .0
        .iter()
        .all(|bytes| f32::from_le_bytes(*bytes).is_finite());
    if !finite {
        return Err(TrainingError::InvalidModel);
    }
    Ok(ModelFacts {
        id: model_id(file),
        header,
        parameters: header.architecture.parameters() as u64,
        bytes: file.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny transformer for fast tests; not a row a person can pick.
    const MICRO: Architecture = Architecture {
        context: 8,
        width: 8,
        layers: 1,
        heads: 2,
    };

    fn micro_header() -> ModelHeader {
        ModelHeader {
            base: BaseModel::Small,
            architecture: MICRO,
            rounds: 7,
            tokens: 9_000_000_000,
            loss_milli: Some(2345),
            check_loss_milli: None,
        }
    }

    #[test]
    fn the_table_has_one_row_per_model_in_order_with_the_documented_sizes() {
        assert_eq!(BASE_MODELS.len(), BaseModel::ALL.len());
        for (row, model) in BASE_MODELS.iter().zip(BaseModel::ALL) {
            assert_eq!(row.model, model);
            assert_eq!(model.info(), row);
            assert_eq!(BaseModel::from_code(model.code()), Some(model));
            assert!(!row.title.is_empty() && !row.summary.is_empty());
            assert_eq!(row.architecture.validate(), Ok(()));
            assert_eq!(row.group_capable, row.architecture.group_capable());
            assert_eq!(ModelHeader::fresh(model).validate(), Ok(()));
        }
        assert_eq!(BaseModel::from_code(4), None);
        let sizes: Vec<usize> = BaseModel::ALL
            .iter()
            .map(|model| model.architecture().parameters())
            .collect();
        assert_eq!(sizes, [65_536, 137_216, 875_520, 2_817_280]);
        assert_eq!(MICRO.parameters(), 5_304);
        assert!(!BaseModel::Large.info().group_capable);
        assert!(BaseModel::Medium.info().group_capable);
        assert_eq!(
            serde_json::to_string(&BaseModel::Medium).unwrap(),
            "\"medium\""
        );
    }

    #[test]
    fn blocks_tile_the_whole_vector_in_order_and_sum_to_the_formula() {
        let mut shapes = BaseModel::ALL.map(BaseModel::architecture).to_vec();
        shapes.push(MICRO);
        for architecture in shapes {
            let blocks = architecture.blocks();
            let mut next = 0;
            for block in &blocks {
                assert_eq!(block.offset, next, "{architecture:?} {block:?}");
                assert!(!block.is_empty());
                assert_eq!(block.range().len(), block.len());
                next += block.len();
            }
            assert_eq!(next, architecture.parameters(), "{architecture:?}");
        }
        assert_eq!(
            Architecture::STARTER.blocks(),
            [ParameterBlock {
                kind: BlockKind::Table,
                layer: None,
                offset: 0,
                rows: 256,
                cols: 256
            }]
        );
        let blocks = MICRO.blocks();
        assert_eq!(blocks.len(), 2 + 12 + 4);
        assert_eq!(blocks[0].kind, BlockKind::ByteEmbedding);
        assert_eq!((blocks[0].rows, blocks[0].cols), (256, 8));
        assert_eq!((blocks[1].rows, blocks[1].cols), (8, 8));
        assert_eq!(blocks[4].kind, BlockKind::QueryKeyValueWeight);
        assert_eq!((blocks[4].rows, blocks[4].cols), (8, 24));
        assert_eq!(blocks[4].layer, Some(0));
        assert_eq!(blocks[12].kind, BlockKind::FeedForwardOutWeight);
        assert_eq!((blocks[12].rows, blocks[12].cols), (32, 8));
        let last = blocks[17];
        assert_eq!(last.kind, BlockKind::OutputBias);
        assert_eq!((last.rows, last.cols, last.layer), (1, 256, None));
    }

    #[test]
    fn only_the_table_and_bounded_transformers_are_valid_shapes() {
        assert_eq!(MICRO.validate(), Ok(()));
        let bad = [
            Architecture {
                context: 2,
                ..Architecture::STARTER
            },
            Architecture {
                width: 64,
                ..Architecture::STARTER
            },
            Architecture {
                heads: 1,
                ..Architecture::STARTER
            },
            Architecture {
                context: 1,
                ..MICRO
            },
            Architecture {
                context: 1025,
                ..MICRO
            },
            Architecture { width: 4, ..MICRO },
            Architecture {
                width: 520,
                ..MICRO
            },
            Architecture { width: 9, ..MICRO },
            Architecture { heads: 0, ..MICRO },
            Architecture {
                heads: 17,
                width: 34,
                ..MICRO
            },
            Architecture {
                layers: 13,
                ..MICRO
            },
            // Inside every single bound, but its file would be far too large.
            Architecture {
                context: 1024,
                width: 512,
                layers: 12,
                heads: 16,
            },
        ];
        for architecture in bad {
            assert_eq!(
                architecture.validate(),
                Err(TrainingError::InvalidModel),
                "{architecture:?}"
            );
        }
        assert!(
            serde_json::from_str::<Architecture>(
                r#"{"context":8,"width":8,"layers":1,"heads":2,"run":"x"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn windows_count_the_guesses_a_packet_can_teach() {
        assert_eq!(Architecture::STARTER.window_tokens(0), 0);
        assert_eq!(Architecture::STARTER.window_tokens(1), 0);
        assert_eq!(Architecture::STARTER.window_tokens(513), 512);
        assert_eq!(MICRO.window_tokens(1), 0);
        assert_eq!(MICRO.window_tokens(5), 4);
        assert_eq!(MICRO.window_tokens(9), 8);
        assert_eq!(MICRO.window_tokens(4096), 8);
    }

    #[test]
    fn a_model_file_round_trips_with_an_exact_length_and_a_content_id() {
        let parameters: Vec<f32> = (0..MICRO.parameters())
            .map(|i| f32::from(u16::try_from(i).unwrap()) * 0.001 - 1.0)
            .collect();
        let header = micro_header();
        let file = encode_model(&header, &parameters).unwrap();
        assert_eq!(file.len(), MICRO.file_len());
        assert_eq!(file.len(), MODEL_HEADER_BYTES + 5_304 * 4);
        let decoded = decode_model(&file).unwrap();
        assert_eq!(decoded.header, header);
        assert_eq!(decoded.parameters, parameters);
        let facts = inspect_model(&file).unwrap();
        assert_eq!(facts.id, ContentHash::of(&file));
        assert_eq!(facts.id, model_id(&file));
        assert_eq!(facts.header, header);
        assert_eq!(facts.parameters, 5_304);
        assert_eq!(facts.bytes, file.len() as u64);

        let fresh = encode_model(
            &ModelHeader::fresh(BaseModel::Starter),
            &vec![0.0; VOCABULARY * VOCABULARY],
        )
        .unwrap();
        assert_eq!(fresh.len(), Architecture::STARTER.file_len());
        assert_eq!(
            decode_model(&fresh).unwrap().header,
            ModelHeader::fresh(BaseModel::Starter)
        );
    }

    #[test]
    fn encoding_refuses_a_wrong_count_bad_values_or_a_mismatched_family() {
        let good = vec![0.0; MICRO.parameters()];
        let header = micro_header();
        assert!(encode_model(&header, &good[1..]).is_err());
        let mut values = good.clone();
        values[3] = f32::NAN;
        assert!(encode_model(&header, &values).is_err());
        values[3] = f32::NEG_INFINITY;
        assert!(encode_model(&header, &values).is_err());
        for header in [
            ModelHeader {
                base: BaseModel::Starter,
                ..header
            },
            ModelHeader {
                architecture: Architecture::STARTER,
                ..header
            },
            ModelHeader {
                loss_milli: Some(u32::MAX),
                ..header
            },
            ModelHeader {
                check_loss_milli: Some(u32::MAX),
                ..header
            },
        ] {
            assert_eq!(
                encode_model(&header, &good),
                Err(TrainingError::InvalidModel)
            );
        }
    }

    #[test]
    fn decoding_refuses_every_damaged_truncated_or_padded_file() {
        let file = encode_model(&micro_header(), &vec![0.5; MICRO.parameters()]).unwrap();
        for cut in [0, 1, 63, 64, 65, file.len() - 4, file.len() - 1] {
            assert_eq!(
                decode_model(&file[..cut]),
                Err(TrainingError::InvalidModel),
                "cut {cut}"
            );
            assert!(inspect_model(&file[..cut]).is_err());
        }
        let mut longer = file.clone();
        longer.extend_from_slice(&[0; 4]);
        assert!(decode_model(&longer).is_err());
        // Every header byte is either checked or part of a checked field.
        for (at, value) in [
            (0, b'X'), // magic
            (8, 2),    // version
            (12, 0),   // family says Starter for a transformer
            (12, 9),   // unknown family
            (13, 2),   // layers: the length no longer matches
            (14, 3),   // heads do not divide the width
            (15, 1),   // reserved
            (16, 9),   // context: the length no longer matches
            (18, 16),  // width: the length no longer matches
            (40, 1),   // declared count
            (47, 1),   // declared count, high byte
            (48, 1),   // reserved
            (63, 1),   // reserved
        ] {
            let mut damaged = file.clone();
            damaged[at] = value;
            assert_eq!(
                decode_model(&damaged),
                Err(TrainingError::InvalidModel),
                "byte {at}"
            );
            assert!(inspect_model(&damaged).is_err(), "byte {at}");
        }
        let mut value = file.clone();
        let at = value.len() - 4;
        value[at..].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert!(decode_model(&value).is_err());
        assert!(inspect_model(&value).is_err());
        assert!(decode_model(&vec![0; MAX_MODEL_BYTES + 1]).is_err());
        // A different file is a different model.
        let mut other = file.clone();
        other[20] ^= 1;
        assert_ne!(
            inspect_model(&other).unwrap().id,
            inspect_model(&file).unwrap().id
        );
    }
}
