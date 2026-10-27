// File-diff and new-file cards across languages, widths and both expansion
// states, painted by the real card renderer for review in a native terminal:
// cargo test --locked --offline specimen::diffs -- --ignored --nocapture
use std::time::Duration;

use crate::events::tool_summary;
use crate::json::Value;
use crate::tui::cards::{self, Presentation, Tool};

const CSS_ONE: &str = "*{box-sizing:border-box}html{scroll-behavior:smooth}body{margin:0;background:#f4f1ec;color:#1d1b19;font-family:'Inter',Arial,sans-serif}.page{max-width:1180px;margin:48px auto 0;background:#fff;overflow:hidden}a{color:inherit;text-decoration:none}a:focus-visible{outline:2px solid #b5835a;outline-offset:4px}.hero{height:640px;position:relative}\n";
const CSS_TWO: &str = "@media(max-width:800px){.page{margin:0}.hero{height:560px}.hero h1{top:28%;font-size:18vw}.section{padding:56px 24px}.about-grid{gap:28px}.card-grid{grid-template-columns:2fr 1fr 1fr;gap:12px}.card{height:320px;padding:16px}.footer-top{grid-template-columns:1fr 1fr}}\n";
const CSS_OLD: &str = "@media(max-width:600px){.hero{height:520px}.header{padding:20px 18px}.brand{font-size:28px}.header nav{font-size:11px}.header nav a{padding:6px}.hero h1{top:32%;font-size:20vw}.hero-bottom{left:20px;right:20px}.round-arrow{width:44px;height:44px}.section{padding:48px 18px}.card-grid{grid-template-columns:1fr}}\n";
const CSS_NEW: &str = "@media(max-width:600px){.hero{height:520px}.header{padding:20px 18px}.brand{font-size:28px}.header nav{font-size:11px}.header nav a{padding:6px}.hero h1{top:32%;font-size:20vw}.hero-bottom{left:20px;right:20px}.round-arrow{width:44px;height:44px}.section{padding:48px 18px}.card-grid{grid-template-columns:1fr;gap:8px}}\n";

const SCRIPT: &str = "// Reveal sections on scroll unless the visitor prefers reduced motion.
const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
if ('IntersectionObserver' in window && !reduceMotion.matches) {
  const targets = document.querySelectorAll('.intro > p, .stats > div, .services > h2, .card-grid > a, .projects > h2, .gallery figure, .contact-form, footer');
  const observer = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (!entry.isIntersecting) return;
      entry.target.classList.add('is-visible');
      observer.unobserve(entry.target);
    });
  }, { rootMargin: '0px 0px -10% 0px', threshold: 0.15 });
  targets.forEach((target) => observer.observe(target));
}
";

const HTML: &str = "<!doctype html>
<html lang=\"en\">
<head>
  <meta charset=\"utf-8\">
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
  <title>Studio Marea</title>
  <link rel=\"preconnect\" href=\"https://fonts.googleapis.com\">
  <link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin>
  <link href=\"https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap\" rel=\"stylesheet\">
  <link rel=\"stylesheet\" href=\"style.css\">
</head>
<body>
<main class=\"page\" id=\"home\">
";

const PROJECTS: &str = "    <section class=\"section projects\" id=\"projects\">
      <p class=\"eyebrow\">(04) &nbsp;Selected work</p><h2>Recent homes and buildings<br>along the coast</h2>
      <div class=\"project-grid\">
        <article class=\"image-card project project-one\"><div><div class=\"project-title\"><h3>CASA RIVA</h3><span class=\"tags\"><i>#Interior</i><i>#Architecture</i></span></div><p>Set between the harbour and the pine woods, the Casa Riva renovation joins the old stone rooms to a new timber wing.</p></div></article>
        <article class=\"image-card project project-two\"><div class=\"project-title\"><h3>MOLO</h3><span class=\"tags\"><i>#Public</i><i>#Interior</i></span></div></article>
        <article class=\"image-card project project-three\"><div class=\"project-title\"><h3>PINETA</h3><span class=\"tags\"><i>#Residential</i></span></div></article>
      </div>
    </section>
";

const PYTHON: &str = "from pathlib import Path


def load_settings(path: Path, *, defaults: dict[str, str] | None = None, strict: bool = False) -> dict[str, str]:
    \"\"\"Read key=value pairs, skipping comments and blank lines.\"\"\"
    settings = dict(defaults or {})
    for number, line in enumerate(path.read_text(encoding=\"utf-8\").splitlines(), start=1):
        if not line.strip() or line.lstrip().startswith(\"#\"):
            continue
        key, separator, value = line.partition(\"=\")
        if not separator and strict:
            raise ValueError(f\"{path}:{number}: expected key=value\")
        settings[key.strip()] = value.strip()
    return settings
";

const YAML: &str = "name: CI
on:
  push:
    branches: [main]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Test
        run: cargo test --locked --offline --all-targets -- --test-threads=4 --skip native_terminal_resizes_and_interrupts
";

const MARKDOWN: &str = "# Studio Marea

Studio Marea designs coastal homes and small public buildings along the Adriatic, working with local craftspeople and materials that age well in salt air and strong summer light.

## Contact
";

const RUST: &str = "impl Limiter {
    pub fn check(&mut self, key: &str, now: Instant) -> Result<(), Blocked> {
        let attempts = self.attempts.entry(key.to_owned()).or_default();
        attempts.retain(|attempt| now.duration_since(*attempt) < self.window);
        if attempts.len() >= self.limit {
            return Err(Blocked { retry_after: self.window - now.duration_since(attempts[0]) });
        }
        attempts.push(now);
        Ok(())
    }
}
";

const GO: &str = "package main

import \"net/http\"

func health(w http.ResponseWriter, r *http.Request) {
\t// Report readiness only after the database answers.
\tif err := db.PingContext(r.Context()); err != nil {
\t\thttp.Error(w, \"database unavailable\", http.StatusServiceUnavailable)
\t\treturn
\t}
\tw.WriteHeader(http.StatusOK)
}
";

const JSON: &str = "{\"name\":\"studio-marea\",\"private\":true,\"scripts\":{\"dev\":\"vite --host 127.0.0.1\",\"build\":\"vite build\",\"preview\":\"vite preview --port 4173\"},\"devDependencies\":{\"vite\":\"^5.4.0\"}}\n";

struct Sample {
    path: &'static str,
    before: Option<String>,
    after: String,
}

fn samples() -> Vec<Sample> {
    let edit = |path, before: &str, old: &str, new: &str| Sample {
        path,
        before: Some(before.to_owned()),
        after: before.replacen(old, new, 1),
    };
    vec![
        Sample {
            path: "style.css",
            before: Some(format!("{CSS_ONE}{CSS_TWO}{CSS_OLD}")),
            after: format!("{CSS_ONE}{CSS_TWO}{CSS_NEW}"),
        },
        Sample {
            path: "script.js",
            before: None,
            after: SCRIPT.to_owned(),
        },
        edit(
            "script.js",
            SCRIPT,
            "rootMargin: '0px 0px -10% 0px', threshold: 0.15",
            "rootMargin: '0px 0px -15% 0px', threshold: 0.2",
        ),
        edit(
            "index.html",
            HTML,
            "  <link rel=\"stylesheet\" href=\"style.css\">\n",
            "  <link rel=\"stylesheet\" href=\"style.css\">\n  <script src=\"script.js\" defer></script>\n",
        ),
        edit(
            "projects.html",
            PROJECTS,
            "</div><p>Set between the harbour and the pine woods, the Casa Riva renovation joins the old stone rooms to a new timber wing.</p></div>",
            "</div></div>",
        ),
        edit(
            "settings.py",
            PYTHON,
            "expected key=value\")",
            "expected key=value, found {line!r}\")",
        ),
        edit(
            ".github/workflows/ci.yml",
            YAML,
            "--test-threads=4",
            "--test-threads=2",
        ),
        edit(
            "README.md",
            MARKDOWN,
            "strong summer light.",
            "strong summer light, from first sketch to handover.",
        ),
        edit(
            "src/limiter.rs",
            RUST,
            "self.window - now",
            "self.window.saturating_sub(now",
        ),
        edit(
            "health.go",
            GO,
            "\"database unavailable\"",
            "\"database not ready\"",
        ),
        edit("package.json", JSON, "\"^5.4.0\"", "\"^5.4.8\""),
    ]
}

#[test]
#[ignore = "prints the diff specimen for review"]
fn diffs() {
    let mut painted = String::new();
    for sample in samples() {
        let change = crate::file_change::Change::between(sample.before.as_deref(), &sample.after);
        let arguments = Value::object([
            ("path", Value::string(sample.path)),
            ("content", Value::string(&sample.after)),
        ]);
        let result = Value::object([("bytes_written", Value::number(sample.after.len()))]);
        let summary = tool_summary("write", &result);
        for (columns, expanded) in [60, 80, 100, 120]
            .into_iter()
            .flat_map(|columns| [(columns, false), (columns, true)])
        {
            let presentation = Presentation {
                elapsed: Some(Duration::from_millis(3)),
                expanded,
                ..Presentation::default()
            };
            let tool = Tool {
                name: "write",
                arguments: &arguments,
                summary: &summary,
                result: Some(&result),
                change: change.as_ref(),
                presentation: &presentation,
            };
            painted.push_str(&format!(
                "@@ {} {columns} {}\n",
                sample.path,
                if expanded { "expanded" } else { "collapsed" }
            ));
            for line in cards::card(tool, columns, false).rows {
                painted.push_str(&line.paint(columns));
                painted.push('\n');
            }
        }
    }
    print!("{painted}");
}
