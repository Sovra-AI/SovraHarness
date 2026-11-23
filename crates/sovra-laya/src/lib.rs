//! Laya inside the Sovra executable: rates how hard a message is, on the CPU.
//!
//! Owns the `laya-candle` port of Convai's Laya (Apache-2.0) and its candle tensor
//! stack, behind the core [`DifficultyJudge`] and [`LayaLoader`] ports. No server,
//! no Python, no network: the runtime downloads and verifies the weights, then hands
//! this crate the folder. Inference is blocking; the runtime runs it off its async
//! threads with a deadline.

use std::path::Path;
use std::sync::Arc;

use laya::{Agent, Checkpoint, DeviceChoice, Question, Questions};
use sovra_core::ai_tiers::{
    DifficultyJudge, LAYA_DIFFICULTY_LEVELS, LAYA_DIFFICULTY_QUESTION, LayaJudgement, LayaLoader,
};

/// Longest message text Laya reads. Its multilingual checkpoint sees 1024 tokens;
/// the start of a message carries what it asks for.
const MAX_REQUEST_CHARS: usize = 4_000;
const QUESTION_ID: &str = "difficulty";

/// Loads Laya on the CPU so every computer behaves the same.
#[derive(Debug, Default, Clone, Copy)]
pub struct CpuLayaLoader;

impl LayaLoader for CpuLayaLoader {
    fn load(&self, dir: &Path) -> Result<Arc<dyn DifficultyJudge>, String> {
        Ok(Arc::new(LayaJudge::load(dir)?))
    }
}

/// A loaded Laya checkpoint and the one question it is asked.
pub struct LayaJudge {
    agent: Agent,
    questions: Questions,
}

impl std::fmt::Debug for LayaJudge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayaJudge")
            .field("checkpoint", &self.agent.label())
            .finish_non_exhaustive()
    }
}

impl LayaJudge {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let checkpoint = Checkpoint::from_dir(dir).map_err(|error| error.to_string())?;
        let device = DeviceChoice::Cpu
            .resolve()
            .map_err(|error| error.to_string())?;
        let agent = Agent::load(&checkpoint, &device).map_err(|error| error.to_string())?;
        tracing::info!(checkpoint = %agent.label(), "Laya loaded");
        Ok(Self {
            agent,
            questions: difficulty_questions(),
        })
    }
}

fn difficulty_questions() -> Questions {
    let question = LAYA_DIFFICULTY_LEVELS.iter().fold(
        Question::score(LAYA_DIFFICULTY_QUESTION),
        |question, level| question.level(*level),
    );
    Questions::new().with(QUESTION_ID, question)
}

impl DifficultyJudge for LayaJudge {
    fn judge(&self, request: &str) -> Result<LayaJudgement, String> {
        let clipped: String = request.chars().take(MAX_REQUEST_CHARS).collect();
        let prediction = self
            .agent
            .predict(serde_json::json!({ "request": clipped }), &self.questions)
            .map_err(|error| error.to_string())?;
        let answer = prediction
            .get(QUESTION_ID)
            .ok_or_else(|| "Laya returned no difficulty answer".to_owned())?;
        let difficulty = answer
            .as_score()
            .ok_or_else(|| "Laya returned a non-score answer".to_owned())?;
        Ok(LayaJudgement {
            difficulty,
            confidence: answer.confidence(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_one_score_question_on_the_core_scale() {
        let questions = difficulty_questions();
        assert_eq!(questions.len(), 1);
        let (id, question) = questions.iter().next().expect("one question");
        assert_eq!(id, QUESTION_ID);
        let levels = question.render_options(id).expect("score levels render");
        assert_eq!(levels.len(), LAYA_DIFFICULTY_LEVELS.len());
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_panic() {
        let error = LayaJudge::load(Path::new("/nonexistent/sovra-laya")).err();
        assert!(error.is_some());
    }

    /// Needs the downloaded weights; the folder is read at build time:
    /// `SOVRA_LAYA_DIR=<folder> cargo test -p sovra-laya --release -- --ignored`.
    #[test]
    #[ignore = "needs Laya's 640 MB weights on disk"]
    fn rates_a_greeting_easier_than_a_proof() {
        let Some(dir) = option_env!("SOVRA_LAYA_DIR") else {
            return;
        };
        let judge = LayaJudge::load(Path::new(&dir)).expect("Laya loads");
        let easy = judge.judge("hi there").expect("judges greeting");
        let hard = judge
            .judge("Prove that there are infinitely many primes of the form 4k+3, then implement a sieve in Rust that counts them below 10^9 efficiently.")
            .expect("judges proof");
        assert!(easy.difficulty < hard.difficulty, "{easy:?} vs {hard:?}");
    }
}
