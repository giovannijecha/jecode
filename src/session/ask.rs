//! The line-based `ask_user` flow: one question at a time, answered by number
//! or in the user's own words, then sent together.

use crate::ask::{Answer, AnswerDraft, Draft, Observer, Question, Request};
use crate::input::read_line;
use crate::sessions::Handle;
use std::io::{BufRead, Write};

/// Ends with the answers sent, the question dismissed, or, when input
/// closes, the question saved for resume.
pub(super) fn answer(
    questions: &Observer,
    request: &Request,
    session: &Handle,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), String> {
    loop {
        let sent = match collect(request, session, input, output)? {
            Flow::Answered(answers) => questions.answer(&request.id, answers),
            Flow::Dismissed => questions.dismiss(&request.id),
            Flow::Closed => {
                questions.suspend();
                return Ok(());
            }
        };
        match sent {
            Ok(()) => return Ok(()),
            Err(error) => writeln!(output, "{error}").map_err(|error| error.to_string())?,
        }
    }
}

enum Flow {
    Answered(Vec<Answer>),
    Dismissed,
    Closed,
}

fn collect(
    request: &Request,
    session: &Handle,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Flow, String> {
    let mut draft = session
        .snapshot()
        .input
        .question
        .filter(|draft| draft.id == request.id && draft.answers.len() == request.questions.len())
        .unwrap_or_else(|| Draft {
            id: request.id.clone(),
            step: 0,
            answers: vec![AnswerDraft::default(); request.questions.len()],
        });
    let start = draft.step.min(request.questions.len() - 1);
    let mut answers = Vec::new();
    for (index, question) in request.questions.iter().enumerate() {
        if index < start
            && let Some(answer) = restored_answer(question, &draft.answers[index])
        {
            answers.push(answer);
            continue;
        }
        show(output, request, index)?;
        loop {
            write!(output, "{}", prompt(question))
                .and_then(|()| output.flush())
                .map_err(|error| error.to_string())?;
            let Some(line) = read_line(input)? else {
                return Ok(Flow::Closed);
            };
            let line = line.trim_end_matches(['\r', '\n']);
            if line.trim() == "/dismiss" {
                return Ok(Flow::Dismissed);
            }
            match parse(question, line) {
                Ok(answer) => {
                    draft.answers[index] = AnswerDraft {
                        choices: answer.choices.clone(),
                        other: !answer.text.is_empty(),
                        text: answer.text.clone(),
                        caret: answer.text.len(),
                        row: question
                            .options
                            .iter()
                            .position(|option| answer.choices.contains(&option.id))
                            .unwrap_or(question.options.len()),
                    };
                    draft.step = (index + 1).min(request.questions.len() - 1);
                    // Keep the same draft format as the TUI, and make each
                    // accepted answer durable before showing the next question.
                    let mut saved = session.snapshot().input;
                    saved.question = Some(draft.clone());
                    session.input(saved);
                    session.flush()?;
                    answers.push(answer);
                    break;
                }
                Err(error) => writeln!(output, "{error}").map_err(|error| error.to_string())?,
            }
        }
    }
    Ok(Flow::Answered(answers))
}

fn restored_answer(question: &Question, draft: &AnswerDraft) -> Option<Answer> {
    let text = if draft.other || question.options.is_empty() {
        draft.text.clone()
    } else {
        String::new()
    };
    let answer = Answer {
        id: question.id.clone(),
        choices: if !question.is_multiple() && !text.trim().is_empty() {
            Vec::new()
        } else {
            draft.choices.clone()
        },
        text,
    };
    question.check_answer(&answer).ok()?;
    Some(answer)
}

fn show(output: &mut impl Write, request: &Request, index: usize) -> Result<(), String> {
    let question = &request.questions[index];
    let mut text = match request.questions.len() {
        1 => format!("\n{}\n{}\n", question.title, question.question),
        count => format!(
            "\nQuestion {} of {count} · {}\n{}\n",
            index + 1,
            question.title,
            question.question
        ),
    };
    if !question.context.is_empty() {
        text.push_str(&question.context);
        text.push('\n');
    }
    for (number, choice) in question.options.iter().enumerate() {
        let recommended = if choice.recommended {
            " (recommended)"
        } else {
            ""
        };
        text.push_str(&format!("{}. {}{recommended}\n", number + 1, choice.label));
        if !choice.description.is_empty() {
            text.push_str(&format!("   {}\n", choice.description));
        }
    }
    write!(output, "{text}").map_err(|error| error.to_string())
}

fn prompt(question: &Question) -> &'static str {
    if question.options.is_empty() {
        "Answer (/dismiss to skip): "
    } else if question.is_multiple() {
        "Answer (numbers such as 1,3 or your own words; /dismiss to skip): "
    } else {
        "Answer (number or your own words; /dismiss to skip): "
    }
}

/// Listed numbers choose options; anything else is the user's own words.
fn parse(question: &Question, line: &str) -> Result<Answer, String> {
    if line.trim().is_empty() {
        return Err("Type an answer, or /dismiss to skip.".into());
    }
    let numbers = line
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<usize>()
                .ok()
                .filter(|number| (1..=question.options.len()).contains(number))
        })
        .collect::<Option<Vec<_>>>()
        .filter(|numbers| question.is_multiple() || numbers.len() == 1);
    let (choices, text) = match numbers {
        Some(numbers) => (
            question
                .options
                .iter()
                .enumerate()
                .filter(|(index, _)| numbers.contains(&(index + 1)))
                .map(|(_, option)| option.id.clone())
                .collect(),
            String::new(),
        ),
        None => (Vec::new(), line.to_owned()),
    };
    let answer = Answer {
        id: question.id.clone(),
        choices,
        text,
    };
    question.check_answer(&answer)?;
    Ok(answer)
}
