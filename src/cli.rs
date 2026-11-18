//! Terminal-facing interaction layer.
//!
//! This module currently provides the concrete [`UserInteraction`] used by
//! `ask_user()`. First-run onboarding (sandbox choice, provider API keys, and
//! resumable state) will live here as the CLI grows; it is intentionally not
//! part of this implementation yet.

use std::io::{self, BufRead, Write};
use std::sync::Mutex;

use crate::tools::external::UserInteraction;

/// A console-backed implementation of [`UserInteraction`].
///
/// Calls are serialized because stdin is process-wide. The blocking terminal
/// read is dispatched to Tokio's blocking pool so it cannot stall an async
/// worker.
#[derive(Clone, Copy, Debug, Default)]
pub struct CliUserInteraction;

// Every instance reads from the same stdin stream, so the lock must be global.
static PROMPT_LOCK: Mutex<()> = Mutex::new(());

impl CliUserInteraction {
    /// Create a console interaction handler.
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PromptOutcome {
    Answer(String),
    EndOfInput,
}

#[async_trait::async_trait]
impl UserInteraction for CliUserInteraction {
    async fn ask(&self, question: String, recommendation: String) -> String {
        // Acquire the process-wide lock inside the blocking closure. If this
        // async future is cancelled, the closure may still be blocked on stdin;
        // it must retain the lock until that read actually finishes.
        match tokio::task::spawn_blocking(move || {
            let _prompt_guard = PROMPT_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            prompt_on_stdio(&question, &recommendation)
        })
        .await
        {
            Ok(Ok(PromptOutcome::Answer(answer))) => answer,
            Ok(Ok(PromptOutcome::EndOfInput)) => no_answer_message(
                "standard input reached EOF before a response was received",
            ),
            Ok(Err(error)) => no_answer_message(&format!(
                "could not read the response from the terminal: {error}"
            )),
            Err(error) => no_answer_message(&format!(
                "the terminal interaction task failed: {error}"
            )),
        }
    }
}

fn no_answer_message(reason: &str) -> String {
    format!(
        "ERROR: {reason}. No user answer was submitted. Do not continue the task; ask the user again when Echelon is running with an available interactive stdin."
    )
}

fn prompt_on_stdio(question: &str, recommendation: &str) -> io::Result<PromptOutcome> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();

    prompt(&mut input, &mut output, question, recommendation)
}

fn prompt<R, W>(
    input: &mut R,
    output: &mut W,
    question: &str,
    recommendation: &str,
) -> io::Result<PromptOutcome>
where
    R: BufRead,
    W: Write,
{
    writeln!(output, "\n=== Echelon needs your input ===")?;
    writeln!(output, "Question:\n{}", terminal_safe_text(question))?;
    writeln!(output, "\nRecommendation:\n{}", terminal_safe_text(recommendation))?;
    write!(
        output,
        "\nYour response (one line; press Enter to accept the recommendation): "
    )?;
    output.flush()?;

    let mut response = String::new();
    let bytes_read = input.read_line(&mut response)?;
    if bytes_read == 0 {
        return Ok(PromptOutcome::EndOfInput);
    }

    let response = response.trim();
    if response.is_empty() {
        Ok(PromptOutcome::Answer(recommendation.to_owned()))
    } else {
        Ok(PromptOutcome::Answer(response.to_owned()))
    }
}

/// Remove terminal control sequences from model-generated text before
/// displaying it. Newlines and tabs are retained for readable formatting.
fn terminal_safe_text(value: &str) -> String {
    let mut safe = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\u{001b}' => match chars.next() {
                // CSI: ESC [ ... final-byte (0x40..=0x7e).
                Some('[') => {
                    for sequence_char in chars.by_ref() {
                        if ('@'..='~').contains(&sequence_char) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] ... BEL or ESC \\
                Some(']') => {
                    while let Some(sequence_char) = chars.next() {
                        if sequence_char == '\u{0007}' {
                            break;
                        }
                        if sequence_char == '\u{001b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Drop other two-character ESC sequences.
                Some(_) | None => {}
            },
            _ if !ch.is_control() || ch == '\n' || ch == '\t' => safe.push(ch),
            _ => {}
        }
    }

    safe
}

#[cfg(test)]
mod tests {
    use super::{prompt, terminal_safe_text, PromptOutcome};
    use std::io::Cursor;

    #[test]
    fn typed_response_overrides_recommendation() {
        let mut input = Cursor::new(&b"Use SQLite instead.\n"[..]);
        let mut output = Vec::new();

        let outcome = prompt(
            &mut input,
            &mut output,
            "Which database should we use?",
            "I recommend PostgreSQL.",
        )
        .unwrap();

        assert_eq!(
            outcome,
            PromptOutcome::Answer("Use SQLite instead.".to_owned())
        );
        let displayed = String::from_utf8(output).unwrap();
        assert!(displayed.contains("Which database should we use?"));
        assert!(displayed.contains("I recommend PostgreSQL."));
        assert!(displayed.contains("press Enter to accept the recommendation"));
    }

    #[test]
    fn empty_response_accepts_recommendation() {
        let mut input = Cursor::new(&b"\n"[..]);
        let mut output = Vec::new();

        let outcome = prompt(
            &mut input,
            &mut output,
            "Which database should we use?",
            "I recommend PostgreSQL.",
        )
        .unwrap();

        assert_eq!(
            outcome,
            PromptOutcome::Answer("I recommend PostgreSQL.".to_owned())
        );
    }

    #[test]
    fn end_of_input_is_not_treated_as_acceptance() {
        let mut input = Cursor::new(&b""[..]);
        let mut output = Vec::new();

        let outcome = prompt(
            &mut input,
            &mut output,
            "Which database should we use?",
            "I recommend PostgreSQL.",
        )
        .unwrap();

        assert_eq!(outcome, PromptOutcome::EndOfInput);
    }

    #[test]
    fn model_generated_terminal_escape_sequences_are_removed() {
        assert_eq!(
            terminal_safe_text("\u{001b}[31mImportant\u{001b}[0m\nNext"),
            "Important\nNext"
        );
    }
}