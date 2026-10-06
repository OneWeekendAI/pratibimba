//! Constraint checks a draft runs through before it can be approved: four rows, each
//! pass / warn / fail / n/a with a one-line reason. A `fail` blocks approval in Core, whatever
//! the dashboard shows.
//!
//! - **technical**: the payload is swapped into a live page, so it may not run code: no
//!   `<script>`, no `on*` handler attributes, no `javascript:`/`vbscript:`/`data:text/html`
//!   URLs, no `<iframe>`, `<object>`, `<embed>`, `<base>` or `<meta>`. `<style>` and `<link>`
//!   warn, because they can restyle the whole page.
//! - **safety**: none of the site's forbidden claims appear in the text.
//! - **brand**: no brand file exists before Phase 2.
//! - **grounding**: agent-written drafts only. Every figure in the copy (a number, a
//!   percentage, a price) must appear in the site's facts.md; one that does not is flagged as a
//!   warn, and with no facts.md every agent claim is unchecked, which also warns. The design
//!   names an embedding cross-check; this is the lexical part of it, with no model to run.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    NotApplicable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub reason: String,
}

pub fn blocks(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.status == Status::Fail)
}

/// What the checks need to know about a draft beyond its HTML.
pub struct Context<'a> {
    pub forbidden_claims: &'a [String],
    /// The `author` column: `human`, `agent` or `ai`.
    pub author: &'a str,
    /// The site's facts.md, if one was given.
    pub facts: Option<&'a str>,
}

pub fn run(html: &str, cx: &Context) -> Vec<Check> {
    vec![technical(html), safety(html, cx.forbidden_claims), brand(), grounding(html, cx.author, cx.facts)]
}

const BANNED_TAGS: [&str; 6] = ["script", "iframe", "object", "embed", "base", "meta"];
const WARN_TAGS: [&str; 2] = ["style", "link"];
const BANNED_SCHEMES: [&str; 3] = ["javascript:", "vbscript:", "data:text/html"];

/// The tags of a fragment as (name, attribute names), lowercased. Good enough for a gate that
/// errs towards refusing: it does not need to render HTML, only to find what may not be there.
fn tags(html: &str) -> Vec<(String, Vec<String>)> {
    let lower = html.to_ascii_lowercase();
    let mut out = vec![];
    let mut rest = lower.as_str();
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        let end = rest.find('>').unwrap_or(rest.len());
        let inner = rest[..end].trim_start_matches('/');
        let mut parts = inner.split(|c: char| c.is_whitespace() || c == '/');
        let name = parts.next().unwrap_or("").to_string();
        let attrs = parts
            .filter(|p| !p.is_empty())
            .map(|p| p.split('=').next().unwrap_or("").trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        out.push((name, attrs));
        rest = &rest[end.min(rest.len())..];
    }
    out
}

fn technical(html: &str) -> Check {
    let fail = |reason: String| Check { name: "technical", status: Status::Fail, reason };
    let tags = tags(html);
    if let Some((t, _)) = tags.iter().find(|(n, _)| BANNED_TAGS.contains(&n.as_str())) {
        return fail(format!("contains a <{t}> tag; a variant may not run or embed code"));
    }
    if let Some(a) = tags.iter().flat_map(|(_, a)| a).find(|a| a.starts_with("on") && a.len() > 2) {
        return fail(format!("has an `{a}` event handler attribute"));
    }
    let squashed: String = html.to_ascii_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(s) = BANNED_SCHEMES.iter().find(|s| squashed.contains(*s)) {
        return fail(format!("links to a `{s}` URL"));
    }
    if let Some((t, _)) = tags.iter().find(|(n, _)| WARN_TAGS.contains(&n.as_str())) {
        return Check { name: "technical", status: Status::Warn, reason: format!("has a <{t}> tag, which can restyle the whole page") };
    }
    Check { name: "technical", status: Status::Pass, reason: "no scripts, handlers or embeds".into() }
}

/// The visible text, roughly: tags removed, whitespace collapsed, lowercased.
fn text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn safety(html: &str, forbidden: &[String]) -> Check {
    if forbidden.is_empty() {
        return Check { name: "safety", status: Status::Warn, reason: "this site has no forbidden-claims list yet".into() };
    }
    let t = text(html);
    match forbidden.iter().find(|c| !c.trim().is_empty() && t.contains(&c.trim().to_lowercase())) {
        Some(c) => Check { name: "safety", status: Status::Fail, reason: format!("makes a forbidden claim: \"{c}\"") },
        None => Check { name: "safety", status: Status::Pass, reason: format!("none of {} forbidden claims appear", forbidden.len()) },
    }
}

fn brand() -> Check {
    Check { name: "brand", status: Status::NotApplicable, reason: "no brand file yet (Phase 2)".into() }
}

/// The figures in a text: digit runs with inner `,` or `.`, and a trailing `%` if present.
fn figures(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = vec![];
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let mut f = String::new();
        while i < chars.len()
            && (chars[i].is_ascii_digit() || ((chars[i] == ',' || chars[i] == '.') && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit())))
        {
            f.push(chars[i]);
            i += 1;
        }
        if chars.get(i) == Some(&'%') {
            f.push('%');
        }
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

fn grounding(html: &str, author: &str, facts: Option<&str>) -> Check {
    let check = |status, reason: String| Check { name: "grounding", status, reason };
    if author == "human" {
        return check(Status::NotApplicable, "written by a person; grounding checks apply to agent drafts".into());
    }
    let Some(facts) = facts.filter(|f| !f.trim().is_empty()) else {
        return check(Status::Warn, "this site has no facts.md, so the agent's claims are unchecked".into());
    };
    let known = figures(&text(facts));
    let missing: Vec<String> = figures(&text(html)).into_iter().filter(|f| !known.contains(f)).collect();
    if missing.is_empty() {
        check(Status::Pass, "every figure in the copy appears in facts.md".into())
    } else {
        check(Status::Warn, format!("not in facts.md: {}", missing.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tech(html: &str) -> Check {
        technical(html)
    }

    #[test]
    fn clean_html_passes() {
        let claims = ["guaranteed".to_string()];
        let c = run("<h1>Gift sets</h1><a href=\"/gifts\" class=\"cta\">Shop gifts</a>", &Context { forbidden_claims: &claims, author: "human", facts: None });
        assert!(!blocks(&c), "{c:?}");
        assert_eq!(c[0].status, Status::Pass);
        assert_eq!(c[1].status, Status::Pass);
    }

    #[test]
    fn script_tag_fails() {
        assert_eq!(tech("<p>hi</p><SCRIPT>alert(1)</script>").status, Status::Fail);
    }

    #[test]
    fn event_handler_attribute_fails() {
        assert_eq!(tech("<img src=x onerror=alert(1)>").status, Status::Fail);
    }

    #[test]
    fn javascript_url_fails() {
        assert_eq!(tech("<a href=\"java\nscript:alert(1)\">x</a>").status, Status::Fail);
    }

    #[test]
    fn iframe_fails() {
        assert_eq!(tech("<iframe src=\"https://x\"></iframe>").status, Status::Fail);
    }

    #[test]
    fn style_tag_warns() {
        assert_eq!(tech("<style>body{display:none}</style><p>x</p>").status, Status::Warn);
    }

    #[test]
    fn a_word_starting_with_on_in_text_passes() {
        // "only" and "online" are text, not attributes.
        assert_eq!(tech("<p>only online today</p>").status, Status::Pass);
    }

    #[test]
    fn forbidden_claim_fails() {
        let c = safety("<p>Results <b>Guaranteed</b> or your money back</p>", &["guaranteed".into()]);
        assert_eq!(c.status, Status::Fail, "{c:?}");
    }

    #[test]
    fn figures_are_found_with_their_percent_sign() {
        assert_eq!(figures("ships in 2 days, 20% off, ₹1,249. 30-day returns."), ["2", "20%", "1,249", "30"]);
    }

    #[test]
    fn grounding_compares_figures_not_formatting() {
        let facts = Some("Orders ship in 2 days. Gift sets from ₹1,249.");
        assert_eq!(grounding("<p>Gift sets from <b>₹1,249</b></p>", "agent", facts).status, Status::Pass);
        assert_eq!(grounding("<p>Gift sets from ₹999</p>", "agent", facts).status, Status::Warn);
        // "2" is in the facts, "20%" is not: a percentage is its own figure.
        assert_eq!(grounding("<p>20% off</p>", "agent", facts).status, Status::Warn);
        assert_eq!(grounding("<p>20% off</p>", "human", facts).status, Status::NotApplicable);
    }

    #[test]
    fn missing_claims_list_warns() {
        assert_eq!(safety("<p>x</p>", &[]).status, Status::Warn);
    }
}
