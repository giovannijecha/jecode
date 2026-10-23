#[derive(Clone, Debug)]
pub(super) struct Url {
    pub value: String,
    origin: String,
}

impl Url {
    pub fn parse(input: &str) -> Result<Self, String> {
        let value = input.trim();
        if value.chars().any(char::is_whitespace) || value.chars().any(char::is_control) {
            return Err("URL must not contain whitespace or control characters".into());
        }
        let (scheme, rest) = value
            .split_once("://")
            .ok_or("URL must start with http:// or https://")?;
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return Err("URL must start with http:// or https://".into());
        }
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        if authority.is_empty()
            || authority.contains('@')
            || authority.chars().any(char::is_whitespace)
        {
            return Err("URL must have a host and must not contain user credentials".into());
        }
        let scheme = scheme.to_ascii_lowercase();
        let mut authority = authority.to_ascii_lowercase();
        if let Some((host, port)) = authority.rsplit_once(':')
            && ((scheme == "http" && port == "80") || (scheme == "https" && port == "443"))
        {
            authority = host.to_string();
        }
        let suffix = rest[end..].split('#').next().unwrap_or("");
        let suffix = if suffix.is_empty() {
            "/".to_string()
        } else if suffix.starts_with('?') {
            format!("/{suffix}")
        } else {
            suffix.to_string()
        };
        let (path, query) = suffix
            .split_once('?')
            .map_or((&*suffix, None), |(path, query)| (path, Some(query)));
        let path = normalize_path(path);
        let suffix = query.map_or(path.clone(), |query| format!("{path}?{query}"));
        let origin = format!("{scheme}://{authority}");
        Ok(Self {
            value: format!("{origin}{suffix}"),
            origin,
        })
    }

    pub fn resolve(&self, reference: &str) -> Option<Self> {
        let reference = reference.trim();
        if reference.is_empty()
            || reference.starts_with('#')
            || reference.chars().any(char::is_whitespace)
        {
            return None;
        }
        if reference.split(['/', '?', '#']).next()?.contains(':') {
            return Self::parse(reference).ok();
        }
        let absolute = if reference.starts_with("//") {
            format!("{}:{reference}", self.value.split(':').next()?)
        } else if reference.starts_with('/') {
            format!("{}{reference}", self.origin)
        } else if reference.starts_with('?') {
            format!("{}{reference}", self.value.split('?').next()?)
        } else {
            let path = self.value.split('?').next()?;
            let base = path.rsplit_once('/').map_or(path, |(base, _)| base);
            format!("{base}/{reference}")
        };
        Self::parse(&absolute).ok()
    }

    pub fn same_origin(&self, other: &Self) -> bool {
        self.origin == other.origin
    }
}

fn normalize_path(path: &str) -> String {
    if !path.split('/').any(|part| part == "." || part == "..") {
        return path.to_string();
    }
    let mut parts = Vec::new();
    let mut segments = path.split('/').skip(1).peekable();
    while let Some(part) = segments.next() {
        match part {
            "." => {
                if segments.peek().is_none() {
                    parts.push("");
                }
            }
            ".." => {
                parts.pop();
                if segments.peek().is_none() {
                    parts.push("");
                }
            }
            part => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::Url;

    #[test]
    fn resolves_and_normalizes() {
        let base = Url::parse("HTTP://Example.COM/a/b?x=1#frag").unwrap();
        assert_eq!(base.value, "http://example.com/a/b?x=1");
        assert_eq!(
            base.resolve("../c#n").unwrap().value,
            "http://example.com/c"
        );
        assert_eq!(
            base.resolve("?p=2").unwrap().value,
            "http://example.com/a/b?p=2"
        );
        assert!(base.resolve("mailto:a@b").is_none());
        for (path, expected) in [
            ("/a//b/../c", "/a//c"),
            ("/a/b/.", "/a/b/"),
            ("/a/b/..", "/a/"),
            ("/a///..", "/a//"),
            ("/../../c", "/c"),
        ] {
            assert_eq!(
                Url::parse(&format!("https://example.com{path}"))
                    .unwrap()
                    .value,
                format!("https://example.com{expected}")
            );
        }
    }
}
