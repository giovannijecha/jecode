//! Small, bounded OAuth scope sets for explicit authorization step-up.

use super::super::types::{Error, ErrorKind};

const MAX_BYTES: usize = 4096;
const MAX_TOKENS: usize = 64;

fn invalid() -> Error {
    Error::new(ErrorKind::Authentication, "Invalid MCP OAuth scope")
}

/// Merge RFC 6749 space-delimited scope tokens, retaining first-seen order.
pub(in crate::mcp) fn union(parts: &[Option<&str>]) -> Result<Option<String>, Error> {
    let mut tokens = Vec::new();
    let mut bytes = 0usize;
    for part in parts.iter().flatten() {
        if part.len() > MAX_BYTES {
            return Err(invalid());
        }
        if !part.bytes().all(|byte| {
            byte == b' '
                || byte == b'!'
                || (b'#'..=b'[').contains(&byte)
                || (b']'..=b'~').contains(&byte)
        }) {
            return Err(invalid());
        }
        for token in part.split(' ').filter(|token| !token.is_empty()) {
            if tokens.contains(&token) {
                continue;
            }
            if tokens.len() == MAX_TOKENS {
                return Err(invalid());
            }
            bytes = bytes.saturating_add(token.len() + usize::from(!tokens.is_empty()));
            if bytes > MAX_BYTES {
                return Err(invalid());
            }
            tokens.push(token);
        }
    }
    Ok((!tokens.is_empty()).then(|| tokens.join(" ")))
}

/// Only a Bearer 403 insufficient_scope challenge can request more access.
pub(in crate::mcp) fn required(error: &Error) -> Option<String> {
    if error.kind != ErrorKind::Authentication || error.status != Some(403) {
        return None;
    }
    error.headers.iter().find_map(|header| {
        let single = std::slice::from_ref(header);
        (super::wire::challenge_param(single, "error").as_deref() == Some("insufficient_scope"))
            .then(|| super::wire::challenge_param(single, "scope"))
            .flatten()
            .and_then(|scope| union(&[Some(&scope)]).ok().flatten())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unions_valid_scope_tokens_and_rejects_unsafe_or_unbounded_input() {
        assert_eq!(
            union(&[Some("read write"), Some("write deploy")]).unwrap(),
            Some("read write deploy".into())
        );
        assert_eq!(union(&[None, Some("")]).unwrap(), None);
        for scope in [
            "read\nwrite",
            "read\twrite",
            "read\"write",
            "read\\write",
            "é",
        ] {
            assert!(union(&[Some(scope)]).is_err());
        }
        assert!(union(&[Some(&"x".repeat(MAX_BYTES + 1))]).is_err());
        assert!(union(&[Some(&vec!["x"; MAX_TOKENS + 1].join(" "))]).is_ok());
        let many = (0..=MAX_TOKENS)
            .map(|index| format!("s{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(union(&[Some(&many)]).is_err());
    }

    #[test]
    fn extracts_only_bearer_insufficient_scope_from_http_403() {
        let mut error = Error::new(ErrorKind::Authentication, "step up");
        error.status = Some(403);
        error.headers = vec![(
            "WWW-Authenticate".into(),
            "Bearer error=\"insufficient_scope\", scope=\"read write\"".into(),
        )];
        assert_eq!(required(&error).as_deref(), Some("read write"));
        error.status = Some(401);
        assert_eq!(required(&error), None);
        error.status = Some(403);
        error.headers[0].1 = "Bearer error=\"invalid_token\", scope=\"read\"".into();
        assert_eq!(required(&error), None);
        error.headers[0].1 = "Basic error=\"insufficient_scope\", scope=\"read\"".into();
        assert_eq!(required(&error), None);
    }
}
