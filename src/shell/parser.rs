use super::*;

pub(super) type Group = (&'static str, Vec<Simple>, bool);

pub(super) fn parse(tokens: &[Token]) -> Result<Vec<Group>, String> {
    let mut groups = Vec::new();
    let mut pipeline = Vec::new();
    let mut simple = Simple::default();
    let mut separator = ";";
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            Token::Word(word) => {
                if let Some(Token::Op(mode @ (">" | ">>" | "<"))) = tokens.get(index + 1)
                    && (word.0 == [Part::Literal("2".into())]
                        || word.0 == [Part::Literal("1".into())])
                {
                    let stream = if word.0 == [Part::Literal("2".into())] {
                        2
                    } else {
                        1
                    };
                    let Some(Token::Word(target)) = tokens.get(index + 2) else {
                        return Err("redirection needs a path".into());
                    };
                    simple.redirects.push(Redirect {
                        stream,
                        mode,
                        target: target.clone(),
                    });
                    index += 3;
                    continue;
                }
                simple.words.push(word.clone());
            }
            Token::Op(mode @ (">" | ">>" | "<")) => {
                let Some(Token::Word(target)) = tokens.get(index + 1) else {
                    return Err("redirection needs a path".into());
                };
                simple.redirects.push(Redirect {
                    stream: 1,
                    mode,
                    target: target.clone(),
                });
                index += 2;
                continue;
            }
            Token::Op(">&2") => simple.redirects.push(Redirect {
                stream: 1,
                mode: ">&2",
                target: Word(Vec::new()),
            }),
            Token::Op("|") => {
                if simple.words.is_empty() {
                    return Err("empty pipeline stage".into());
                }
                pipeline.push(std::mem::take(&mut simple));
            }
            Token::Op(op @ (";" | "&&" | "||" | "&")) => {
                if !simple.words.is_empty() || !simple.redirects.is_empty() {
                    pipeline.push(std::mem::take(&mut simple));
                }
                if !pipeline.is_empty() {
                    groups.push((separator, std::mem::take(&mut pipeline), *op == "&"));
                }
                separator = if *op == "&" { ";" } else { op };
            }
            _ => return Err("unsupported shell operator".into()),
        }
        index += 1;
    }
    if !simple.words.is_empty() || !simple.redirects.is_empty() {
        pipeline.push(simple);
    }
    if matches!(tokens.last(), Some(Token::Op("&&" | "||" | "|"))) {
        return Err("incomplete command list".into());
    }
    if !pipeline.is_empty() {
        groups.push((separator, pipeline, false));
    }
    Ok(groups)
}
