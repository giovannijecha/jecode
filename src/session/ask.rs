//! The line-based `ask_user` flow: one question at a time, answered by number
//! or in the user's own words, then sent together.

use crate::ask::{Answer, Observer, Question, Request};
use crate::input::read_line;
use std::io::{BufRead, Write};

/// Ends with the answers sent, the question dismissed, or, when input
/// closes, the question saved for resume.
pub(super) fn answer(
    questions: &Observer,
    request: &Request,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), String> {
    loop {
        let sent = match collect(request, input, output)? {
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
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Flow, String> {
    let mut answers = Vec::new();
    for (index, question) in request.questions.iter().enumerate() {
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
                    answers.push(answer);
                    break;
                }
                Err(error) => writeln!(output, "{error}").map_err(|error| error.to_string())?,
            }
        }
    }
    Ok(Flow::Answered(answers))
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
    Ok(Answer {
        id: question.id.clone(),
        choices,
        text,
    })
}
