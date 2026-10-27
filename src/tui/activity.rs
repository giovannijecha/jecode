use std::time::{Duration, Instant};

pub struct Activity {
    pub started: Instant,
    pub tools: usize,
    /// Tokens the turn's responses generated, as they reported them.
    pub generated: usize,
    pub stopping: bool,
}
impl Activity {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            tools: 0,
            generated: 0,
            stopping: false,
        }
    }
}

pub fn duration(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{:.1}s", elapsed.as_secs_f64())
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3600, seconds / 60 % 60)
    }
}

/// `↓ 640`, `↓ 1.2k`, `↓ 128k`, `↓ 1.4M`: generated tokens, absent at zero.
pub fn generated(tokens: usize) -> Option<String> {
    let value = match tokens {
        0 => return None,
        1..1000 => tokens.to_string(),
        1000..100_000 => format!("{:.1}k", tokens as f64 / 1000.0),
        100_000..1_000_000 => format!("{}k", tokens / 1000),
        _ => format!("{:.1}M", tokens as f64 / 1_000_000.0),
    };
    Some(format!("↓ {value}"))
}

/// `1 tool`, `4 tools`, absent at zero.
pub fn tools(count: usize) -> Option<String> {
    (count > 0).then(|| format!("{count} {}", if count == 1 { "tool" } else { "tools" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_moves_from_tenths_to_minutes_to_hours() {
        assert_eq!(duration(Duration::from_millis(3200)), "3.2s");
        assert_eq!(duration(Duration::from_secs(64)), "1m 04s");
        assert_eq!(duration(Duration::from_secs(70 * 60 + 52)), "1h 10m");
        assert_eq!(duration(Duration::from_secs(3600 + 5 * 60)), "1h 05m");
    }

    #[test]
    fn generated_tokens_and_tools_read_compactly_and_hide_at_zero() {
        assert_eq!(generated(0), None);
        assert_eq!(generated(640).as_deref(), Some("↓ 640"));
        assert_eq!(generated(1234).as_deref(), Some("↓ 1.2k"));
        assert_eq!(generated(128_400).as_deref(), Some("↓ 128k"));
        assert_eq!(generated(1_420_000).as_deref(), Some("↓ 1.4M"));
        assert_eq!(tools(0), None);
        assert_eq!(tools(1).as_deref(), Some("1 tool"));
        assert_eq!(tools(4).as_deref(), Some("4 tools"));
    }
}
