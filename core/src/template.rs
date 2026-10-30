//! P1: logic-free templates for copy fields (dynamic-content.md §2). A field's string may hold
//! placeholders, `{path | formatter: arg, ...}`, filled in Core from the visitor's history at
//! decide time. The language has no logic: a closed list of paths and a closed list of
//! formatters, nothing else. Conditions live on the variant (`requires`), never in the string.
//!
//! A template is parsed once, at the door (site file, dashboard draft, MCP), into [`Part`]s,
//! and the parsed form is what is stored, so decide never parses. `{{` and `}}` are a literal
//! `{` and `}`. Quoted arguments use single quotes; `\'` is a quote and `\\` a backslash.
//!
//! **Formatter order (fill must follow it, and [`longest`] assumes it):** the value is
//! formatted by `money`, `int`, `percent`, `upper`, `lower` and `truncate` in the order written;
//! then, if the value exists and is not empty, `prefix` and `suffix` wrap it; otherwise the
//! `fallback` (unformatted, unwrapped) stands, or nothing.
//!
//! **Fill ([`fill`], at decide):** a path's value comes from the visitor's history facts
//! ([`value`]); it is missing when there are no facts (cookieless, no consent, history not
//! allowed), when the fact is absent (no orders in the look-back), or when a count is 0 (there is
//! nothing to say about zero orders; `fallback` says it). `<` and `>` are stripped from a value,
//! never escaped; control characters too. The filled string is cut to the field's `max` on a word
//! boundary, ends trimmed: decide never refuses. `money` groups digits with `,` and adds `.00` (the facts carry
//! no currency in P1); `int` rounds; `percent` adds `%`.
//!
//! **Braces.** At the door (site file, dashboard draft, MCP) every string is template language:
//! `{{` and `}}` are literal braces and a lone `{` opens a placeholder, so `Use code {SAVE}` is
//! refused with a hint to write `{{SAVE}}`. A stored payload is never read as template language:
//! there a string is plain text, verbatim, and a template is only ever the parsed object
//! (`{"tpl", "src"}`). So copy stored before migration 0300 (or since, as plain text with a brace)
//! loads and renders as written.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The P1 paths: what a visitor's history can say. Raw lifetime value is never one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    PurchaseCount,
    DaysSinceLastPurchase,
    LtvBucket,
    CartAbandoned,
    LastViewed,
    TopItem,
    /// The top item's name: its id in P1; the catalogue gives names in P2.
    TopItemName,
    TopItemCount,
    LastOrderItems,
    VisitCount,
}

pub const PATHS: [Path; 10] = [
    Path::PurchaseCount,
    Path::DaysSinceLastPurchase,
    Path::LtvBucket,
    Path::CartAbandoned,
    Path::LastViewed,
    Path::TopItem,
    Path::TopItemName,
    Path::TopItemCount,
    Path::LastOrderItems,
    Path::VisitCount,
];

impl Path {
    pub fn as_str(&self) -> &'static str {
        match self {
            Path::PurchaseCount => "visitor.purchase_count",
            Path::DaysSinceLastPurchase => "visitor.days_since_last_purchase",
            Path::LtvBucket => "visitor.ltv_bucket",
            Path::CartAbandoned => "visitor.cart_abandoned",
            Path::LastViewed => "visitor.last_viewed",
            Path::TopItem => "visitor.top_item",
            Path::TopItemName => "visitor.top_item.name",
            Path::TopItemCount => "visitor.top_item_count",
            Path::LastOrderItems => "visitor.last_order_items",
            Path::VisitCount => "visitor.visit_count",
        }
    }

    pub fn parse(s: &str) -> Result<Path, String> {
        if let Some(p) = PATHS.iter().find(|p| p.as_str() == s) {
            return Ok(*p);
        }
        if s.starts_with("catalog.") || s.starts_with("catalog(") || s.starts_with("offer.") {
            return Err(format!("{s:?} is available after the catalogue / offers phase; today a template may use {}", allowed()));
        }
        Err(format!("unknown path {s:?}; a template may use {}", allowed()))
    }

    /// A number: the only paths `money`, `int` and `percent` may format.
    pub fn is_number(&self) -> bool {
        matches!(self, Path::PurchaseCount | Path::DaysSinceLastPurchase | Path::TopItemCount | Path::VisitCount)
    }

    /// The most characters this path's value can have.
    pub fn max_len(&self) -> usize {
        let id = crate::commerce::MAX_ITEM_ID;
        match self {
            Path::TopItem | Path::TopItemName | Path::LastViewed => id,
            // Up to three ids joined with ", ".
            Path::LastOrderItems => MAX_JOINED * id + (MAX_JOINED - 1) * 2,
            Path::PurchaseCount | Path::TopItemCount | Path::VisitCount => 5,
            Path::DaysSinceLastPurchase => 3,
            // none / low / mid / high
            Path::LtvBucket => 4,
            // yes / no
            Path::CartAbandoned => 3,
        }
    }
}

/// How many ids `visitor.last_order_items` joins.
pub const MAX_JOINED: usize = 3;

fn allowed() -> String {
    PATHS.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ")
}

impl Serialize for Path {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Path::parse(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// A formatter, from the closed list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fmt {
    Money,
    Int,
    Percent,
    Upper,
    Lower,
    Truncate(u32),
    Prefix(String),
    Suffix(String),
    Fallback(String),
}

impl Fmt {
    fn same_kind(&self, o: &Fmt) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(o)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Part {
    Lit(String),
    Ref { path: Path, fmt: Vec<Fmt> },
}

/// A parsed template: its source as written, and its parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub src: String,
    pub parts: Vec<Part>,
}

/// The longest truncate argument accepted (a copy field's max is at most 500).
pub const MAX_TRUNCATE: u32 = 500;

/// Whether `s` holds a placeholder: a `{` that is not half of a `{{`.
pub fn is_template(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{' {
            if b.get(i + 1) == Some(&b'{') {
                i += 2;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

/// Text a template may show: no markup characters, no control characters.
fn plain(what: &str, s: &str) -> Result<(), String> {
    if s.contains(['<', '>']) {
        return Err(format!("{what} contains < or >: copy fields are plain text, no HTML"));
    }
    if s.chars().any(char::is_control) {
        return Err(format!("{what} contains a control character or newline: a field is one line"));
    }
    Ok(())
}

pub fn parse(src: &str) -> Result<Template, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut parts = vec![];
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => {
                lit.push('{');
                i += 2;
            }
            '}' if chars.get(i + 1) == Some(&'}') => {
                lit.push('}');
                i += 2;
            }
            '}' => return Err(format!("a lone }} at character {}: write }}}} for a literal brace", i + 1)),
            '{' => {
                let start = i;
                let mut j = i + 1;
                let mut quoted = false;
                while j < chars.len() {
                    match chars[j] {
                        '\\' if quoted => j += 1,
                        '\'' => quoted = !quoted,
                        '}' if !quoted => break,
                        '{' if !quoted => return Err(format!("a placeholder opened at character {} holds another {{", start + 1)),
                        _ => {}
                    }
                    j += 1;
                }
                if j >= chars.len() {
                    return Err(format!("the placeholder opened at character {} is never closed with }}", start + 1));
                }
                if !lit.is_empty() {
                    plain("the template's text", &lit)?;
                    parts.push(Part::Lit(std::mem::take(&mut lit)));
                }
                let inner: String = chars[start + 1..j].iter().collect();
                parts.push(placeholder(&inner)?);
                i = j + 1;
            }
            c => {
                lit.push(c);
                i += 1;
            }
        }
    }
    if !lit.is_empty() {
        plain("the template's text", &lit)?;
        parts.push(Part::Lit(lit));
    }
    Ok(Template { src: src.to_string(), parts })
}

/// One placeholder's inside: `path | fmt: arg, fmt, ...` (`|` or `,` between formatters).
fn placeholder(inner: &str) -> Result<Part, String> {
    let (path_s, rest) = match inner.find('|') {
        Some(k) => (&inner[..k], Some(&inner[k + 1..])),
        None => (inner, None),
    };
    let path = Path::parse(path_s.trim())?;
    let mut fmt: Vec<Fmt> = vec![];
    if let Some(rest) = rest {
        for item in split_args(rest)? {
            let f = formatter(item.trim())?;
            if fmt.iter().any(|g| g.same_kind(&f)) {
                return Err(format!("{{{}}}: a formatter is given twice", inner.trim()));
            }
            if matches!(f, Fmt::Money | Fmt::Int | Fmt::Percent) && !path.is_number() {
                return Err(format!("{{{}}}: money, int and percent format numbers; {} is text", inner.trim(), path.as_str()));
            }
            fmt.push(f);
        }
    }
    Ok(Part::Ref { path, fmt })
}

/// Split formatters on `,` or `|` outside quotes.
fn split_args(s: &str) -> Result<Vec<String>, String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' if quoted => {
                cur.push(c);
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            '\'' => {
                quoted = !quoted;
                cur.push(c);
            }
            ',' | '|' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if quoted {
        return Err("a quoted argument is never closed with '".into());
    }
    out.push(cur);
    if out.iter().any(|a| a.trim().is_empty()) {
        return Err("an empty formatter: write `| name` or `| name: arg`".into());
    }
    Ok(out)
}

fn formatter(s: &str) -> Result<Fmt, String> {
    let (name, arg) = match s.find(':') {
        Some(k) => (s[..k].trim(), Some(s[k + 1..].trim())),
        None => (s, None),
    };
    let text = |arg: Option<&str>| -> Result<String, String> {
        let a = arg.ok_or_else(|| format!("{name} takes a quoted argument, e.g. {name}: 'text'"))?;
        let v = unquote(a).ok_or_else(|| format!("{name}'s argument must be in single quotes, e.g. {name}: 'text'"))?;
        plain(&format!("{name}'s argument"), &v)?;
        Ok(v)
    };
    let bare = |f: Fmt| if arg.is_some() { Err(format!("{name} takes no argument")) } else { Ok(f) };
    match name {
        "money" => bare(Fmt::Money),
        "int" => bare(Fmt::Int),
        "percent" => bare(Fmt::Percent),
        "upper" => bare(Fmt::Upper),
        "lower" => bare(Fmt::Lower),
        "truncate" => match arg.and_then(|a| a.parse::<u32>().ok()) {
            Some(n) if (1..=MAX_TRUNCATE).contains(&n) => Ok(Fmt::Truncate(n)),
            _ => Err(format!("truncate takes a length from 1 to {MAX_TRUNCATE}, e.g. truncate: 20")),
        },
        "prefix" => text(arg).map(Fmt::Prefix),
        "suffix" => text(arg).map(Fmt::Suffix),
        "fallback" => text(arg).map(Fmt::Fallback),
        other => Err(format!(
            "unknown formatter {other:?}; a template may use money, int, percent, upper, lower, truncate, prefix, suffix, fallback"
        )),
    }
}

/// `'text'` with `\'` and `\\` escapes, or `None` when `a` is not one quoted string.
fn unquote(a: &str) -> Option<String> {
    let inner = a.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => out.push(it.next()?),
            '\'' => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

/// One placeholder's worst-case length.
pub fn ref_longest(path: &Path, fmt: &[Fmt]) -> usize {
    let mut v = path.max_len();
    let (mut pre, mut suf, mut fallback) = (0, 0, 0);
    for f in fmt {
        match f {
            // A currency symbol (up to 3), a group separator per three digits, and ".00".
            Fmt::Money => v = v + v / 3 + 3 + 3,
            Fmt::Int => {}
            Fmt::Percent => v += 1,
            // Case mapping can lengthen a character (ß to SS; some to three).
            Fmt::Upper | Fmt::Lower => v *= 3,
            Fmt::Truncate(n) => v = v.min(*n as usize),
            Fmt::Prefix(p) => pre = p.chars().count(),
            Fmt::Suffix(s) => suf = s.chars().count(),
            Fmt::Fallback(s) => fallback = s.chars().count(),
        }
    }
    (pre + v + suf).max(fallback)
}

/// The most characters `t` can render to: every placeholder at its longest value.
pub fn longest(t: &Template) -> usize {
    t.parts
        .iter()
        .map(|p| match p {
            Part::Lit(s) => s.chars().count(),
            Part::Ref { path, fmt } => ref_longest(path, fmt),
        })
        .sum()
}

/// The placeholder that can grow longest, to name in a refusal.
pub fn widest(t: &Template) -> Option<Path> {
    t.parts
        .iter()
        .filter_map(|p| match p {
            Part::Ref { path, fmt } => Some((ref_longest(path, fmt), *path)),
            Part::Lit(_) => None,
        })
        .max_by_key(|(n, _)| *n)
        .map(|(_, p)| p)
}

/// The visitor's value for `path`, or `None` when missing (see the module doc).
pub fn value(path: &Path, f: &crate::history::Facts) -> Option<String> {
    let count = |n: i64| (n > 0).then(|| n.to_string());
    match path {
        Path::PurchaseCount => count(f.purchase_count),
        Path::DaysSinceLastPurchase => f.days_since_last_purchase.map(|d| format!("{}", d.max(0.0) as i64)),
        Path::LtvBucket => crate::history::BUCKETS.get(f.ltv_bucket as usize).map(|b| b.to_string()),
        Path::CartAbandoned => Some(if f.cart_abandoned { "yes" } else { "no" }.to_string()),
        Path::LastViewed => f.touches.first().map(|t| t.0.clone()),
        // The id in P1; the catalogue gives names in P2.
        Path::TopItem | Path::TopItemName => f.top_item.as_ref().map(|t| t.0.clone()),
        Path::TopItemCount => f.top_item.as_ref().and_then(|t| count(t.1)),
        Path::LastOrderItems => (!f.last_order_items.is_empty()).then(|| f.last_order_items.join(", ")),
        Path::VisitCount => count(f.visit_count),
    }
    .map(|v| clean(&v))
    .filter(|v| !v.is_empty())
}

/// A filled value is plain text: markup characters and control characters go, never escaped.
fn clean(v: &str) -> String {
    v.chars().filter(|c| !matches!(c, '<' | '>') && !c.is_control()).collect()
}

/// `s` within `max` characters, cut at the last space inside the limit when there is one (a
/// single long word is cut where it stands), trailing spaces trimmed.
pub fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    let next_is_space = s.chars().nth(max).is_some_and(char::is_whitespace);
    let out = match head.rfind(' ') {
        Some(k) if !next_is_space && k > 0 => head[..k].to_string(),
        _ => head,
    };
    out.trim_end().to_string()
}

fn money(v: &str) -> String {
    let Ok(n) = v.parse::<f64>() else { return v.to_string() };
    let whole = n.trunc().abs() as u64;
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let cents = ((n.abs().fract() * 100.0).round() as u64).min(99);
    format!("{}{grouped}.{cents:02}", if n < 0.0 { "-" } else { "" })
}

/// One placeholder filled: formatted in the order written, then wrapped, or its fallback.
fn fill_ref(path: &Path, fmt: &[Fmt], f: Option<&crate::history::Facts>) -> String {
    let mut v = f.and_then(|f| value(path, f));
    let (mut pre, mut suf, mut fallback) = ("", "", "");
    for x in fmt {
        match x {
            Fmt::Prefix(p) => pre = p,
            Fmt::Suffix(s) => suf = s,
            Fmt::Fallback(s) => fallback = s,
            _ => {}
        }
        let Some(cur) = v.as_mut() else { continue };
        match x {
            Fmt::Money => *cur = money(cur),
            Fmt::Int => *cur = cur.parse::<f64>().map(|n| format!("{}", n.round() as i64)).unwrap_or_else(|_| cur.clone()),
            Fmt::Percent => cur.push('%'),
            Fmt::Upper => *cur = cur.to_uppercase(),
            Fmt::Lower => *cur = cur.to_lowercase(),
            Fmt::Truncate(n) => *cur = cut(cur, *n as usize),
            _ => {}
        }
    }
    match v.filter(|v| !v.is_empty()) {
        Some(v) => format!("{pre}{v}{suf}"),
        None => fallback.to_string(),
    }
}

/// `t` filled from the visitor's facts (`None`: no history may be read), within `max` characters.
pub fn fill(t: &Template, f: Option<&crate::history::Facts>, max: u32) -> String {
    let mut out = String::new();
    for p in &t.parts {
        match p {
            Part::Lit(s) => out.push_str(s),
            Part::Ref { path, fmt } => out.push_str(&fill_ref(path, fmt, f)),
        }
    }
    // A vanished placeholder at either end leaves no dangling space.
    cut(out.trim(), max as usize)
}

/// Whether a visitor lacks any path a variant `requires` (no facts lack every one). A stored
/// name that is not a path counts as missing: the arm closes rather than opens.
pub fn missing(requires: &[String], f: Option<&crate::history::Facts>) -> bool {
    if requires.is_empty() {
        return false;
    }
    let Some(f) = f else { return true };
    requires.iter().any(|r| Path::parse(r).ok().and_then(|p| value(&p, f)).is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asha() -> crate::history::Facts {
        crate::history::Facts {
            days_since_last_purchase: Some(3.0),
            purchase_count: 10,
            ltv_bucket: 2,
            cart_abandoned: false,
            touches: vec![],
            top_item: Some(("rajma-chawal".into(), 9)),
            last_order_items: vec!["dal".into(), "roti".into()],
            visit_count: 12,
        }
    }

    fn ravi() -> crate::history::Facts {
        crate::history::Facts { purchase_count: 0, days_since_last_purchase: None, top_item: None, last_order_items: vec![], visit_count: 1, ..asha() }
    }

    #[test]
    fn fill_uses_value() {
        let t = parse("Your usual {visitor.top_item.name} is ready").unwrap();
        assert_eq!(fill(&t, Some(&asha()), 100), "Your usual rajma-chawal is ready");
    }

    // Mutation: ignore `fallback` (missing fills empty) -> red.
    #[test]
    fn fill_uses_fallback_when_missing() {
        let t = parse("Try our {visitor.top_item | fallback: 'thali'}").unwrap();
        assert_eq!(fill(&t, Some(&ravi()), 100), "Try our thali");
    }

    // Mutation: emit prefix and suffix even when the value is missing -> red.
    #[test]
    fn prefix_suffix_vanish_when_missing() {
        let t = parse("Hello{visitor.top_item | prefix: ', fan of ', suffix: '!'}").unwrap();
        assert_eq!(fill(&t, Some(&ravi()), 100), "Hello");
    }

    #[test]
    fn prefix_suffix_wrap_a_present_value() {
        let t = parse("Hello{visitor.top_item | prefix: ', fan of ', suffix: '!'}").unwrap();
        assert_eq!(fill(&t, Some(&asha()), 100), "Hello, fan of rajma-chawal!");
    }

    // Mutation: cut at `max` characters mid-word -> red.
    #[test]
    fn fill_truncates_on_word_boundary() {
        let t = parse("Your usual {visitor.top_item} is ready").unwrap();
        // "Your usual rajma-chawal is ready" is 32; at 25 the cut falls inside "is".
        assert_eq!(fill(&t, Some(&asha()), 25), "Your usual rajma-chawal");
    }

    // Mutation: drop the stripping in `clean` -> red.
    #[test]
    fn fill_strips_angle_brackets_from_item_name() {
        let mut f = asha();
        f.top_item = Some(("<img src=x onerror=alert(1)>".repeat(5), 3));
        let t = parse("Usual {visitor.top_item}").unwrap();
        let out = fill(&t, Some(&f), 40);
        assert!(!out.contains(['<', '>']) && out.chars().count() <= 40, "{out}");
    }

    #[test]
    fn a_zero_count_is_missing() {
        let t = parse("{visitor.purchase_count | suffix: ' orders', fallback: 'Welcome'}").unwrap();
        assert_eq!(fill(&t, Some(&ravi()), 100), "Welcome");
    }

    #[test]
    fn formatters_apply_in_order() {
        let t = parse("{visitor.top_item | truncate: 5, upper} {visitor.visit_count | money} {visitor.last_order_items}").unwrap();
        assert_eq!(fill(&t, Some(&asha()), 100), "RAJMA 12.00 dal, roti");
    }

    #[test]
    fn money_groups_thousands() {
        assert_eq!(money("1234567"), "1,234,567.00");
    }

    // Mutation: return false for absent facts -> red.
    #[test]
    fn missing_true_without_facts() {
        assert!(missing(&["visitor.top_item".into()], None));
    }

    #[test]
    fn missing_false_when_present() {
        assert!(!missing(&["visitor.top_item".into(), "visitor.visit_count".into()], Some(&asha())));
    }

    #[test]
    fn missing_true_when_one_fact_is_absent() {
        assert!(missing(&["visitor.visit_count".into(), "visitor.top_item".into()], Some(&ravi())));
    }

    fn lit(s: &str) -> Part {
        Part::Lit(s.into())
    }

    #[test]
    fn parses_literal_only() {
        assert_eq!(parse("Home food").unwrap().parts, vec![lit("Home food")]);
    }

    #[test]
    fn parses_ref_with_fallback() {
        let t = parse("Your usual {visitor.top_item.name | fallback: 'thali'} is ready").unwrap();
        assert_eq!(
            t.parts,
            vec![
                lit("Your usual "),
                Part::Ref { path: Path::TopItemName, fmt: vec![Fmt::Fallback("thali".into())] },
                lit(" is ready")
            ]
        );
    }

    // Mutation: delete the formatter whitelist match arm (unknown names accepted) -> red.
    #[test]
    fn refuses_unknown_formatter() {
        let e = parse("{visitor.top_item | reverse}").unwrap_err();
        assert!(e.contains("reverse"), "{e}");
    }

    #[test]
    fn refuses_unknown_path() {
        let e = parse("{visitor.ltv}").unwrap_err();
        assert!(e.contains("visitor.ltv") && e.contains("visitor.top_item_count"), "{e}");
    }

    // Mutation: drop the catalog branch in Path::parse -> red.
    #[test]
    fn refuses_catalog_in_p1() {
        let e = parse("{catalog.item(x).name}").unwrap_err();
        assert!(e.contains("catalogue"), "{e}");
    }

    #[test]
    fn refuses_unclosed_brace() {
        assert!(parse("Order {visitor.top_item").is_err());
    }

    #[test]
    fn double_brace_is_literal() {
        assert_eq!(parse("a {{b}}").unwrap().parts, vec![lit("a {b}")]);
    }

    #[test]
    fn refuses_markup_in_literal() {
        let e = parse("<b>{visitor.top_item}</b>").unwrap_err();
        assert!(e.contains("no HTML"), "{e}");
    }

    // Mutation: drop the is_number check in placeholder -> red.
    #[test]
    fn refuses_number_formatter_on_text_path() {
        let e = parse("{visitor.top_item | money}").unwrap_err();
        assert!(e.contains("format numbers"), "{e}");
    }

    #[test]
    fn parses_several_formatters_and_escaped_quote() {
        let t = parse(r"{visitor.visit_count | prefix: 'it\'s visit ', suffix: '!'}").unwrap();
        assert_eq!(t.parts, vec![Part::Ref { path: Path::VisitCount, fmt: vec![Fmt::Prefix("it's visit ".into()), Fmt::Suffix("!".into())] }]);
    }

    #[test]
    fn is_template_ignores_doubled_braces() {
        assert!(!is_template("a {{b}}"));
        assert!(is_template("a {visitor.top_item}"));
    }

    #[test]
    fn longest_counts_truncate_and_fallback() {
        let t = parse("Hi {visitor.top_item | truncate: 10, fallback: 'friend of ours'}").unwrap();
        assert_eq!(longest(&t), 3 + 14);
    }
}
