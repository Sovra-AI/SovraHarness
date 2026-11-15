//! A finished model written as a GGUF file, the format Ollama, llama.cpp and
//! LM Studio read.
//!
//! Small, Medium and Large are the shape those programs call `gpt2`: learned
//! positions, a normalisation before attention and before the feed-forward
//! part, one joined query-key-value matrix, and an output layer of its own. So a
//! model goes across value for value; nothing is approximated or retrained here.
//! Two things differ on the other side and are theirs to decide: llama.cpp
//! works out GELU with a close approximation, not the exact form used in
//! training, and it reads the output offsets only in recent versions (Ollama
//! 0.40.15 and later). The Starter table is not a transformer and has no GGUF
//! form.
//!
//! The layout is GGUF version 3 as written down in the ggml project's
//! `docs/gguf.md`. Only bytes are produced; nothing is read from or sent to
//! another program here.

use super::{Architecture, BlockKind, ParameterBlock, TrainingError, VOCABULARY, decode_model};

/// The architecture name llama.cpp knows this shape by.
const ARCHITECTURE: &str = "gpt2";
/// Tensor data starts, and every tensor is padded to, a multiple of this.
const ALIGNMENT: usize = 32;
/// The same, as the file states it.
const ALIGNMENT_KEY: u32 = 32;
const _: () = assert!(ALIGNMENT == ALIGNMENT_KEY as usize);
/// The same value as `NORM_EPSILON` in the training engine.
const NORM_EPSILON: f32 = 1e-5;

// GGUF value type ids.
const UINT32: u32 = 4;
const INT32: u32 = 5;
const FLOAT32: u32 = 6;
const BOOL: u32 = 7;
const STRING: u32 = 8;
const ARRAY: u32 = 9;
/// The ggml tensor type for 32-bit floats.
const F32: u32 = 0;
/// A token of the ordinary kind, as opposed to a control or unused one.
const NORMAL_TOKEN: i32 = 1;

/// Whether a model file's output offsets are all zero. Such a model is written
/// without them, which changes nothing about what it computes and lets older
/// readers, which do not know that part, load it.
fn offsets_are_zero(blocks: &[ParameterBlock], parameters: &[f32]) -> bool {
    blocks
        .iter()
        .filter(|block| block.kind == BlockKind::OutputBias)
        .all(|block| parameters[block.range()].iter().all(|value| *value == 0.0))
}

/// Whether the GGUF form of this model file needs a reader that knows output
/// offsets (llama.cpp since November 2026, Ollama 0.40.15). Every trained
/// transformer does; only a model whose offsets are exactly zero does not.
pub fn gguf_needs_output_offsets(model: &[u8]) -> Result<bool, TrainingError> {
    let model = decode_model(model)?;
    let architecture = model.header.architecture;
    if !exports_to_gguf(architecture) {
        return Err(TrainingError::InvalidModel);
    }
    Ok(!offsets_are_zero(&architecture.blocks(), &model.parameters))
}

/// Whether a model of this shape can be written as GGUF: every transformer
/// can, the Starter table cannot.
pub fn exports_to_gguf(architecture: Architecture) -> bool {
    !architecture.is_table()
}

struct Writer(Vec<u8>);

impl Writer {
    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: usize) {
        self.0.extend_from_slice(&(value as u64).to_le_bytes());
    }

    fn string(&mut self, text: &str) {
        self.u64(text.len());
        self.0.extend_from_slice(text.as_bytes());
    }

    fn key(&mut self, name: &str, kind: u32) {
        self.string(name);
        self.u32(kind);
    }

    fn text(&mut self, name: &str, value: &str) {
        self.key(name, STRING);
        self.string(value);
    }

    fn number(&mut self, name: &str, value: u32) {
        self.key(name, UINT32);
        self.u32(value);
    }

    fn pad(&mut self) {
        while !self.0.len().is_multiple_of(ALIGNMENT) {
            self.0.push(0);
        }
    }
}

/// The tensor a block becomes: its name, and whether it is a matrix that turns
/// its input (which GGUF stores the other way round from the model file).
fn tensor(block: &ParameterBlock) -> Option<(String, bool)> {
    let layer = block.layer.unwrap_or(0);
    let named = |suffix: &str| format!("blk.{layer}.{suffix}");
    Some(match block.kind {
        // Not a transformer block: callers refuse the Starter table first.
        BlockKind::Table => return None,
        // One row per byte and per position, as GGUF wants them.
        BlockKind::ByteEmbedding => ("token_embd.weight".to_owned(), false),
        BlockKind::PositionEmbedding => ("position_embd.weight".to_owned(), false),
        BlockKind::AttentionNormGain => (named("attn_norm.weight"), false),
        BlockKind::AttentionNormBias => (named("attn_norm.bias"), false),
        BlockKind::QueryKeyValueWeight => (named("attn_qkv.weight"), true),
        BlockKind::QueryKeyValueBias => (named("attn_qkv.bias"), false),
        BlockKind::AttentionOutWeight => (named("attn_output.weight"), true),
        BlockKind::AttentionOutBias => (named("attn_output.bias"), false),
        BlockKind::FeedForwardNormGain => (named("ffn_norm.weight"), false),
        BlockKind::FeedForwardNormBias => (named("ffn_norm.bias"), false),
        BlockKind::FeedForwardInWeight => (named("ffn_up.weight"), true),
        BlockKind::FeedForwardInBias => (named("ffn_up.bias"), false),
        BlockKind::FeedForwardOutWeight => (named("ffn_down.weight"), true),
        BlockKind::FeedForwardOutBias => (named("ffn_down.bias"), false),
        BlockKind::FinalNormGain => ("output_norm.weight".to_owned(), false),
        BlockKind::FinalNormBias => ("output_norm.bias".to_owned(), false),
        BlockKind::OutputWeight => ("output.weight".to_owned(), true),
        BlockKind::OutputBias => ("output.bias".to_owned(), false),
    })
}

/// Writes a model file (as `encode_model` makes it) as a GGUF file called
/// `name`. Refuses the Starter table and anything `decode_model` refuses.
pub fn to_gguf(model: &[u8], name: &str) -> Result<Vec<u8>, TrainingError> {
    let model = decode_model(model)?;
    let architecture = model.header.architecture;
    if !exports_to_gguf(architecture) {
        return Err(TrainingError::InvalidModel);
    }
    let mut blocks = architecture.blocks();
    if offsets_are_zero(&blocks, &model.parameters) {
        blocks.retain(|block| block.kind != BlockKind::OutputBias);
    }
    let width = u32::from(architecture.width);
    let mut out = Writer(Vec::with_capacity(model.parameters.len() * 4 + 16 * 1024));
    out.0.extend_from_slice(b"GGUF");
    out.u32(3);
    out.u64(blocks.len());
    out.u64(14);

    out.text("general.architecture", ARCHITECTURE);
    out.text("general.name", name);
    out.number("general.alignment", ALIGNMENT_KEY);
    // Zero: every value is a 32-bit float.
    out.number("general.file_type", 0);
    out.number("gpt2.context_length", u32::from(architecture.context));
    out.number("gpt2.embedding_length", width);
    out.number("gpt2.block_count", u32::from(architecture.layers));
    out.number("gpt2.feed_forward_length", width * 4);
    out.number("gpt2.attention.head_count", u32::from(architecture.heads));
    out.key("gpt2.attention.layer_norm_epsilon", FLOAT32);
    out.0.extend_from_slice(&NORM_EPSILON.to_le_bytes());
    // One token for each byte, written `\x00` to `\xff`. llama.cpp's "rwkv"
    // reader turns these back into single raw bytes and matches text against
    // them byte by byte, with no start or end token: exactly how a model reads
    // here. The hex must be lower case for its parser.
    out.text("tokenizer.ggml.model", "rwkv");
    out.key("tokenizer.ggml.tokens", ARRAY);
    out.u32(STRING);
    out.u64(VOCABULARY);
    for byte in 0..VOCABULARY {
        out.string(&format!("\\x{byte:02x}"));
    }
    out.key("tokenizer.ggml.token_type", ARRAY);
    out.u32(INT32);
    out.u64(VOCABULARY);
    for _ in 0..VOCABULARY {
        out.0.extend_from_slice(&NORMAL_TOKEN.to_le_bytes());
    }
    out.key("tokenizer.ggml.add_bos_token", BOOL);
    out.0.push(0);

    // Where each tensor is, then the values themselves in the same order.
    let mut offset = 0_usize;
    let mut plan = Vec::with_capacity(blocks.len());
    for block in &blocks {
        let (name, turns) = tensor(block).ok_or(TrainingError::InvalidModel)?;
        out.string(&name);
        if block.rows == 1 {
            out.u32(1);
            out.u64(block.cols);
        } else {
            // GGUF lists the innermost size first. A matrix that turns its input
            // is stored here as [inputs, outputs] and there as [outputs][inputs],
            // which it describes as (inputs, outputs). An embedding is already
            // one row per entry, described as (width, entries).
            out.u32(2);
            let (first, second) = if turns {
                (block.rows, block.cols)
            } else {
                (block.cols, block.rows)
            };
            out.u64(first);
            out.u64(second);
        }
        out.u32(F32);
        out.u64(offset);
        plan.push((*block, turns));
        offset += (block.len() * 4).next_multiple_of(ALIGNMENT);
    }
    out.pad();
    for (block, turns) in plan {
        let values = &model.parameters[block.range()];
        if turns {
            for column in 0..block.cols {
                for row in 0..block.rows {
                    out.0
                        .extend_from_slice(&values[row * block.cols + column].to_le_bytes());
                }
            }
        } else {
            for value in values {
                out.0.extend_from_slice(&value.to_le_bytes());
            }
        }
        out.pad();
    }
    Ok(out.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::training::{BASE_MODELS, BaseModel, ModelHeader, encode_model};

    const MICRO: Architecture = Architecture {
        context: 8,
        width: 8,
        layers: 1,
        heads: 2,
    };

    fn file(architecture: Architecture) -> (Vec<u8>, Vec<f32>) {
        // Every value is its own position, so a value can be traced to its place.
        #[expect(clippy::cast_precision_loss, reason = "small test counts")]
        let values: Vec<f32> = (0..architecture.parameters()).map(|i| i as f32).collect();
        let header = ModelHeader {
            base: if architecture.is_table() {
                BaseModel::Starter
            } else {
                BaseModel::Small
            },
            architecture,
            rounds: 3,
            tokens: 4,
            loss_milli: None,
            check_loss_milli: None,
        };
        (encode_model(&header, &values).unwrap(), values)
    }

    /// A reader just strict enough to check what the writer made.
    struct Reader<'a>(&'a [u8], usize);

    impl Reader<'_> {
        fn take(&mut self, count: usize) -> &[u8] {
            let part = &self.0[self.1..self.1 + count];
            self.1 += count;
            part
        }

        fn u32(&mut self) -> u32 {
            u32::from_le_bytes(self.take(4).try_into().unwrap())
        }

        fn u64(&mut self) -> usize {
            usize::try_from(u64::from_le_bytes(self.take(8).try_into().unwrap())).unwrap()
        }

        fn string(&mut self) -> String {
            let len = self.u64();
            String::from_utf8(self.take(len).to_vec()).unwrap()
        }

        /// Skips one value of `kind` and gives it back as text for the simple kinds.
        fn value(&mut self, kind: u32) -> String {
            match kind {
                UINT32 => self.u32().to_string(),
                INT32 => i32::from_le_bytes(self.take(4).try_into().unwrap()).to_string(),
                FLOAT32 => f32::from_le_bytes(self.take(4).try_into().unwrap()).to_string(),
                BOOL => self.take(1)[0].to_string(),
                STRING => self.string(),
                ARRAY => {
                    let inner = self.u32();
                    let count = self.u64();
                    let items: Vec<String> = (0..count).map(|_| self.value(inner)).collect();
                    items.join(",")
                }
                other => panic!("unexpected value type {other}"),
            }
        }
    }

    struct Parsed {
        keys: Vec<(String, String)>,
        /// Name, sizes as written, offset.
        tensors: Vec<(String, Vec<usize>, usize)>,
        data: usize,
    }

    fn parse(bytes: &[u8]) -> Parsed {
        let mut reader = Reader(bytes, 0);
        assert_eq!(reader.take(4), b"GGUF");
        assert_eq!(reader.u32(), 3);
        let tensors = reader.u64();
        let keys = reader.u64();
        let keys = (0..keys)
            .map(|_| {
                let name = reader.string();
                let kind = reader.u32();
                (name, reader.value(kind))
            })
            .collect();
        let tensors = (0..tensors)
            .map(|_| {
                let name = reader.string();
                let dims = reader.u32();
                let sizes = (0..dims).map(|_| reader.u64()).collect();
                assert_eq!(reader.u32(), F32);
                (name, sizes, reader.u64())
            })
            .collect();
        Parsed {
            keys,
            tensors,
            data: reader.1.next_multiple_of(ALIGNMENT),
        }
    }

    fn values(bytes: &[u8], parsed: &Parsed, name: &str) -> Vec<f32> {
        let (_, sizes, offset) = parsed
            .tensors
            .iter()
            .find(|(found, ..)| found == name)
            .unwrap();
        let count: usize = sizes.iter().product();
        let start = parsed.data + offset;
        bytes[start..start + count * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect()
    }

    #[test]
    fn a_model_becomes_a_gpt2_gguf_with_one_token_for_each_byte() {
        let (model, _) = file(MICRO);
        let gguf = to_gguf(&model, "Garden notes").unwrap();
        let parsed = parse(&gguf);
        let key = |name: &str| {
            parsed
                .keys
                .iter()
                .find(|(found, _)| found == name)
                .map_or_else(|| panic!("{name} is written"), |(_, value)| value.clone())
        };
        assert_eq!(key("general.architecture"), "gpt2");
        assert_eq!(key("general.name"), "Garden notes");
        assert_eq!(key("gpt2.context_length"), "8");
        assert_eq!(key("gpt2.embedding_length"), "8");
        assert_eq!(key("gpt2.block_count"), "1");
        assert_eq!(key("gpt2.feed_forward_length"), "32");
        assert_eq!(key("gpt2.attention.head_count"), "2");
        assert_eq!(key("gpt2.attention.layer_norm_epsilon"), "0.00001");
        assert_eq!(key("tokenizer.ggml.model"), "rwkv");
        assert_eq!(key("tokenizer.ggml.add_bos_token"), "0");
        let tokens = key("tokenizer.ggml.tokens");
        let tokens: Vec<&str> = tokens.split(',').collect();
        assert_eq!(tokens.len(), 256);
        assert_eq!(
            (tokens[0], tokens[10], tokens[255]),
            ("\\x00", "\\x0a", "\\xff")
        );
        assert_eq!(key("tokenizer.ggml.token_type").split(',').count(), 256);
        // No key is written twice, and the count in the header is the truth.
        let mut names: Vec<&str> = parsed.keys.iter().map(|(name, _)| name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), parsed.keys.len());
    }

    #[test]
    fn every_block_is_written_once_in_the_place_and_direction_gguf_expects() {
        let (model, source) = file(MICRO);
        let gguf = to_gguf(&model, "m").unwrap();
        let parsed = parse(&gguf);
        // Six tensors outside the layers and twelve in each layer, no more.
        assert_eq!(parsed.tensors.len(), 6 + 12);
        let mut expected = 0;
        for (name, sizes, offset) in &parsed.tensors {
            assert!(name.len() < 64);
            assert_eq!(*offset, expected, "{name} follows the one before");
            assert!(offset.is_multiple_of(ALIGNMENT));
            expected += (sizes.iter().product::<usize>() * 4).next_multiple_of(ALIGNMENT);
        }
        assert_eq!(gguf.len(), parsed.data + expected);
        let sizes = |name: &str| {
            parsed
                .tensors
                .iter()
                .find(|(found, ..)| found == name)
                .unwrap()
                .1
                .clone()
        };
        // Innermost size first: (width, entries) and (inputs, outputs).
        assert_eq!(sizes("token_embd.weight"), [8, 256]);
        assert_eq!(sizes("position_embd.weight"), [8, 8]);
        assert_eq!(sizes("blk.0.attn_qkv.weight"), [8, 24]);
        assert_eq!(sizes("blk.0.attn_qkv.bias"), [24]);
        assert_eq!(sizes("blk.0.ffn_up.weight"), [8, 32]);
        assert_eq!(sizes("blk.0.ffn_down.weight"), [32, 8]);
        assert_eq!(sizes("output.weight"), [8, 256]);
        assert_eq!(sizes("output.bias"), [256]);

        let blocks = MICRO.blocks();
        let block = |kind| *blocks.iter().find(|block| block.kind == kind).unwrap();
        // An embedding is copied as it stands.
        let embedding = block(BlockKind::ByteEmbedding);
        assert_eq!(
            values(&gguf, &parsed, "token_embd.weight"),
            source[embedding.range()]
        );
        // A matrix that turns its input is laid out the other way round: the
        // value for (input 2, output 5) moves to (output 5, input 2).
        let qkv = block(BlockKind::QueryKeyValueWeight);
        let written = values(&gguf, &parsed, "blk.0.attn_qkv.weight");
        assert_eq!(
            written[5 * qkv.rows + 2],
            source[qkv.offset + 2 * qkv.cols + 5]
        );
        // The square matrix is turned as well, where a mistake would not show
        // in its sizes.
        let out = block(BlockKind::AttentionOutWeight);
        let written = values(&gguf, &parsed, "blk.0.attn_output.weight");
        assert_eq!(written[3 * 8 + 1], source[out.offset + 8 + 3]);
        assert_ne!(written[3 * 8 + 1], source[out.offset + 3 * 8 + 1]);
        let bias = block(BlockKind::OutputBias);
        assert_eq!(values(&gguf, &parsed, "output.bias"), source[bias.range()]);
    }

    #[test]
    fn every_transformer_exports_and_the_starter_table_does_not() {
        for row in BASE_MODELS {
            let (model, _) = file(row.architecture);
            assert_eq!(
                exports_to_gguf(row.architecture),
                row.model != BaseModel::Starter
            );
            match to_gguf(&model, row.title) {
                Ok(gguf) => {
                    assert!(exports_to_gguf(row.architecture));
                    let parsed = parse(&gguf);
                    assert_eq!(
                        parsed.tensors.len(),
                        6 + 12 * usize::from(row.architecture.layers)
                    );
                    // Every value is there, plus a little for names and padding.
                    assert!(gguf.len() >= model.len() - 64);
                    assert!(gguf.len() < model.len() + 64 * 1024);
                }
                Err(error) => {
                    assert_eq!(row.model, BaseModel::Starter);
                    assert_eq!(error, TrainingError::InvalidModel);
                }
            }
        }
        assert_eq!(
            to_gguf(b"not a model", "m"),
            Err(TrainingError::InvalidModel)
        );
    }

    #[test]
    fn output_offsets_of_exactly_zero_are_left_out_for_older_readers() {
        let (model, mut values) = file(MICRO);
        assert_eq!(gguf_needs_output_offsets(&model), Ok(true));
        let bias = *MICRO
            .blocks()
            .iter()
            .find(|block| block.kind == BlockKind::OutputBias)
            .unwrap();
        values[bias.range()].fill(0.0);
        let header = ModelHeader {
            base: BaseModel::Small,
            architecture: MICRO,
            rounds: 3,
            tokens: 4,
            loss_milli: None,
            check_loss_milli: None,
        };
        let plain = encode_model(&header, &values).unwrap();
        assert_eq!(gguf_needs_output_offsets(&plain), Ok(false));
        let parsed = parse(&to_gguf(&plain, "m").unwrap());
        assert_eq!(parsed.tensors.len(), 5 + 12);
        assert!(
            parsed
                .tensors
                .iter()
                .all(|(name, ..)| name != "output.bias")
        );
        // One value off zero and the offsets are written again.
        values[bias.offset] = 0.5;
        let offset = encode_model(&header, &values).unwrap();
        assert_eq!(gguf_needs_output_offsets(&offset), Ok(true));
        assert_eq!(parse(&to_gguf(&offset, "m").unwrap()).tensors.len(), 6 + 12);
        let (table, _) = file(Architecture::STARTER);
        assert!(gguf_needs_output_offsets(&table).is_err());
    }
}
