//! Heuristic static code map (D87): "skeleton first" outlines for the
//! Trail-Code-style analysis method (D86).
//!
//! A file's outline is its declarations without bodies — type/method/field
//! signatures with the annotations that matter, a one-line doc summary and the
//! line range — so an agent reads ~1–2k characters instead of the whole file
//! and then reads only the ranges it needs. Zero new crates: hand-rolled,
//! byte-level scanners tuned for the priority stack of the SDS legacy estate:
//!
//! - **Java** (Spring, eGovFrame, Struts): a comment/string/text-block aware
//!   lexer + a brace-frame scanner that parses headers only at top level or
//!   directly inside a type, so method bodies (lambdas, anonymous/local
//!   classes) are just brace-counted and can't confuse it.
//! - **XML**: MyBatis 3 mappers / iBATIS 2 sqlMaps (statement ids, parameter/
//!   result types, `include`s, and the tables each statement touches with
//!   C/R/U/D), `web.xml` / `struts-config.xml` entry maps, Spring bean files.
//! - **SQL / PL-SQL**: CREATE TABLE (columns, PK, `COMMENT ON`), VIEW, INDEX,
//!   SEQUENCE, PROCEDURE/FUNCTION, PACKAGE [BODY] members (+ tables per
//!   member), TRIGGER; plain DML scripts get a table summary.
//!
//! Everything is best-effort and never panics: unbalanced input sets
//! `degraded`, non-UTF-8 (EUC-KR) text only loses doc/comment text (the
//! identifiers are ASCII). These are pure functions over bytes — the FabriX
//! tool adapter (fabrix_tools.rs) owns sandboxing and size limits.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::files::{is_binary, is_secret_name, walk_files_with, WalkStop, WALK_MAX};

/// Files larger than this are not outlined (callers should search instead).
pub const OUTLINE_FILE_MAX: u64 = 1024 * 1024;
const MAX_ITEMS: usize = 300;
const SIG_MAX: usize = 200;
const DOC_MAX: usize = 100;
const ANN_MAX: usize = 100;
const FIELDS_PER_TYPE: usize = 25;
const ENUM_CONSTS_MAX: usize = 20;
const COLUMNS_MAX: usize = 40;
const TABLES_SHOWN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    Java,
    Xml,
    Sql,
    Markdown,
    #[default]
    Other,
}

impl Lang {
    pub fn label(self) -> &'static str {
        match self {
            Lang::Java => "Java",
            Lang::Xml => "XML",
            Lang::Sql => "SQL",
            Lang::Markdown => "Markdown",
            Lang::Other => "텍스트",
        }
    }
}

/// The outline language for a file name (by extension).
pub fn lang_for(path: &Path) -> Lang {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "java" => Lang::Java,
        "xml" => Lang::Xml,
        "sql" | "pks" | "pkb" | "pck" | "prc" | "fnc" | "trg" | "vw" | "tab" | "ddl" => Lang::Sql,
        "md" | "markdown" => Lang::Markdown,
        _ => Lang::Other,
    }
}

/// One declaration in an outline.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineItem {
    /// 1-based start line.
    pub line: usize,
    /// 1-based end line (closing brace / tag / `END name;`), when known.
    pub end_line: Option<usize>,
    /// Nesting depth (members of a class are depth 1).
    pub depth: usize,
    /// `class` / `interface` / `enum` / `record` / `annotation` / `method` /
    /// `ctor` / `field` / `const` / `statement` / `table` / `column` / … /
    /// `more` (an elision line).
    pub kind: &'static str,
    pub signature: String,
    pub doc: Option<String>,
    /// An entry point (controller, servlet, Struts action, scheduled job,
    /// `main`, URL mapping…).
    pub entry: bool,
    /// Tables this item touches with C/R/U/D (mapper statements, SQL routines,
    /// views) — the structured form of the `테이블:` doc text.
    pub tables: Vec<(String, String)>,
}

impl OutlineItem {
    fn new(line: usize, depth: usize, kind: &'static str, signature: String) -> Self {
        Self { line, end_line: None, depth, kind, signature, doc: None, entry: false, tables: Vec::new() }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Outline {
    pub lang: Lang,
    /// File-level lines shown above the items (package, mapper namespace,
    /// CRUD summary…).
    pub notes: Vec<String>,
    pub items: Vec<OutlineItem>,
    /// Java `import` targets (input for hub ranking).
    pub imports: Vec<String>,
    /// Unbalanced/unterminated input — the outline may miss items.
    pub degraded: bool,
    /// Line-pattern fallback for an unsupported language.
    pub approximate: bool,
    /// The item cap was hit.
    pub truncated: bool,
}

impl Outline {
    fn with_lang(lang: Lang) -> Self {
        Self { lang, ..Default::default() }
    }

    fn push(&mut self, item: OutlineItem) -> Option<usize> {
        if self.items.len() >= MAX_ITEMS {
            self.truncated = true;
            return None;
        }
        self.items.push(item);
        Some(self.items.len() - 1)
    }

    fn set_end(&mut self, idx: Option<usize>, line: usize) {
        if let Some(it) = idx.and_then(|i| self.items.get_mut(i)) {
            it.end_line = Some(line);
        }
    }
}

/// Outline a file's bytes.
pub fn outline(lang: Lang, src: &[u8]) -> Outline {
    match lang {
        Lang::Java => outline_java(src),
        Lang::Xml => outline_xml(src),
        Lang::Sql => outline_sql(src),
        Lang::Markdown => outline_markdown(src),
        Lang::Other => outline_generic(src),
    }
}

/// Render an outline for an agent: a header, the file-level notes, one line
/// per item (`L12-40 signature — doc`, indented by depth, `▶` = entry point)
/// and a hint to read bodies by line range. Cut at `max_chars`.
pub fn render_outline(o: &Outline, path_label: &str, max_chars: usize) -> String {
    let mut flags: Vec<String> = Vec::new();
    if o.approximate {
        flags.push("근사 — 지원 언어가 아니라 선언처럼 보이는 줄만 표시".into());
    }
    if o.degraded {
        flags.push("구문이 불완전해 일부 항목이 빠졌을 수 있음".into());
    }
    if o.truncated {
        flags.push(format!("항목 상한 {MAX_ITEMS}개"));
    }
    let mut out = format!(
        "{path_label} — {} 아웃라인(본문 제외): 항목 {}개{}\n",
        o.lang.label(),
        o.items.len(),
        if flags.is_empty() { String::new() } else { format!(" ({})", flags.join(", ")) }
    );
    for n in &o.notes {
        out.push_str(n);
        out.push('\n');
    }
    if !o.imports.is_empty() {
        out.push_str(&format!("import {}개\n", o.imports.len()));
    }
    if o.items.iter().any(|i| i.entry) {
        out.push_str("(▶ = 진입점: 컨트롤러·URL 매핑·서블릿·배치·main 등)\n");
    }
    let footer = "본문이 필요하면 read_file(path, start_line=<시작 줄>, max_lines=<범위>)로 해당 범위만 읽으세요.\n";
    let mut used = out.chars().count();
    let budget = max_chars.saturating_sub(footer.chars().count() + 80);
    let mut shown = 0usize;
    for it in &o.items {
        let row = render_item(it);
        let n = row.chars().count();
        if used + n > budget {
            break;
        }
        out.push_str(&row);
        used += n;
        shown += 1;
    }
    if shown < o.items.len() {
        out.push_str(&format!("…(출력 상한 — 항목 {}개 중 {shown}개 표시)\n", o.items.len()));
    }
    if !o.items.is_empty() {
        out.push_str(footer);
    }
    out
}

fn render_item(it: &OutlineItem) -> String {
    let indent = "  ".repeat(it.depth.min(6));
    if it.kind == "more" {
        return format!("{indent}{}\n", it.signature);
    }
    let range = match it.end_line {
        Some(e) if e > it.line => format!("L{}-{}", it.line, e),
        _ => format!("L{}", it.line),
    };
    let mark = if it.entry { "▶ " } else { "" };
    let doc = it.doc.as_ref().map(|d| format!(" — {d}")).unwrap_or_default();
    format!("{indent}{mark}{range} {}{doc}\n", it.signature)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Line-start index for byte offset → 1-based line lookups.
struct Lines(Vec<usize>);

impl Lines {
    fn new(src: &[u8]) -> Self {
        let mut starts = vec![0];
        for (i, b) in src.iter().enumerate() {
            if *b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self(starts)
    }

    fn of(&self, pos: usize) -> usize {
        self.0.partition_point(|&s| s <= pos).max(1)
    }
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let t: String = s.chars().take(max).collect();
    format!("{t}…")
}

fn find_bytes(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() || hay.len() - from < needle.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Advance `i` to the next char boundary of `s` (or its end).
fn boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
}

/// "ORDERS(R), CUSTOMER(RU)" — capped.
fn format_tables(tables: &[(String, String)]) -> String {
    let mut parts: Vec<String> =
        tables.iter().take(TABLES_SHOWN).map(|(t, c)| format!("{t}({c})")).collect();
    if tables.len() > TABLES_SHOWN {
        parts.push(format!("외 {}개", tables.len() - TABLES_SHOWN));
    }
    parts.join(", ")
}

/// Merge `(table, crud)` pairs into `acc`, keeping first-seen order and the
/// CRUD letters in C-R-U-D order.
fn merge_tables(acc: &mut Vec<(String, String)>, more: &[(String, String)]) {
    for (t, c) in more {
        if let Some((_, have)) = acc.iter_mut().find(|(n, _)| n == t) {
            for ch in c.chars() {
                if !have.contains(ch) {
                    have.push(ch);
                }
            }
            *have = order_crud(have);
        } else {
            acc.push((t.clone(), order_crud(c)));
        }
    }
}

fn order_crud(s: &str) -> String {
    "CRUD".chars().filter(|c| s.contains(*c)).collect()
}

// ---------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------

const CODE: u8 = 0;
const COMMENT: u8 = 1;
const STR: u8 = 2;

fn fill(mask: &mut [u8], from: usize, to: usize, class: u8) {
    let to = to.min(mask.len());
    if from < to {
        mask[from..to].fill(class);
    }
}

/// Per-byte class mask (code/comment/string) + the javadoc comment spans.
fn lex_java(src: &[u8]) -> (Vec<u8>, Vec<(usize, usize)>) {
    let n = src.len();
    let mut mask = vec![CODE; n];
    let mut docs = Vec::new();
    let mut i = 0;
    while i < n {
        let c = src[i];
        if c == b'/' && src.get(i + 1) == Some(&b'/') {
            let end = src[i..].iter().position(|&b| b == b'\n').map(|p| i + p).unwrap_or(n);
            fill(&mut mask, i, end, COMMENT);
            i = end;
            continue;
        }
        if c == b'/' && src.get(i + 1) == Some(&b'*') {
            let end = find_bytes(src, i + 2, b"*/").map(|p| p + 2).unwrap_or(n);
            if src.get(i + 2) == Some(&b'*') && src.get(i + 3) != Some(&b'/') {
                docs.push((i, end));
            }
            fill(&mut mask, i, end, COMMENT);
            i = end;
            continue;
        }
        if c == b'"' && src[i..].starts_with(b"\"\"\"") {
            let mut j = i + 3;
            let mut end = n;
            while j < n {
                if src[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if src[j..].starts_with(b"\"\"\"") {
                    end = j + 3;
                    break;
                }
                j += 1;
            }
            fill(&mut mask, i, end, STR);
            i = end.min(n);
            continue;
        }
        if c == b'"' {
            let mut j = i + 1;
            let mut end = n;
            while j < n {
                match src[j] {
                    b'\\' => {
                        j += 2;
                        continue;
                    }
                    b'"' => {
                        end = j + 1;
                        break;
                    }
                    // An unterminated string ends at the line end (recovery).
                    b'\n' => {
                        end = j;
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            fill(&mut mask, i, end, STR);
            i = end.min(n).max(i + 1);
            continue;
        }
        if c == b'\'' {
            let mut j = i + 1;
            let mut end = None;
            while j < n && j <= i + 10 {
                match src[j] {
                    b'\\' => {
                        j += 2;
                        continue;
                    }
                    b'\'' => {
                        end = Some(j + 1);
                        break;
                    }
                    b'\n' => break,
                    _ => {}
                }
                j += 1;
            }
            if let Some(e) = end {
                fill(&mut mask, i, e, STR);
                i = e.min(n);
                continue;
            }
        }
        i += 1;
    }
    (mask, docs)
}

/// `src[from..to]` with comments blanked (strings kept), whitespace collapsed.
fn header_text(src: &[u8], mask: &[u8], from: usize, to: usize) -> String {
    let to = to.min(src.len());
    if from >= to {
        return String::new();
    }
    let bytes: Vec<u8> = (from..to).map(|i| if mask[i] == COMMENT { b' ' } else { src[i] }).collect();
    collapse_ws(&String::from_utf8_lossy(&bytes))
}

/// The first javadoc sentence (≤100 chars). `None` for non-UTF-8 bytes.
fn doc_summary(raw: &[u8]) -> Option<String> {
    let raw = std::str::from_utf8(raw).ok()?;
    let body = raw.trim_start_matches("/**").trim_end_matches("*/");
    let mut text = String::new();
    for line in body.lines() {
        let l = line.trim().trim_start_matches('*').trim();
        if l.starts_with('@') {
            break;
        }
        if l.is_empty() {
            if text.is_empty() {
                continue;
            }
            break;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(l);
    }
    // Drop simple HTML tags (<p>, <br/>, <code>…).
    let mut plain = String::new();
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => plain.push(ch),
            _ => {}
        }
    }
    let plain = collapse_ws(&plain);
    let first = match plain.find(". ") {
        Some(p) => plain[..p].to_string(),
        None => plain.trim_end_matches('.').to_string(),
    };
    let first = first.trim();
    if first.is_empty() {
        None
    } else {
        Some(trunc(first, DOC_MAX))
    }
}

/// Annotations worth showing (Spring/eGov/JPA/Servlet roles, injection,
/// transactions, scheduling) — every other annotation is dropped.
const KEEP_ANN: &[&str] = &[
    "RestController", "Controller", "Service", "Repository", "Component", "Configuration",
    "Mapper", "SpringBootApplication", "RequestMapping", "GetMapping", "PostMapping", "PutMapping",
    "DeleteMapping", "PatchMapping", "Transactional", "Entity", "Table", "FeignClient", "Scheduled",
    "Bean", "EventListener", "KafkaListener", "JmsListener", "Async", "PreAuthorize", "Autowired",
    "Resource", "Value", "Inject", "Column", "Id", "WebServlet", "WebFilter",
];
/// Parameter annotations kept inside a signature (bindings that matter).
const PARAM_KEEP: &[&str] = &[
    "PathVariable", "RequestParam", "RequestBody", "ModelAttribute", "RequestHeader", "Param",
];
const MODIFIERS: &[&str] = &[
    "public", "protected", "private", "static", "final", "abstract", "native", "synchronized",
    "transient", "volatile", "strictfp", "default", "sealed", "non-sealed",
];

/// Split leading annotations off a header: (kept annotation texts, rest).
fn peel_annotations(s: &str) -> (Vec<String>, String) {
    let b = s.as_bytes();
    let mut i = 0;
    let mut anns = Vec::new();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'@' && !s[i..].starts_with("@interface") {
            let start = i;
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'.' | b'$')) {
                i += 1;
            }
            let name_end = i;
            let mut j = i;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if j < b.len() && b[j] == b'(' {
                let mut depth = 0i32;
                let mut k = j;
                let mut quote: Option<u8> = None;
                while k < b.len() {
                    let c = b[k];
                    if let Some(q) = quote {
                        if c == b'\\' {
                            k += 2;
                            continue;
                        }
                        if c == q {
                            quote = None;
                        }
                    } else {
                        match c {
                            b'"' | b'\'' => quote = Some(c),
                            b'(' => depth += 1,
                            b')' => {
                                depth -= 1;
                                if depth == 0 {
                                    k += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    k += 1;
                }
                i = boundary(s, k.min(b.len()));
            }
            let simple = s[start + 1..name_end].rsplit('.').next().unwrap_or("");
            if KEEP_ANN.contains(&simple) {
                anns.push(trunc(&s[start..i], ANN_MAX));
            }
            continue;
        }
        break;
    }
    (anns, s[boundary(s, i)..].to_string())
}

/// Strip leading modifiers off `s` (returns the remaining core).
fn strip_modifiers(s: &str) -> &str {
    let mut core = s.trim_start();
    loop {
        let end = core.find(char::is_whitespace).unwrap_or(core.len());
        if MODIFIERS.contains(&&core[..end]) {
            core = core[end..].trim_start();
            if core.is_empty() {
                return core;
            }
        } else {
            return core;
        }
    }
}

/// Byte index of the parameter list's `(`: the first `(` outside type-argument
/// brackets, before any `=` (a field initializer).
fn find_call_paren(core: &str) -> Option<usize> {
    let mut angle = 0i32;
    for (i, c) in core.bytes().enumerate() {
        match c {
            b'<' => angle += 1,
            b'>' => angle = (angle - 1).max(0),
            b'(' if angle == 0 => return Some(i),
            b'=' if angle == 0 => return None,
            _ => {}
        }
    }
    None
}

/// `<K, V extends List<K>> Map<K,V>` → `Map<K,V>`.
fn strip_type_params(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with('<') {
        return s;
    }
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return s[i + 1..].trim();
                }
            }
            _ => {}
        }
    }
    ""
}

/// Inside the parameter list, drop annotations not in `PARAM_KEEP` (`@Valid`,
/// `@NotNull`…) and `final`.
fn strip_param_annotations(s: &str) -> String {
    let Some(p) = s.find('(') else { return s.to_string() };
    let (head, tail) = s.split_at(p);
    let chars: Vec<char> = tail.chars().collect();
    let mut out = String::from(head);
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let start = i;
            let mut j = i + 1;
            while j < chars.len() && (is_ident_char(chars[j]) || chars[j] == '.') {
                j += 1;
            }
            let name: String = chars[start + 1..j].iter().collect();
            let simple = name.rsplit('.').next().unwrap_or("").to_string();
            let mut k = j;
            let mut kk = j;
            while kk < chars.len() && chars[kk] == ' ' {
                kk += 1;
            }
            if kk < chars.len() && chars[kk] == '(' {
                let mut d = 0i32;
                while kk < chars.len() {
                    if chars[kk] == '(' {
                        d += 1;
                    } else if chars[kk] == ')' {
                        d -= 1;
                        if d == 0 {
                            kk += 1;
                            break;
                        }
                    }
                    kk += 1;
                }
                k = kk;
            }
            if PARAM_KEEP.contains(&simple.as_str()) {
                out.extend(&chars[start..k]);
            } else {
                while k < chars.len() && chars[k] == ' ' {
                    k += 1;
                }
            }
            i = k;
            continue;
        }
        if chars[i..].starts_with(&['f', 'i', 'n', 'a', 'l', ' ']) && (i == 0 || !is_ident_char(chars[i - 1])) {
            i += 6;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    collapse_ws(&out)
}

fn join_display(anns: &[String], rest: &str) -> String {
    if anns.is_empty() {
        rest.trim().to_string()
    } else {
        format!("{} {}", anns.join(" "), rest.trim())
    }
}

enum Header {
    Empty,
    Package(String),
    Import(String),
    Type { kind: &'static str, name: String, sig: String, is_enum: bool },
    Callable { sig: String, ctor: bool },
    Field { sig: String },
    Initializer,
}

fn classify_header(h: &str, enclosing: Option<&str>) -> Header {
    if h.is_empty() {
        return Header::Empty;
    }
    if let Some(p) = h.strip_prefix("package ") {
        return Header::Package(p.trim().to_string());
    }
    if let Some(p) = h.strip_prefix("import ") {
        return Header::Import(p.trim().to_string());
    }
    let (anns, rest) = peel_annotations(h);
    let core = strip_modifiers(&rest).trim();
    if core.is_empty() {
        return Header::Initializer;
    }
    let first = core.split_whitespace().next().unwrap_or("");
    let type_kind = match first {
        "class" => Some("class"),
        "interface" => Some("interface"),
        "enum" => Some("enum"),
        "record" => Some("record"),
        "@interface" => Some("annotation"),
        _ => None,
    };
    if let Some(kind) = type_kind {
        let after = core[first.len()..].trim_start();
        let name: String = after.chars().take_while(|c| is_ident_char(*c)).collect();
        return Header::Type {
            kind,
            name,
            sig: trunc(&join_display(&anns, &rest), SIG_MAX),
            is_enum: kind == "enum",
        };
    }
    if let Some(p) = find_call_paren(core) {
        let before = core[..p].trim_end();
        let name_start = before
            .char_indices()
            .rev()
            .find(|(_, c)| !is_ident_char(*c))
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        let name = &before[name_start..];
        if !name.is_empty() {
            let ret = strip_type_params(&before[..name_start]);
            return Header::Callable {
                sig: trunc(&join_display(&anns, &strip_param_annotations(&rest)), SIG_MAX),
                ctor: ret.is_empty(),
            };
        }
    }
    if Some(core) == enclosing {
        // A record's compact canonical constructor: `Point {`.
        return Header::Callable { sig: trunc(&join_display(&anns, &rest), SIG_MAX), ctor: true };
    }
    // Cut the initializer off the declaration (not the annotations' `name = …`).
    let decl = rest.split('=').next().unwrap_or(&rest);
    Header::Field { sig: trunc(&join_display(&anns, decl), SIG_MAX) }
}

fn is_entry_type(sig: &str) -> bool {
    const PATS: &[&str] = &[
        "@RestController", "@Controller", "@SpringBootApplication", "@WebServlet",
        "extends HttpServlet", "extends Action", "extends DispatchAction", "extends MappingDispatchAction",
        "extends LookupDispatchAction", "MultiActionController", "extends AbstractController",
        "SimpleFormController", "implements Controller",
    ];
    PATS.iter().any(|p| sig.contains(p))
}

fn is_entry_member(sig: &str) -> bool {
    const PATS: &[&str] = &[
        "@GetMapping", "@PostMapping", "@PutMapping", "@DeleteMapping", "@PatchMapping",
        "@RequestMapping", "@Scheduled", "@KafkaListener", "@JmsListener", "@EventListener",
        "static void main(", "ActionMapping",
    ];
    PATS.iter().any(|p| sig.contains(p))
}

enum Frame {
    Type { item: Option<usize>, name: String, is_enum: bool, consts_open: bool, fields: usize, consts: usize },
    Member { item: Option<usize> },
    /// An expression/body block; `keep_stmt` = it sits inside a header-level
    /// statement (array/anonymous-class initializer, annotation array, lambda
    /// argument) that continues after the block closes.
    Expr { keep_stmt: bool },
}

struct JavaScan<'a> {
    src: &'a [u8],
    mask: Vec<u8>,
    docs: Vec<(usize, usize)>,
    lines: Lines,
    o: Outline,
    frames: Vec<Frame>,
    paren: i32,
    stmt_start: usize,
}

impl<'a> JavaScan<'a> {
    fn header_level(&self) -> bool {
        matches!(self.frames.last(), None | Some(Frame::Type { .. }))
    }

    fn type_depth(&self) -> usize {
        self.frames.iter().filter(|f| matches!(f, Frame::Type { .. })).count()
    }

    fn enclosing(&self) -> Option<String> {
        self.frames.iter().rev().find_map(|f| match f {
            Frame::Type { name, .. } => Some(name.clone()),
            _ => None,
        })
    }

    fn line_at(&self, from: usize, to: usize) -> usize {
        let to = to.min(self.src.len());
        let first = (from..to).find(|&i| self.mask[i] == CODE && !self.src[i].is_ascii_whitespace());
        self.lines.of(first.unwrap_or(to.saturating_sub(1).max(from)))
    }

    fn doc_for(&self, from: usize, to: usize) -> Option<String> {
        self.docs
            .iter()
            .rev()
            .find(|(s, e)| *s >= from && *e <= to)
            .and_then(|(s, e)| doc_summary(&self.src[*s..*e]))
    }

    fn enum_consts_open(&self) -> bool {
        matches!(self.frames.last(), Some(Frame::Type { is_enum: true, consts_open: true, .. }))
    }

    fn enum_const(&mut self, header: &str, line: usize) {
        let (_, rest) = peel_annotations(header);
        let name: String = rest.trim().chars().take_while(|c| is_ident_char(*c)).collect();
        if name.is_empty() {
            return;
        }
        let depth = self.type_depth();
        let over = match self.frames.last_mut() {
            Some(Frame::Type { consts, .. }) => {
                *consts += 1;
                *consts > ENUM_CONSTS_MAX
            }
            _ => false,
        };
        if !over {
            self.o.push(OutlineItem::new(line, depth, "const", name));
        }
    }

    fn push_member(&mut self, kind: &'static str, sig: String, from: usize, to: usize) -> Option<usize> {
        let mut it = OutlineItem::new(self.line_at(from, to), self.type_depth(), kind, sig);
        it.doc = self.doc_for(from, to);
        it.entry = is_entry_member(&it.signature);
        self.o.push(it)
    }

    fn push_field(&mut self, sig: String, from: usize, to: usize) {
        let over = match self.frames.iter_mut().rev().find(|f| matches!(f, Frame::Type { .. })) {
            Some(Frame::Type { fields, .. }) => {
                *fields += 1;
                *fields > FIELDS_PER_TYPE
            }
            _ => false,
        };
        if !over {
            let mut it = OutlineItem::new(self.line_at(from, to), self.type_depth(), "field", sig);
            it.doc = self.doc_for(from, to);
            self.o.push(it);
        }
    }

    fn on_open(&mut self, i: usize) {
        let from = self.stmt_start;
        let header = header_text(self.src, &self.mask, from, i);
        if self.enum_consts_open() {
            // An enum constant with a body: `GREEN("g") { … }`.
            let line = self.line_at(from, i);
            self.enum_const(&header, line);
            self.frames.push(Frame::Expr { keep_stmt: false });
            return;
        }
        let enclosing = self.enclosing();
        match classify_header(&header, enclosing.as_deref()) {
            Header::Type { kind, name, sig, is_enum } => {
                let mut it = OutlineItem::new(self.line_at(from, i), self.type_depth(), kind, sig);
                it.doc = self.doc_for(from, i);
                it.entry = is_entry_type(&it.signature);
                let idx = self.o.push(it);
                self.frames.push(Frame::Type { item: idx, name, is_enum, consts_open: is_enum, fields: 0, consts: 0 });
                self.stmt_start = i + 1;
            }
            Header::Callable { sig, ctor } => {
                let idx = self.push_member(if ctor { "ctor" } else { "method" }, sig, from, i);
                self.frames.push(Frame::Member { item: idx });
                self.stmt_start = i + 1;
            }
            // `int[] A = {…}` / `= new Runnable() {…}`: the statement goes on
            // to its `;`, which records the field.
            Header::Field { .. } => self.frames.push(Frame::Expr { keep_stmt: true }),
            Header::Initializer | Header::Empty | Header::Package(_) | Header::Import(_) => {
                self.frames.push(Frame::Member { item: None });
                self.stmt_start = i + 1;
            }
        }
    }

    fn on_semicolon(&mut self, i: usize) {
        let from = self.stmt_start;
        let header = header_text(self.src, &self.mask, from, i);
        self.stmt_start = i + 1;
        if self.enum_consts_open() {
            if !header.is_empty() {
                let line = self.line_at(from, i);
                self.enum_const(&header, line);
            }
            if let Some(Frame::Type { consts_open, .. }) = self.frames.last_mut() {
                *consts_open = false;
            }
            return;
        }
        let enclosing = self.enclosing();
        match classify_header(&header, enclosing.as_deref()) {
            Header::Package(p) => self.o.notes.push(format!("package {p}")),
            Header::Import(p) => self.o.imports.push(p),
            Header::Callable { sig, .. } => {
                self.push_member("method", sig, from, i);
            }
            Header::Field { sig } => self.push_field(sig, from, i),
            _ => {}
        }
    }

    fn on_comma(&mut self, i: usize) {
        let from = self.stmt_start;
        let header = header_text(self.src, &self.mask, from, i);
        let line = self.line_at(from, i);
        self.enum_const(&header, line);
        self.stmt_start = i + 1;
    }

    fn on_close(&mut self, i: usize) {
        let line = self.lines.of(i);
        // A trailing enum constant right before the enum's `}` (no `;`).
        if self.enum_consts_open() {
            let header = header_text(self.src, &self.mask, self.stmt_start, i);
            if !header.is_empty() {
                let l = self.line_at(self.stmt_start, i);
                self.enum_const(&header, l);
            }
        }
        match self.frames.pop() {
            None => {
                self.o.degraded = true;
                self.stmt_start = i + 1;
            }
            Some(Frame::Type { item, fields, consts, .. }) => {
                self.o.set_end(item, line);
                let depth = self.type_depth() + 1;
                if fields > FIELDS_PER_TYPE {
                    self.o.push(OutlineItem::new(line, depth, "more", format!("…(필드 {}개 더)", fields - FIELDS_PER_TYPE)));
                }
                if consts > ENUM_CONSTS_MAX {
                    self.o.push(OutlineItem::new(line, depth, "more", format!("…(상수 {}개 더)", consts - ENUM_CONSTS_MAX)));
                }
                self.stmt_start = i + 1;
            }
            Some(Frame::Member { item }) => {
                self.o.set_end(item, line);
                self.stmt_start = i + 1;
            }
            Some(Frame::Expr { keep_stmt }) => {
                if !keep_stmt && self.header_level() {
                    self.stmt_start = i + 1;
                }
            }
        }
    }
}

fn outline_java(src: &[u8]) -> Outline {
    let (mask, docs) = lex_java(src);
    let mut s = JavaScan {
        src,
        mask,
        docs,
        lines: Lines::new(src),
        o: Outline::with_lang(Lang::Java),
        frames: Vec::new(),
        paren: 0,
        stmt_start: 0,
    };
    for i in 0..src.len() {
        if s.mask[i] != CODE {
            continue;
        }
        let c = src[i];
        if !s.header_level() {
            match c {
                b'{' => s.frames.push(Frame::Expr { keep_stmt: false }),
                b'}' => s.on_close(i),
                _ => {}
            }
            continue;
        }
        match c {
            b'(' => s.paren += 1,
            b')' => s.paren = (s.paren - 1).max(0),
            b'{' if s.paren > 0 => s.frames.push(Frame::Expr { keep_stmt: true }),
            b'{' => s.on_open(i),
            b';' if s.paren == 0 => s.on_semicolon(i),
            b',' if s.paren == 0 && s.enum_consts_open() => s.on_comma(i),
            b'}' => s.on_close(i),
            _ => {}
        }
    }
    if !s.frames.is_empty() {
        s.o.degraded = true;
    }
    s.o
}

// ---------------------------------------------------------------------------
// XML (MyBatis / iBATIS / web.xml / struts-config / Spring beans)
// ---------------------------------------------------------------------------

enum XEv {
    Open { name: String, attrs: Vec<(String, String)>, self_closing: bool, pos: usize },
    Close { name: String, pos: usize },
    Text { text: String },
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Byte-level XML event scanner (comments / PI / DOCTYPE skipped, CDATA as
/// text, quote-aware attributes). Returns the events + whether a tag was left
/// unterminated.
fn xml_events(src: &[u8]) -> (Vec<XEv>, bool) {
    let n = src.len();
    let mut out = Vec::new();
    let mut broken = false;
    let mut i = 0;
    let ws = |b: u8| b.is_ascii_whitespace();
    while i < n {
        if src[i] != b'<' {
            let s = i;
            while i < n && src[i] != b'<' {
                i += 1;
            }
            out.push(XEv::Text { text: decode_entities(&lossy(&src[s..i])) });
            continue;
        }
        if src[i..].starts_with(b"<!--") {
            i = find_bytes(src, i + 4, b"-->").map(|p| p + 3).unwrap_or(n);
            continue;
        }
        if src[i..].starts_with(b"<![CDATA[") {
            let end = find_bytes(src, i + 9, b"]]>");
            let stop = end.unwrap_or(n);
            out.push(XEv::Text { text: lossy(&src[i + 9..stop]) });
            i = end.map(|p| p + 3).unwrap_or(n);
            continue;
        }
        if src[i..].starts_with(b"<?") {
            i = find_bytes(src, i + 2, b"?>").map(|p| p + 2).unwrap_or(n);
            continue;
        }
        if src[i..].starts_with(b"<!") {
            while i < n && src[i] != b'>' {
                i += 1;
            }
            i += 1;
            continue;
        }
        let pos = i;
        i += 1;
        let close = src.get(i) == Some(&b'/');
        if close {
            i += 1;
        }
        let ns = i;
        while i < n && !ws(src[i]) && src[i] != b'>' && src[i] != b'/' {
            i += 1;
        }
        let name = lossy(&src[ns..i]);
        if close {
            while i < n && src[i] != b'>' {
                i += 1;
            }
            if i >= n {
                broken = true;
            }
            i += 1;
            out.push(XEv::Close { name, pos });
            continue;
        }
        let mut attrs = Vec::new();
        let mut self_closing = false;
        let mut terminated = false;
        while i < n {
            while i < n && ws(src[i]) {
                i += 1;
            }
            if i >= n {
                break;
            }
            if src[i] == b'>' {
                i += 1;
                terminated = true;
                break;
            }
            if src[i] == b'/' && src.get(i + 1) == Some(&b'>') {
                self_closing = true;
                terminated = true;
                i += 2;
                break;
            }
            let an = i;
            while i < n && !ws(src[i]) && src[i] != b'=' && src[i] != b'>' && src[i] != b'/' {
                i += 1;
            }
            if i == an {
                i += 1; // a stray byte — skip it (no infinite loop)
                continue;
            }
            let aname = lossy(&src[an..i]);
            while i < n && ws(src[i]) {
                i += 1;
            }
            let mut val = String::new();
            if i < n && src[i] == b'=' {
                i += 1;
                while i < n && ws(src[i]) {
                    i += 1;
                }
                if i < n && (src[i] == b'"' || src[i] == b'\'') {
                    let q = src[i];
                    i += 1;
                    let vs = i;
                    while i < n && src[i] != q {
                        i += 1;
                    }
                    val = decode_entities(&lossy(&src[vs..i.min(n)]));
                    i += 1;
                } else {
                    let vs = i;
                    while i < n && !ws(src[i]) && src[i] != b'>' {
                        i += 1;
                    }
                    val = lossy(&src[vs..i]);
                }
            }
            attrs.push((aname, val));
        }
        if !terminated {
            broken = true;
        }
        out.push(XEv::Open { name, attrs, self_closing, pos });
    }
    (out, broken)
}

fn attr<'a>(attrs: &'a [(String, String)], keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()))
        .filter(|v| !v.trim().is_empty())
}

/// Replace MyBatis/iBATIS bind markers (`#{x}`, `${x}`, `#x#`, `$x$`) with `?`
/// so they don't read as SQL words.
fn strip_binds(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '#' || c == '$' {
            if chars.get(i + 1) == Some(&'{') {
                if let Some(off) = chars[i + 2..].iter().position(|&x| x == '}') {
                    out.push_str(" ? ");
                    i += off + 3;
                    continue;
                }
            } else if let Some(off) = chars[i + 1..].iter().take(60).position(|&x| x == c) {
                let inner = &chars[i + 1..i + 1 + off];
                if !inner.is_empty() && !inner.iter().any(|x| x.is_whitespace()) {
                    out.push_str(" ? ");
                    i += off + 2;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

const STMT_TAGS: &[&str] = &["select", "insert", "update", "delete", "sql", "procedure", "statement"];

struct XStmt {
    tag: String,
    attrs: Vec<(String, String)>,
    line: usize,
    depth: usize,
    sql: String,
    includes: Vec<String>,
    skip_depth: Option<usize>,
}

fn finish_stmt(o: &mut Outline, st: XStmt, end_line: Option<usize>, crud_acc: &mut Vec<(String, String)>) {
    let id = attr(&st.attrs, &["id"]).unwrap_or("?");
    let mut sig = format!("{} id={id}", st.tag);
    if let Some(p) = attr(&st.attrs, &["parameterType", "parameterClass", "parameterMap"]) {
        sig.push_str(&format!(" param={p}"));
    }
    if let Some(r) = attr(&st.attrs, &["resultType", "resultClass", "resultMap"]) {
        sig.push_str(&format!(" result={r}"));
    }
    let sql = strip_binds(&st.sql);
    let toks = sql_tokens(sql.as_bytes());
    let tables = sql_tables_in(&toks);
    let calls = sql_calls(&toks);
    let mut extra: Vec<String> = Vec::new();
    if !tables.is_empty() {
        extra.push(format!("테이블: {}", format_tables(&tables)));
        merge_tables(crud_acc, &tables);
    }
    if !calls.is_empty() {
        extra.push(format!("호출: {}", calls.join(", ")));
    }
    if !st.includes.is_empty() {
        extra.push(format!("include: {}", st.includes.join(", ")));
    }
    let mut it = OutlineItem::new(st.line, 0, "statement", trunc(&sig, SIG_MAX));
    if !extra.is_empty() {
        it.doc = Some(extra.join(" | "));
    }
    it.end_line = end_line;
    it.tables = tables;
    o.push(it);
}

fn outline_xml(src: &[u8]) -> Outline {
    let lines = Lines::new(src);
    let (events, broken) = xml_events(src);
    let mut o = Outline::with_lang(Lang::Xml);
    o.degraded = broken;
    let root = events.iter().find_map(|e| match e {
        XEv::Open { name, attrs, .. } => Some((local_name(name).to_string(), attrs.clone())),
        _ => None,
    });
    let Some((root, root_attrs)) = root else {
        o.notes.push("XML 요소가 없습니다.".into());
        return o;
    };
    let has_statements = events.iter().any(|e| matches!(e, XEv::Open { name, .. } if STMT_TAGS.contains(&local_name(name))));
    let ns = attr(&root_attrs, &["namespace"]).unwrap_or("-");
    match root.as_str() {
        "mapper" => outline_mapper(&events, &lines, format!("MyBatis mapper — namespace={ns}"), &mut o),
        "sqlMap" => outline_mapper(&events, &lines, format!("iBATIS sqlMap — namespace={ns}"), &mut o),
        "web-app" => outline_webxml(&events, &lines, &mut o),
        "struts-config" => outline_struts(&events, &lines, &mut o),
        "beans" => outline_beans(&events, &lines, &mut o),
        "configuration" | "sqlMapConfig" => outline_mybatis_config(&events, &lines, &root, &mut o),
        // A mapper fragment / non-standard root that still holds SQL statements.
        _ if has_statements => outline_mapper(&events, &lines, format!("SQL 매퍼 조각(root=<{root}>)"), &mut o),
        _ => {
            // Unknown XML: first-level element names with counts.
            o.approximate = true;
            let mut depth = 0usize;
            let mut counts: Vec<(String, usize)> = Vec::new();
            for e in &events {
                match e {
                    XEv::Open { name, self_closing, .. } => {
                        if depth == 1 {
                            let n = local_name(name).to_string();
                            match counts.iter_mut().find(|(k, _)| *k == n) {
                                Some((_, c)) => *c += 1,
                                None => counts.push((n, 1)),
                            }
                        }
                        if !self_closing {
                            depth += 1;
                        }
                    }
                    XEv::Close { .. } => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
            let list: Vec<String> = counts.iter().take(30).map(|(n, c)| format!("{n}×{c}")).collect();
            o.notes.push(format!("XML(root=<{root}>) — 매퍼/설정 형식이 아님. 하위 요소: {}", list.join(", ")));
        }
    }
    o
}

fn outline_mapper(events: &[XEv], lines: &Lines, note: String, o: &mut Outline) {
    o.notes.push(note);
    let mut depth = 0usize;
    let mut cur: Option<XStmt> = None;
    let mut crud: Vec<(String, String)> = Vec::new();
    for e in events {
        match e {
            XEv::Open { name, attrs, self_closing, pos } => {
                let lname = local_name(name);
                let d = depth + 1;
                if cur.is_none() {
                    if STMT_TAGS.contains(&lname) {
                        let st = XStmt {
                            tag: lname.to_string(),
                            attrs: attrs.clone(),
                            line: lines.of(*pos),
                            depth: d,
                            sql: String::new(),
                            includes: Vec::new(),
                            skip_depth: None,
                        };
                        if *self_closing {
                            finish_stmt(o, st, None, &mut crud);
                        } else {
                            cur = Some(st);
                        }
                    } else if lname == "resultMap" {
                        let id = attr(attrs, &["id"]).unwrap_or("?");
                        let ty = attr(attrs, &["type", "class"]).unwrap_or("?");
                        o.push(OutlineItem::new(lines.of(*pos), 0, "resultMap", format!("resultMap id={id} type={ty}")));
                    }
                } else if let Some(st) = cur.as_mut() {
                    if lname == "selectKey" && !self_closing && st.skip_depth.is_none() {
                        st.skip_depth = Some(d);
                    } else if lname == "include" {
                        if let Some(r) = attr(attrs, &["refid"]) {
                            st.includes.push(r.to_string());
                        }
                    }
                }
                if !self_closing {
                    depth = d;
                }
            }
            XEv::Close { name, pos } => {
                let lname = local_name(name);
                let mut done = false;
                if let Some(st) = cur.as_mut() {
                    if st.skip_depth == Some(depth) {
                        st.skip_depth = None;
                    } else if depth == st.depth && lname == st.tag {
                        done = true;
                    }
                }
                if done {
                    if let Some(st) = cur.take() {
                        finish_stmt(o, st, Some(lines.of(*pos)), &mut crud);
                    }
                }
                depth = depth.saturating_sub(1);
            }
            XEv::Text { text } => {
                if let Some(st) = cur.as_mut() {
                    if st.skip_depth.is_none() {
                        st.sql.push_str(text);
                        st.sql.push(' ');
                    }
                }
            }
        }
    }
    if let Some(st) = cur.take() {
        o.degraded = true;
        finish_stmt(o, st, None, &mut crud);
    }
    if !crud.is_empty() {
        o.notes.push(format!("테이블 CRUD: {}", format_tables(&crud)));
    }
}

/// Collect `<group><leaf>text</leaf>…</group>` blocks directly under the root.
fn xml_groups(events: &[XEv], lines: &Lines, groups: &[&str]) -> Vec<(String, usize, HashMap<String, Vec<String>>)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur: Option<(String, usize, usize, HashMap<String, Vec<String>>)> = None;
    let mut leaf: Option<String> = None;
    for e in events {
        match e {
            XEv::Open { name, self_closing, pos, .. } => {
                let lname = local_name(name).to_string();
                if cur.is_none() && depth == 1 && groups.contains(&lname.as_str()) && !self_closing {
                    cur = Some((lname, lines.of(*pos), depth + 1, HashMap::new()));
                } else if cur.is_some() {
                    leaf = Some(lname);
                }
                if !self_closing {
                    depth += 1;
                }
            }
            XEv::Close { .. } => {
                if cur.as_ref().is_some_and(|(_, _, d, _)| *d == depth) {
                    if let Some((g, line, _, map)) = cur.take() {
                        out.push((g, line, map));
                    }
                }
                leaf = None;
                depth = depth.saturating_sub(1);
            }
            XEv::Text { text } => {
                if let (Some((_, _, _, map)), Some(l)) = (cur.as_mut(), leaf.as_ref()) {
                    let t = text.trim();
                    if !t.is_empty() {
                        map.entry(l.clone()).or_default().push(t.to_string());
                    }
                }
            }
        }
    }
    out
}

fn outline_webxml(events: &[XEv], lines: &Lines, o: &mut Outline) {
    o.notes.push("web.xml — 서블릿·필터·URL 매핑".into());
    let groups = ["servlet", "servlet-mapping", "filter", "filter-mapping", "listener", "context-param"];
    for (g, line, map) in xml_groups(events, lines, &groups) {
        let get = |k: &str| map.get(k).map(|v| v.join(", ")).unwrap_or_else(|| "?".into());
        let (sig, entry) = match g.as_str() {
            "servlet" => {
                let target = map.get("servlet-class").or_else(|| map.get("jsp-file")).map(|v| v.join(", ")).unwrap_or_else(|| "?".into());
                (format!("servlet {} → {target}", get("servlet-name")), true)
            }
            "servlet-mapping" => (format!("servlet-mapping {}: {}", get("servlet-name"), get("url-pattern")), true),
            "filter" => (format!("filter {} → {}", get("filter-name"), get("filter-class")), false),
            "filter-mapping" => (format!("filter-mapping {}: {}", get("filter-name"), get("url-pattern")), false),
            "listener" => (format!("listener {}", get("listener-class")), false),
            _ => (format!("context-param {} = {}", get("param-name"), trunc(&get("param-value"), 120)), false),
        };
        let mut it = OutlineItem::new(line, 0, "config", trunc(&sig, SIG_MAX));
        it.entry = entry;
        o.push(it);
    }
}

fn outline_struts(events: &[XEv], lines: &Lines, o: &mut Outline) {
    o.notes.push("struts-config — 액션 매핑".into());
    for e in events {
        let XEv::Open { name, attrs, pos, .. } = e else { continue };
        match local_name(name) {
            "action" => {
                let mut sig = format!(
                    "action path={} → {}",
                    attr(attrs, &["path"]).unwrap_or("?"),
                    attr(attrs, &["type"]).unwrap_or("?")
                );
                if let Some(f) = attr(attrs, &["name"]) {
                    sig.push_str(&format!(" form={f}"));
                }
                if let Some(p) = attr(attrs, &["parameter"]) {
                    sig.push_str(&format!(" parameter={p}"));
                }
                let mut it = OutlineItem::new(lines.of(*pos), 0, "config", trunc(&sig, SIG_MAX));
                it.entry = true;
                o.push(it);
            }
            "form-bean" => {
                let sig = format!(
                    "form-bean {} → {}",
                    attr(attrs, &["name"]).unwrap_or("?"),
                    attr(attrs, &["type"]).unwrap_or("?")
                );
                o.push(OutlineItem::new(lines.of(*pos), 0, "config", trunc(&sig, SIG_MAX)));
            }
            _ => {}
        }
    }
}

fn outline_beans(events: &[XEv], lines: &Lines, o: &mut Outline) {
    o.notes.push("Spring XML 설정 — bean 정의".into());
    let mut depth = 0usize;
    for e in events {
        match e {
            XEv::Open { name, attrs, self_closing, pos } => {
                let lname = local_name(name);
                if depth == 1 {
                    match lname {
                        "bean" => {
                            let id = attr(attrs, &["id", "name"]).unwrap_or("(익명)");
                            let class = attr(attrs, &["class", "parent"]).unwrap_or("?");
                            o.push(OutlineItem::new(lines.of(*pos), 0, "bean", trunc(&format!("bean {id} → {class}"), SIG_MAX)));
                        }
                        "component-scan" => {
                            o.notes.push(format!("component-scan base-package={}", attr(attrs, &["base-package"]).unwrap_or("?")));
                        }
                        "import" => {
                            o.notes.push(format!("import resource={}", attr(attrs, &["resource"]).unwrap_or("?")));
                        }
                        _ => {}
                    }
                }
                if !self_closing {
                    depth += 1;
                }
            }
            XEv::Close { .. } => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
}

fn outline_mybatis_config(events: &[XEv], lines: &Lines, root: &str, o: &mut Outline) {
    o.notes.push(format!("{root} — 매퍼 등록 목록"));
    for e in events {
        let XEv::Open { name, attrs, pos, .. } = e else { continue };
        let lname = local_name(name);
        if lname == "mapper" || lname == "sqlMap" {
            if let Some(r) = attr(attrs, &["resource", "class", "url"]) {
                o.push(OutlineItem::new(lines.of(*pos), 0, "config", format!("{lname} {r}")));
            }
        } else if lname == "package" {
            if let Some(r) = attr(attrs, &["name"]) {
                o.push(OutlineItem::new(lines.of(*pos), 0, "config", format!("package {r}")));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SQL / PL-SQL
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum TK {
    Word,
    Sym(u8),
    Str,
}

#[derive(Debug, Clone)]
struct Tok {
    kind: TK,
    start: usize,
    end: usize,
    /// Uppercased word (quoted identifiers keep their case); empty otherwise.
    up: String,
}

fn is_sql_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'#' | b'.' | b'@') || b >= 0x80
}

/// SQL tokens: words (incl. `schema.table`, `t@dblink`), quoted identifiers,
/// string literals (`''` escape, Oracle `q'[…]'`), symbols. Comments dropped.
fn sql_tokens(src: &[u8]) -> Vec<Tok> {
    let n = src.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let c = src[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'-' && src.get(i + 1) == Some(&b'-') {
            while i < n && src[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && src.get(i + 1) == Some(&b'*') {
            i = find_bytes(src, i + 2, b"*/").map(|p| p + 2).unwrap_or(n);
            continue;
        }
        if (c == b'q' || c == b'Q') && src.get(i + 1) == Some(&b'\'') && i + 2 < n {
            let open = src[i + 2];
            let close = match open {
                b'[' => b']',
                b'{' => b'}',
                b'(' => b')',
                b'<' => b'>',
                o => o,
            };
            let mut j = i + 3;
            let mut end = n;
            while j + 1 < n {
                if src[j] == close && src[j + 1] == b'\'' {
                    end = j + 2;
                    break;
                }
                j += 1;
            }
            out.push(Tok { kind: TK::Str, start: i, end, up: String::new() });
            i = end;
            continue;
        }
        if c == b'\'' {
            let mut j = i + 1;
            let mut end = n;
            while j < n {
                if src[j] == b'\'' {
                    if src.get(j + 1) == Some(&b'\'') {
                        j += 2;
                        continue;
                    }
                    end = j + 1;
                    break;
                }
                j += 1;
            }
            out.push(Tok { kind: TK::Str, start: i, end, up: String::new() });
            i = end;
            continue;
        }
        if c == b'"' {
            let mut j = i + 1;
            while j < n && src[j] != b'"' {
                j += 1;
            }
            let inner = lossy(&src[i + 1..j.min(n)]);
            let end = (j + 1).min(n);
            out.push(Tok { kind: TK::Word, start: i, end, up: inner });
            i = end;
            continue;
        }
        if is_sql_word_byte(c) {
            let s = i;
            while i < n && is_sql_word_byte(src[i]) {
                i += 1;
            }
            out.push(Tok { kind: TK::Word, start: s, end: i, up: lossy(&src[s..i]).to_uppercase() });
            continue;
        }
        out.push(Tok { kind: TK::Sym(c), start: i, end: i + 1, up: String::new() });
        i += 1;
    }
    out
}

fn word(t: &[Tok], i: usize) -> Option<&str> {
    t.get(i).filter(|x| x.kind == TK::Word).map(|x| x.up.as_str())
}

fn sym(t: &[Tok], i: usize) -> Option<u8> {
    match t.get(i)?.kind {
        TK::Sym(c) => Some(c),
        _ => None,
    }
}

/// Index just past the `)` matching the `(` at `i`.
fn skip_parens(t: &[Tok], i: usize) -> usize {
    let mut depth = 0i32;
    let mut j = i;
    while j < t.len() {
        match sym(t, j) {
            Some(b'(') => depth += 1,
            Some(b')') => {
                depth -= 1;
                if depth <= 0 {
                    return j + 1;
                }
            }
            _ => {}
        }
        j += 1;
    }
    t.len()
}

/// Reserved words that can never be a table name (and end a FROM list).
const SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "GROUP", "BY", "ORDER", "HAVING", "UNION", "ALL", "INTERSECT", "MINUS",
    "EXCEPT", "CONNECT", "START", "WITH", "LIMIT", "OFFSET", "FETCH", "FOR", "ON", "JOIN", "INNER",
    "LEFT", "RIGHT", "FULL", "CROSS", "OUTER", "NATURAL", "SET", "VALUES", "VALUE", "RETURNING", "INTO",
    "AS", "AND", "OR", "NOT", "NULL", "IS", "IN", "EXISTS", "CASE", "WHEN", "THEN", "ELSE", "END",
    "LOOP", "BEGIN", "DECLARE", "IF", "ELSIF", "USING", "MATCHED", "MERGE", "INSERT", "UPDATE",
    "DELETE", "TABLE", "LATERAL", "DISTINCT", "TOP", "LIKE", "BETWEEN", "PARTITION", "OVER", "WINDOW",
    "PIVOT", "UNPIVOT", "MODEL", "SAMPLE", "NOWAIT", "WAIT", "SKIP", "LOCKED", "OF", "BULK", "COLLECT",
    "FORALL", "EXECUTE", "IMMEDIATE", "RETURN", "CALL", "EXEC", "DEFAULT", "PRIOR", "LEVEL", "ONLY",
    "ROWS", "ROW", "NEXT", "FIRST", "DUAL", "SIBLINGS", "NOCYCLE", "TRUNCATE", "CREATE", "DROP",
    "ALTER", "GRANT", "REVOKE", "COMMIT", "ROLLBACK", "WHILE", "EXIT", "RAISE", "EXCEPTION", "OPEN",
    "CLOSE", "CURSOR", "TYPE", "ROWTYPE", "IS", "OUT", "NOCOPY",
];

fn is_table_name(w: &str, ctes: &HashSet<String>) -> bool {
    let Some(first) = w.chars().next() else { return false };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    let w = w.trim_end_matches('.');
    !SQL_KEYWORDS.contains(&w) && !ctes.contains(w)
}

fn add_table(out: &mut Vec<(String, String)>, name: &str, crud: char) {
    let name = name.trim_end_matches('.').to_string();
    merge_tables(out, &[(name, crud.to_string())]);
}

fn read_one(t: &[Tok], i: usize, crud: char, ctes: &HashSet<String>, out: &mut Vec<(String, String)>) {
    if let Some(w) = word(t, i) {
        if is_table_name(w, ctes) {
            add_table(out, w, crud);
        }
    }
}

/// A FROM-style list: `a x, (subquery) y, b` — records the named tables; the
/// subqueries' own FROMs are found by the main loop.
fn read_list(t: &[Tok], i: usize, crud: char, ctes: &HashSet<String>, out: &mut Vec<(String, String)>) {
    let mut j = i;
    loop {
        if sym(t, j) == Some(b'(') {
            j = skip_parens(t, j);
        } else if let Some(w) = word(t, j) {
            if !is_table_name(w, ctes) {
                break;
            }
            add_table(out, w, crud);
            j += 1;
        } else {
            break;
        }
        if word(t, j) == Some("AS") {
            j += 2;
        } else if let Some(w) = word(t, j) {
            if !SQL_KEYWORDS.contains(&w) {
                j += 1;
            }
        }
        if sym(t, j) == Some(b',') {
            j += 1;
            continue;
        }
        break;
    }
}

/// Functions whose argument syntax contains `FROM` (`EXTRACT(YEAR FROM d)`).
const FROM_FUNCS: &[&str] = &["EXTRACT", "TRIM", "SUBSTRING", "SUBSTR", "OVERLAY", "POSITION"];

/// Tables referenced by a token stream, with C/R/U/D (first-seen order).
fn sql_tables_in(t: &[Tok]) -> Vec<(String, String)> {
    let mut ctes: HashSet<String> = HashSet::new();
    for i in 0..t.len() {
        if let (Some(w), Some("AS"), Some(b'(')) = (word(t, i), word(t, i + 1), sym(t, i + 2)) {
            ctes.insert(w.to_string());
        }
    }
    let mut out = Vec::new();
    let mut expect_into: Option<char> = None;
    let mut skip_from: Option<usize> = None;
    let mut parens: Vec<String> = Vec::new();
    for i in 0..t.len() {
        match sym(t, i) {
            Some(b';') => {
                expect_into = None;
                continue;
            }
            Some(b'(') => {
                parens.push(i.checked_sub(1).and_then(|p| word(t, p)).unwrap_or("").to_string());
                continue;
            }
            Some(b')') => {
                parens.pop();
                continue;
            }
            Some(_) => continue,
            None => {}
        }
        let Some(w) = word(t, i) else { continue };
        let prev = i.checked_sub(1).and_then(|p| word(t, p));
        match w {
            "INSERT" => expect_into = Some('C'),
            "MERGE" => expect_into = Some('U'),
            "INTO" => {
                if let Some(c) = expect_into.take() {
                    read_one(t, i + 1, c, &ctes, &mut out);
                }
            }
            "UPDATE" => {
                if !matches!(prev, Some("FOR") | Some("KEY") | Some("THEN")) {
                    read_one(t, i + 1, 'U', &ctes, &mut out);
                }
            }
            "DELETE" => {
                if word(t, i + 1) == Some("FROM") {
                    read_list(t, i + 2, 'D', &ctes, &mut out);
                    skip_from = Some(i + 1);
                } else if prev != Some("THEN") {
                    read_one(t, i + 1, 'D', &ctes, &mut out);
                }
            }
            "FROM" => {
                let in_func = parens.last().map(|f| FROM_FUNCS.contains(&f.as_str())).unwrap_or(false);
                if skip_from != Some(i) && !in_func {
                    read_list(t, i + 1, 'R', &ctes, &mut out);
                }
            }
            "JOIN" => read_one(t, i + 1, 'R', &ctes, &mut out),
            "USING" => {
                if sym(t, i + 1) != Some(b'(') {
                    read_one(t, i + 1, 'R', &ctes, &mut out);
                }
            }
            "TRUNCATE" => {
                if word(t, i + 1) == Some("TABLE") {
                    read_one(t, i + 2, 'D', &ctes, &mut out);
                }
            }
            _ => {}
        }
    }
    out
}

/// Tables referenced by a SQL text with their C/R/U/D letters.
#[cfg_attr(not(test), allow(dead_code))] // public helper; the outlines use `sql_tables_in`
pub fn sql_tables(sql: &str) -> Vec<(String, String)> {
    sql_tables_in(&sql_tokens(sql.as_bytes()))
}

/// `{call PKG.P(?)}` / `EXEC P` targets.
fn sql_calls(t: &[Tok]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for i in 0..t.len() {
        if matches!(word(t, i), Some("CALL") | Some("EXEC") | Some("EXECUTE")) {
            if let Some(n) = word(t, i + 1).filter(|n| *n != "IMMEDIATE") {
                if !out.iter().any(|x| x == n) {
                    out.push(n.to_string());
                }
            }
        }
    }
    out
}

/// Re-join tokens from the original text: `VARCHAR2 ( 20 BYTE )` →
/// `VARCHAR2(20 BYTE)`, `p_id IN NUMBER , p_x` → `p_id IN NUMBER, p_x`.
fn render_tokens(src: &[u8], t: &[Tok]) -> String {
    let mut out = String::new();
    let mut prev_word = false;
    for tok in t {
        match tok.kind {
            TK::Word | TK::Str => {
                if prev_word {
                    out.push(' ');
                }
                out.push_str(&lossy(&src[tok.start..tok.end.min(src.len())]));
                prev_word = true;
            }
            TK::Sym(c) => {
                if c == b',' {
                    out.push_str(", ");
                } else {
                    out.push(c as char);
                }
                // `(10) NOT` keeps its space; `NUMBER(` / `a, b` don't double it.
                prev_word = c == b')';
            }
        }
    }
    collapse_ws(&out)
}

/// The text of a string literal token (quotes stripped, `''` unescaped);
/// `None` when not valid UTF-8 (legacy EUC-KR comments are skipped).
fn str_value(src: &[u8], tok: &Tok) -> Option<String> {
    let raw = std::str::from_utf8(&src[tok.start..tok.end.min(src.len())]).ok()?;
    let inner = raw.strip_prefix('\'').unwrap_or(raw);
    let inner = inner.strip_suffix('\'').unwrap_or(inner);
    Some(inner.replace("''", "'"))
}

fn last_segment(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_string()
}

/// A `/` alone on its line (SQL*Plus statement terminator).
fn is_slash_line(src: &[u8], tok: &Tok) -> bool {
    if tok.kind != TK::Sym(b'/') {
        return false;
    }
    let before = src[..tok.start].iter().rev().take_while(|&&b| b != b'\n').all(|b| b.is_ascii_whitespace());
    let after = src[tok.end.min(src.len())..].iter().take_while(|&&b| b != b'\n').all(|b| b.is_ascii_whitespace());
    before && after
}

const PLSQL_OBJECTS: &[&str] = &["PROCEDURE", "FUNCTION", "PACKAGE", "PACKAGE BODY", "TRIGGER", "TYPE", "TYPE BODY"];

/// Token index where the object starting at `from` ends (exclusive).
fn object_end(src: &[u8], t: &[Tok], from: usize, label: &str) -> usize {
    let plsql = PLSQL_OBJECTS.contains(&label);
    let mut depth = 0i32;
    for j in from..t.len() {
        if is_slash_line(src, &t[j]) {
            return j;
        }
        match sym(t, j) {
            Some(b'(') => depth += 1,
            Some(b')') => depth -= 1,
            Some(b';') if !plsql && depth <= 0 => return j,
            _ => {}
        }
        if plsql && word(t, j) == Some("CREATE") && j > 0 && matches!(sym(t, j - 1), Some(b';') | Some(b'/')) {
            return j;
        }
    }
    t.len()
}

struct SqlComments {
    tables: HashMap<String, String>,
    columns: HashMap<(String, String), String>,
}

fn sql_comments(src: &[u8], t: &[Tok]) -> SqlComments {
    let mut c = SqlComments { tables: HashMap::new(), columns: HashMap::new() };
    for i in 0..t.len() {
        if word(t, i) != Some("COMMENT") || word(t, i + 1) != Some("ON") {
            continue;
        }
        let target = word(t, i + 2);
        let Some(name) = word(t, i + 3) else { continue };
        if word(t, i + 4) != Some("IS") {
            continue;
        }
        let Some(val) = t.get(i + 5).filter(|x| x.kind == TK::Str).and_then(|x| str_value(src, x)) else { continue };
        match target {
            Some("TABLE") => {
                c.tables.insert(last_segment(name), val);
            }
            Some("COLUMN") => {
                let parts: Vec<&str> = name.split('.').collect();
                if parts.len() >= 2 {
                    let col = parts[parts.len() - 1].to_string();
                    let table = parts[parts.len() - 2].to_string();
                    c.columns.insert((table, col), val);
                }
            }
            _ => {}
        }
    }
    c
}

const COL_STOP: &[&str] = &[
    "NOT", "NULL", "DEFAULT", "CONSTRAINT", "PRIMARY", "REFERENCES", "CHECK", "UNIQUE", "GENERATED",
    "COMMENT", "ENABLE", "DISABLE", "AUTO_INCREMENT", "IDENTITY", "COLLATE", "CHARACTER",
];

fn outline_table(src: &[u8], t: &[Tok], lines: &Lines, k: usize, end: usize, name: &str, line: usize, cm: &SqlComments, o: &mut Outline) {
    let key = last_segment(&name.to_uppercase());
    let mut pk: Vec<String> = Vec::new();
    let mut cols: Vec<OutlineItem> = Vec::new();
    let mut ncols = 0usize;
    if sym(t, k) == Some(b'(') {
        let close = skip_parens(t, k).min(end.max(k + 1)).min(t.len());
        // Split the column list at depth-1 commas.
        let mut segs: Vec<(usize, usize)> = Vec::new();
        let mut depth = 0i32;
        let mut s = k + 1;
        for j in k + 1..close.saturating_sub(1) {
            match sym(t, j) {
                Some(b'(') => depth += 1,
                Some(b')') => depth -= 1,
                Some(b',') if depth == 0 => {
                    segs.push((s, j));
                    s = j + 1;
                }
                _ => {}
            }
        }
        segs.push((s, close.saturating_sub(1)));
        for (a, b) in segs {
            if a >= b {
                continue;
            }
            let seg = &t[a..b];
            let first = word(seg, 0).unwrap_or("");
            let has_pk = seg.windows(2).any(|w| w[0].up == "PRIMARY" && w[1].up == "KEY");
            if matches!(first, "CONSTRAINT" | "PRIMARY" | "FOREIGN" | "UNIQUE" | "CHECK" | "KEY" | "INDEX" | "PERIOD" | "SUPPLEMENTAL") {
                if has_pk {
                    if let Some(p) = seg.iter().position(|x| x.kind == TK::Sym(b'(')) {
                        let close = skip_parens(seg, p);
                        pk.extend(seg[p + 1..close.saturating_sub(1)].iter().filter(|x| x.kind == TK::Word).map(|x| x.up.clone()));
                    }
                }
                continue;
            }
            if first.is_empty() {
                continue;
            }
            let col = first.to_string();
            let stop = seg.iter().enumerate().skip(1).find(|(_, x)| x.kind == TK::Word && COL_STOP.contains(&x.up.as_str())).map(|(i, _)| i).unwrap_or(seg.len());
            let ty = render_tokens(src, &seg[1..stop]);
            if has_pk {
                pk.push(col.clone());
            }
            ncols += 1;
            if ncols > COLUMNS_MAX {
                continue;
            }
            let mut it = OutlineItem::new(lines.of(seg[0].start), 1, "column", trunc(&format!("{col} {ty}"), SIG_MAX));
            let inline = seg.windows(2).find(|w| w[0].up == "COMMENT" && w[1].kind == TK::Str).and_then(|w| str_value(src, &w[1]));
            it.doc = cm.columns.get(&(key.clone(), col.clone())).cloned().or(inline).map(|d| trunc(&d, DOC_MAX));
            cols.push(it);
        }
    }
    let mut sig = format!("TABLE {name}");
    if !pk.is_empty() {
        sig.push_str(&format!(" (PK: {})", pk.join(", ")));
    }
    let mut it = OutlineItem::new(line, 0, "table", sig);
    it.doc = cm.tables.get(&key).map(|d| trunc(d, DOC_MAX));
    if word(t, k) == Some("AS") {
        let tables = sql_tables_in(&t[k..end.min(t.len())]);
        if !tables.is_empty() {
            it.doc = Some(format!("CTAS — 테이블: {}", format_tables(&tables)));
        }
    }
    if end > k && end <= t.len() {
        it.end_line = Some(lines.of(t[end.saturating_sub(1)].start));
    }
    o.push(it);
    for c in cols {
        o.push(c);
    }
    if ncols > COLUMNS_MAX {
        o.push(OutlineItem::new(line, 1, "more", format!("…(컬럼 {}개 더)", ncols - COLUMNS_MAX)));
    }
}

/// `NAME(params) RETURN type` starting at token `k` (just after the name).
/// Returns (signature tail, index after the signature).
fn routine_sig(src: &[u8], t: &[Tok], k: usize) -> (String, usize) {
    let mut j = k;
    let mut tail = String::new();
    if sym(t, j) == Some(b'(') {
        let close = skip_parens(t, j);
        tail = render_tokens(src, &t[j..close]);
        j = close;
    }
    if word(t, j) == Some("RETURN") {
        if let Some(r) = t.get(j + 1) {
            tail.push_str(&format!(" RETURN {}", lossy(&src[r.start..r.end.min(src.len())])));
            j += 2;
        }
    }
    (tail, j)
}

fn outline_package(src: &[u8], t: &[Tok], lines: &Lines, k: usize, end: usize, body: bool, crud: &mut Vec<(String, String)>, o: &mut Outline) {
    // Members: PROCEDURE|FUNCTION <name> not preceded by END.
    let mut members: Vec<usize> = Vec::new();
    for j in k..end.min(t.len()) {
        if matches!(word(t, j), Some("PROCEDURE") | Some("FUNCTION")) && word(t, j.wrapping_sub(1)) != Some("END") && word(t, j + 1).is_some() {
            members.push(j);
        }
    }
    for (mi, &j) in members.iter().enumerate() {
        let kw = word(t, j).unwrap_or("");
        let name_tok = &t[j + 1];
        let name = lossy(&src[name_tok.start..name_tok.end.min(src.len())]);
        let (tail, after) = routine_sig(src, t, j + 2);
        let mut it = OutlineItem::new(lines.of(t[j].start), 1, "member", trunc(&format!("{kw} {name}{tail}"), SIG_MAX));
        if body {
            let stop = members.get(mi + 1).copied().unwrap_or(end).min(t.len());
            let range = &t[after.min(stop)..stop];
            let tables = sql_tables_in(range);
            if !tables.is_empty() {
                it.doc = Some(format!("테이블: {}", format_tables(&tables)));
                merge_tables(crud, &tables);
            }
            it.tables = tables;
            let up = name.to_uppercase();
            if let Some(e) = (after..stop).find(|&x| word(t, x) == Some("END") && word(t, x + 1) == Some(up.as_str())) {
                it.end_line = Some(lines.of(t[e].start));
            }
        }
        o.push(it);
    }
}

fn outline_sql(src: &[u8]) -> Outline {
    let t = sql_tokens(src);
    let lines = Lines::new(src);
    let mut o = Outline::with_lang(Lang::Sql);
    let cm = sql_comments(src, &t);
    let mut crud: Vec<(String, String)> = Vec::new();
    let mut found = false;
    let mut i = 0;
    while i < t.len() {
        if word(&t, i) != Some("CREATE") {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while matches!(
            word(&t, j),
            Some("OR" | "REPLACE" | "EDITIONABLE" | "NONEDITIONABLE" | "EDITIONING" | "FORCE" | "NOFORCE" | "GLOBAL" | "TEMPORARY" | "UNIQUE" | "BITMAP" | "PUBLIC")
        ) {
            j += 1;
        }
        let Some(kind) = word(&t, j) else {
            i += 1;
            continue;
        };
        let (label, mut k): (&str, usize) = match kind {
            "MATERIALIZED" if word(&t, j + 1) == Some("VIEW") => ("MATERIALIZED VIEW", j + 2),
            "PACKAGE" if word(&t, j + 1) == Some("BODY") => ("PACKAGE BODY", j + 2),
            "TYPE" if word(&t, j + 1) == Some("BODY") => ("TYPE BODY", j + 2),
            "TABLE" | "VIEW" | "INDEX" | "SEQUENCE" | "SYNONYM" | "PROCEDURE" | "FUNCTION" | "PACKAGE" | "TRIGGER" | "TYPE" => (kind, j + 1),
            _ => {
                i += 1;
                continue;
            }
        };
        if word(&t, k) == Some("IF") && word(&t, k + 1) == Some("NOT") && word(&t, k + 2) == Some("EXISTS") {
            k += 3;
        }
        let Some(name_tok) = t.get(k).filter(|x| x.kind == TK::Word) else {
            i += 1;
            continue;
        };
        let name = lossy(&src[name_tok.start..name_tok.end.min(src.len())]);
        let name = name.trim_matches('"').to_string();
        k += 1;
        found = true;
        let line = lines.of(t[i].start);
        let end = object_end(src, &t, k, label);
        let end_line = (end > k).then(|| lines.of(t[end.min(t.len()) - 1].start));
        match label {
            "TABLE" => outline_table(src, &t, &lines, k, end, &name, line, &cm, &mut o),
            "VIEW" | "MATERIALIZED VIEW" => {
                let tables = sql_tables_in(&t[k..end.min(t.len())]);
                let mut it = OutlineItem::new(line, 0, "view", format!("{label} {name}"));
                if !tables.is_empty() {
                    it.doc = Some(format!("참조: {}", format_tables(&tables)));
                }
                it.end_line = end_line;
                o.push(it);
            }
            "INDEX" => {
                let mut sig = format!("INDEX {name}");
                if let Some(on) = (k..end.min(t.len())).find(|&x| word(&t, x) == Some("ON")) {
                    if let Some(tb) = word(&t, on + 1) {
                        sig.push_str(&format!(" ON {tb}"));
                        if sym(&t, on + 2) == Some(b'(') {
                            let close = skip_parens(&t, on + 2);
                            sig.push_str(&render_tokens(src, &t[on + 2..close]));
                        }
                    }
                }
                o.push(OutlineItem::new(line, 0, "index", trunc(&sig, SIG_MAX)));
            }
            "SEQUENCE" => {
                o.push(OutlineItem::new(line, 0, "sequence", format!("SEQUENCE {name}")));
            }
            "SYNONYM" => {
                let target = (k..end.min(t.len())).find(|&x| word(&t, x) == Some("FOR")).and_then(|x| word(&t, x + 1)).unwrap_or("?");
                o.push(OutlineItem::new(line, 0, "synonym", format!("SYNONYM {name} FOR {target}")));
            }
            "PROCEDURE" | "FUNCTION" => {
                let (tail, after) = routine_sig(src, &t, k);
                let mut it = OutlineItem::new(line, 0, "routine", trunc(&format!("{label} {name}{tail}"), SIG_MAX));
                let tables = sql_tables_in(&t[after.min(end)..end.min(t.len())]);
                if !tables.is_empty() {
                    it.doc = Some(format!("테이블: {}", format_tables(&tables)));
                    merge_tables(&mut crud, &tables);
                }
                it.tables = tables;
                it.end_line = end_line;
                o.push(it);
            }
            "TRIGGER" => {
                let on = (k..end.min(t.len())).find(|&x| word(&t, x) == Some("ON"));
                let timing = on.map(|x| render_tokens(src, &t[k..x])).unwrap_or_default();
                let table = on.and_then(|x| word(&t, x + 1)).unwrap_or("?");
                let mut it = OutlineItem::new(line, 0, "trigger", trunc(&format!("TRIGGER {name} {timing} ON {table}"), SIG_MAX));
                if let Some(x) = on {
                    let tables = sql_tables_in(&t[(x + 2).min(end)..end.min(t.len())]);
                    if !tables.is_empty() {
                        it.doc = Some(format!("테이블: {}", format_tables(&tables)));
                    }
                }
                it.end_line = end_line;
                o.push(it);
            }
            _ => {
                // PACKAGE / PACKAGE BODY / TYPE / TYPE BODY
                let mut it = OutlineItem::new(line, 0, "package", format!("{label} {name}"));
                it.end_line = end_line;
                o.push(it);
                let body = label.ends_with("BODY");
                outline_package(src, &t, &lines, k, end, body, &mut crud, &mut o);
            }
        }
        i = end.max(i + 1);
    }
    if !found {
        let stmts = t.iter().filter(|x| x.kind == TK::Sym(b';')).count();
        o.notes.push(format!("SQL 스크립트 — 객체 정의(CREATE) 없음, 문장 약 {}개", stmts.max(1)));
        let tables = sql_tables_in(&t);
        if !tables.is_empty() {
            o.notes.push(format!("테이블: {}", format_tables(&tables)));
        }
    } else if !crud.is_empty() {
        o.notes.push(format!("루틴이 다루는 테이블 CRUD: {}", format_tables(&crud)));
    }
    o
}

// ---------------------------------------------------------------------------
// Markdown / generic fallback
// ---------------------------------------------------------------------------

fn outline_markdown(src: &[u8]) -> Outline {
    let text = lossy(src);
    let mut o = Outline::with_lang(Lang::Markdown);
    let mut fence: Option<(char, usize)> = None;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let lead = line.len() - trimmed.len();
        let fc = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        if lead <= 3 {
            if let Some(ch) = fc {
                let run = trimmed.chars().take_while(|c| *c == ch).count();
                if run >= 3 {
                    match fence {
                        Some((c, len)) if c == ch && run >= len && trimmed[run..].trim().is_empty() => fence = None,
                        None => fence = Some((ch, run)),
                        _ => {}
                    }
                    continue;
                }
            }
        }
        if fence.is_some() || lead > 3 {
            continue;
        }
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
            o.push(OutlineItem::new(i + 1, level - 1, "heading", trunc(trimmed[level..].trim(), SIG_MAX)));
        }
    }
    o
}

const GENERIC_PREFIXES: &[&str] = &[
    "export ", "class ", "interface ", "function ", "def ", "async def ", "func ", "namespace ", "public ",
    "private ", "protected ", "internal ", "module ", "struct ", "impl ", "fn ", "pub fn ", "enum ", "type ",
    "trait ", "sub ", "procedure ", "<%@",
];

fn outline_generic(src: &[u8]) -> Outline {
    let text = lossy(src);
    let mut o = Outline::with_lang(Lang::Other);
    o.approximate = true;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        let lower = t.to_lowercase();
        if GENERIC_PREFIXES.iter().any(|p| lower.starts_with(p)) {
            o.push(OutlineItem::new(i + 1, 0, "line", trunc(t.trim_end_matches('{').trim(), SIG_MAX)));
        }
    }
    o
}

// ---------------------------------------------------------------------------
// Repository map (D88): a whole-codebase orientation computed without an LLM
// ---------------------------------------------------------------------------

/// Options for [`repo_map`].
pub struct MapOpts {
    /// Stop parsing after this many source files.
    pub max_parsed: usize,
    /// Wall-clock budget; past it the map is rendered from what was collected.
    pub deadline: Duration,
    /// Identifier-like words from the request (table/type/route names) — what
    /// they hit is listed first.
    pub focus_terms: Vec<String>,
    /// Total output budget in characters.
    pub budget: usize,
    /// How much to render (D91): the standard orientation map, a larger one
    /// for FabriX (whose every lookup is a billed call), or only the
    /// request-specific sections.
    pub detail: MapDetail,
}

impl Default for MapOpts {
    fn default() -> Self {
        Self {
            max_parsed: 6000,
            deadline: Duration::from_secs(6),
            focus_terms: Vec::new(),
            budget: 12_000,
            detail: MapDetail::Standard,
        }
    }
}

impl MapOpts {
    /// The options for a `codebase_map` detail level ("extended" / "focus";
    /// anything else = standard).
    pub fn for_detail(detail: Option<&str>, focus_terms: Vec<String>) -> Self {
        let detail = match detail.map(str::trim) {
            Some("extended") => MapDetail::Extended,
            Some("focus") => MapDetail::Focus,
            _ => MapDetail::Standard,
        };
        let budget = if detail == MapDetail::Extended { 24_000 } else { 12_000 };
        Self { focus_terms, budget, detail, ..Self::default() }
    }
}

/// Render level of [`repo_map`] (D91).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapDetail {
    /// The orientation map every agent gets (D88).
    Standard,
    /// FabriX's codebase step: more key-file outlines + the bodies of the SQL
    /// statements the request names, so fewer lookup rounds are needed.
    Extended,
    /// FabriX's source-analysis step: only the request hits, dependency chains,
    /// statement bodies and key outlines (its transcript lacks the earlier map).
    Focus,
}

impl MapDetail {
    /// (key-file outlines, SQL statement bodies) rendered.
    fn counts(self) -> (usize, usize) {
        match self {
            MapDetail::Standard => (3, 0),
            MapDetail::Extended => (6, 5),
            MapDetail::Focus => (4, 5),
        }
    }

    /// Section budgets at the 12k reference total (scaled to `budget`):
    /// directory, request hits, routes, chains, hubs, tables, key outlines,
    /// statement bodies. Extended's are meant for its 24k budget (×2).
    fn weights(self) -> [usize; 8] {
        match self {
            MapDetail::Standard => [3000, 1200, 2200, 2000, 900, 1000, 2000, 0],
            MapDetail::Extended => [1500, 1000, 1300, 1500, 450, 600, 3500, 2000],
            MapDetail::Focus => [0, 2000, 0, 3000, 0, 0, 4000, 3000],
        }
    }
}

/// Lines of one SQL statement body shown in the map.
const STMT_BODY_LINES: usize = 40;

/// Directories the map skips on top of `files::SKIP_DIRS` (build output and
/// generated copies would skew the hub ranking).
const MAP_SKIP_DIRS: &[&str] = &["build", "out", "dist", "classes", ".settings", "generated", "logs"];

struct JavaFile {
    rel: String,
    outline: Outline,
    /// Top-level type names this file declares.
    types: Vec<String>,
    /// String literals — DAO files only (eGov/iBATIS DAOs call statements by
    /// `"namespace.id"` strings).
    strings: Vec<String>,
}

struct TypeDecl {
    file: usize,
    kind: &'static str,
    entry: bool,
    implements: Vec<String>,
    /// Simple type names of the fields (injected collaborators).
    fields: Vec<String>,
}

struct StmtRef {
    rel: String,
    ns: String,
    id: String,
    tables: Vec<(String, String)>,
    /// 1-based line range of the statement in its file.
    line: usize,
    end_line: Option<usize>,
}

#[derive(Default)]
struct DirStat {
    total: usize,
    direct: usize,
    exts: HashMap<String, usize>,
    children: Vec<String>,
}

fn simple_type(t: &str) -> String {
    let t = t.split('<').next().unwrap_or(t);
    let t = t.trim().trim_end_matches("...").trim_end_matches("[]");
    t.rsplit('.').next().unwrap_or(t).trim().to_string()
}

/// The declared name of a type signature.
fn type_name_of(sig: &str) -> Option<String> {
    let toks: Vec<&str> = sig.split_whitespace().collect();
    let i = toks.iter().position(|t| matches!(*t, "class" | "interface" | "enum" | "record" | "@interface"))?;
    let name: String = toks.get(i + 1)?.chars().take_while(|c| is_ident_char(*c)).collect();
    (!name.is_empty()).then_some(name)
}

/// Simple names after `implements` (or an interface's `extends`).
fn implemented(sig: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(p) = sig.find(" implements ").map(|p| p + 12).or_else(|| {
        (sig.contains(" interface ") || sig.starts_with("interface ")).then(|| sig.find(" extends ").map(|p| p + 9)).flatten()
    }) else {
        return out;
    };
    let rest = &sig[p..];
    let rest = rest.split(" permits ").next().unwrap_or(rest);
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in rest.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(simple_type(&cur));
                cur.clear();
                continue;
            }
            _ => {}
        }
        if depth == 0 && ch != '>' {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        out.push(simple_type(&cur));
    }
    out.retain(|s| !s.is_empty());
    out
}

/// The declared type of a field signature (`@Autowired private Foo<Bar> foo`).
fn field_type_of(sig: &str) -> Option<String> {
    let (_, rest) = peel_annotations(sig);
    let core = strip_modifiers(&rest);
    let words: Vec<&str> = core.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }
    let t = simple_type(&words[..words.len() - 1].join(" "));
    t.chars().next().is_some_and(|c| c.is_uppercase()).then_some(t)
}

/// A method signature's name (annotations peeled first — their args have parens).
fn method_name_of(sig: &str) -> String {
    let (_, rest) = peel_annotations(sig);
    rest.split('(').next().unwrap_or("").split_whitespace().last().unwrap_or("").to_string()
}

/// `(HTTP method, path)` of the first Spring mapping annotation in a signature.
fn mapping_of(sig: &str) -> Option<(String, String)> {
    const ANNS: &[(&str, &str)] = &[
        ("@GetMapping", "GET"),
        ("@PostMapping", "POST"),
        ("@PutMapping", "PUT"),
        ("@DeleteMapping", "DELETE"),
        ("@PatchMapping", "PATCH"),
        ("@RequestMapping", ""),
    ];
    let (pos, ann, verb) = ANNS.iter().filter_map(|(a, v)| sig.find(a).map(|p| (p, *a, *v))).min_by_key(|x| x.0)?;
    let after = &sig[pos + ann.len()..];
    let mut method = verb.to_string();
    let mut path = String::new();
    if let Some(args) = after.strip_prefix('(') {
        let args = &args[..args.find(')').unwrap_or(args.len())];
        if let Some(q) = args.find('"') {
            path = args[q + 1..].split('"').next().unwrap_or("").to_string();
        }
        if method.is_empty() {
            if let Some(m) = args.find("RequestMethod.") {
                method = args[m + 14..].chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            }
        }
    }
    if method.is_empty() {
        method = "ANY".into();
    }
    Some((method, path))
}

fn join_paths(a: &str, b: &str) -> String {
    let a = a.trim_end_matches('/');
    match (a.is_empty(), b.is_empty()) {
        (true, true) => "/".into(),
        (_, true) => a.to_string(),
        _ if b.starts_with('/') => format!("{a}{b}"),
        _ => format!("{a}/{b}"),
    }
}

fn is_dao_like(name: &str, sig: &str) -> bool {
    name.ends_with("DAO")
        || name.ends_with("Dao")
        || ["EgovAbstractDAO", "EgovAbstractMapper", "SqlMapClientDaoSupport", "SqlSessionDaoSupport"].iter().any(|p| sig.contains(p))
}

/// String literal contents of a Java source (≤200 chars each).
fn java_strings(src: &[u8]) -> Vec<String> {
    let (mask, _) = lex_java(src);
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        if mask[i] == STR && src[i] == b'"' {
            let s = i + 1;
            let mut j = s;
            while j < src.len() && mask[j] == STR && src[j] != b'"' {
                j += 1;
            }
            if j > s && j - s <= 200 {
                out.push(lossy(&src[s..j]));
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out
}

fn bump_tables(acc: &mut Vec<(String, String, usize)>, tables: &[(String, String)]) {
    for (t, c) in tables {
        match acc.iter_mut().find(|(n, _, _)| n == t) {
            Some((_, have, refs)) => {
                *have = order_crud(&format!("{have}{c}"));
                *refs += 1;
            }
            None => acc.push((t.clone(), order_crud(c), 1)),
        }
    }
}

fn clip_section(title: &str, lines: &[String], budget: usize) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = format!("## {title}\n");
    let mut used = out.chars().count();
    let mut shown = 0usize;
    for l in lines {
        let n = l.chars().count() + 1;
        if used + n > budget {
            break;
        }
        out.push_str(l);
        out.push('\n');
        used += n;
        shown += 1;
    }
    if shown < lines.len() {
        out.push_str(&format!("…({}줄 생략)\n", lines.len() - shown));
    }
    out.push('\n');
    out
}

/// Directory tree lines: single-child chains without own files collapse into
/// one `a/b/c/` line; children sorted by size, ≤12 per level, depth ≤4.
fn tree_lines(dirs: &HashMap<String, DirStat>, key: &str, label: &str, depth: usize, out: &mut Vec<String>) {
    let mut key = key.to_string();
    let mut label = label.to_string();
    while let Some(st) = dirs.get(&key) {
        if st.direct == 0 && st.children.len() == 1 {
            let child = st.children[0].clone();
            label.push_str(child.rsplit('/').next().unwrap_or(&child));
            label.push('/');
            key = child;
        } else {
            break;
        }
    }
    let Some(st) = dirs.get(&key) else { return };
    let mut exts: Vec<(&String, &usize)> = st.exts.iter().collect();
    exts.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let ext_s: Vec<String> = exts.iter().take(2).map(|(e, c)| format!("{e} {c}")).collect();
    let next_depth = if label.is_empty() {
        depth
    } else {
        out.push(format!(
            "{}{label}  (파일 {}{})",
            "  ".repeat(depth),
            st.total,
            if ext_s.is_empty() { String::new() } else { format!(" · {}", ext_s.join(", ")) }
        ));
        depth + 1
    };
    if next_depth > 4 {
        return;
    }
    let mut kids: Vec<&String> = st.children.iter().collect();
    kids.sort_by(|a, b| dirs[*b].total.cmp(&dirs[*a].total).then(a.cmp(b)));
    for (i, k) in kids.iter().enumerate() {
        if i >= 12 {
            out.push(format!("{}…(폴더 {}개 더)", "  ".repeat(next_depth), kids.len() - 12));
            break;
        }
        let name = k.rsplit('/').next().unwrap_or(k);
        tree_lines(dirs, k, &format!("{name}/"), next_depth, out);
    }
}

const STACK_HINTS: &[(&str, &str)] = &[
    ("spring-boot", "Spring Boot"),
    ("spring-webmvc", "Spring MVC"),
    ("egovframe", "eGovFrame"),
    ("egovframework", "eGovFrame"),
    ("mybatis", "MyBatis"),
    ("ibatis", "iBATIS"),
    ("struts", "Struts"),
    ("hibernate", "Hibernate/JPA"),
    ("spring-data-jpa", "Hibernate/JPA"),
    ("spring-batch", "Spring Batch"),
    ("quartz", "Quartz"),
    ("tiles", "Tiles"),
    ("lombok", "Lombok"),
    ("ojdbc", "Oracle JDBC"),
    ("tibero", "Tibero JDBC"),
];

fn add_hints(stack: &mut Vec<&'static str>, text: &str) {
    for (hint, label) in STACK_HINTS {
        if text.contains(hint) && !stack.contains(label) {
            stack.push(label);
        }
    }
}

/// The interface's implementation (`XImpl`, or a class `implements X`).
fn resolve_impl(decls: &HashMap<String, TypeDecl>, t: &str) -> String {
    match decls.get(t) {
        Some(d) if d.kind == "interface" => {
            let named = format!("{t}Impl");
            if decls.contains_key(&named) {
                return named;
            }
            let mut found: Vec<&String> = decls.iter().filter(|(_, x)| x.implements.iter().any(|i| i == t)).map(|(n, _)| n).collect();
            found.sort();
            found.first().map(|n| (*n).clone()).unwrap_or_else(|| t.to_string())
        }
        _ => t.to_string(),
    }
}

fn stmt_summary(stmts: &[StmtRef], idx: &[usize]) -> String {
    let mut parts: Vec<String> = idx
        .iter()
        .take(4)
        .map(|&i| {
            let s = &stmts[i];
            let t: Vec<String> = s.tables.iter().take(3).map(|(n, c)| format!("{n} {c}")).collect();
            if t.is_empty() { s.id.clone() } else { format!("{} [{}]", s.id, t.join(", ")) }
        })
        .collect();
    if idx.len() > 4 {
        parts.push(format!("외 {}개", idx.len() - 4));
    }
    parts.join(", ")
}

struct ChainCtx<'a> {
    decls: &'a HashMap<String, TypeDecl>,
    java: &'a [JavaFile],
    stmts: &'a [StmtRef],
    by_ns: HashMap<String, Vec<usize>>,
    by_key: HashMap<String, usize>,
}

impl ChainCtx<'_> {
    /// Follow injected collaborators from `name`: service (→ impl) → DAO /
    /// mapper; a mapper interface links to its XML statements by namespace, a
    /// DAO by the `"namespace.id"` strings it passes.
    fn walk(&self, name: &str, depth: usize, seen: &mut HashSet<String>, out: &mut Vec<String>) {
        if depth > 4 || !seen.insert(name.to_string()) {
            return;
        }
        let Some(d) = self.decls.get(name) else { return };
        let indent = "  ".repeat(depth);
        for f in d.fields.iter().filter(|f| self.decls.contains_key(*f)).take(5) {
            let target = resolve_impl(self.decls, f);
            let label = if &target == f { f.clone() } else { format!("{f} → {target}") };
            let mut hits: Vec<usize> = self.by_ns.get(f).cloned().unwrap_or_default();
            if let Some(td) = self.decls.get(&target) {
                for s in &self.java[td.file].strings {
                    if let Some(&i) = self.by_key.get(s) {
                        if !hits.contains(&i) {
                            hits.push(i);
                        }
                    }
                }
            }
            if hits.is_empty() {
                out.push(format!("{indent}→ {label}"));
                self.walk(&target, depth + 1, seen, out);
            } else {
                out.push(format!(
                    "{indent}→ {label} ⇢ {}: {}",
                    self.stmts[hits[0]].rel.rsplit('/').next().unwrap_or(""),
                    stmt_summary(self.stmts, &hits)
                ));
            }
        }
    }
}

/// Build the repository map of `root` as Markdown text (≈`opts.budget`
/// chars): tech stack, directory tree, request-related hits, entry points/
/// routes, dependency chains down to SQL tables, hub types, table CRUD and the
/// outlines of a few key files. Deterministic, no model involved.
pub fn repo_map(root: &Path, opts: &MapOpts, cancel: &AtomicBool) -> String {
    let started = Instant::now();
    let mut files: Vec<(String, u64)> = Vec::new();
    let stop = walk_files_with(root, cancel, WALK_MAX, &|n| MAP_SKIP_DIRS.contains(&n), &mut |p, size| {
        if let Ok(rel) = p.strip_prefix(root) {
            files.push((rel.to_string_lossy().replace('\\', "/"), size));
        }
        started.elapsed() < opts.deadline
    });
    let mut partial: Vec<String> = Vec::new();
    match stop {
        WalkStop::Limit => partial.push(format!("탐색 항목 상한 {WALK_MAX}")),
        WalkStop::Visitor => partial.push("시간 상한(파일 목록)".into()),
        WalkStop::Canceled => partial.push("취소됨".into()),
        WalkStop::Done => {}
    }

    // ── Directory stats + stack hints ────────────────────────────────────────
    let mut dirs: HashMap<String, DirStat> = HashMap::new();
    dirs.entry(String::new()).or_default();
    let mut stack: Vec<&'static str> = Vec::new();
    let mut build_files: Vec<String> = Vec::new();
    let mut jsp = 0usize;
    for (rel, _) in &files {
        let name = rel.rsplit('/').next().unwrap_or(rel).to_lowercase();
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default();
        if ext == "jsp" {
            jsp += 1;
        }
        if matches!(name.as_str(), "pom.xml" | "build.gradle" | "build.gradle.kts" | "build.xml" | "web.xml") {
            build_files.push(rel.clone());
        }
        if ext == "jar" {
            add_hints(&mut stack, &name);
        }
        let parts: Vec<&str> = rel.split('/').collect();
        let mut parent = String::new();
        for (i, part) in parts.iter().enumerate() {
            let st = dirs.entry(parent.clone()).or_default();
            st.total += 1;
            if !ext.is_empty() {
                *st.exts.entry(ext.clone()).or_default() += 1;
            }
            if i + 1 == parts.len() {
                st.direct += 1;
                break;
            }
            let child = if parent.is_empty() { part.to_string() } else { format!("{parent}/{part}") };
            if !st.children.contains(&child) {
                st.children.push(child.clone());
            }
            parent = child;
        }
    }
    for bf in &build_files {
        if let Ok(bytes) = fs::read(root.join(bf)) {
            add_hints(&mut stack, &String::from_utf8_lossy(&bytes[..bytes.len().min(512 * 1024)]).to_lowercase());
        }
    }

    // ── Parse sources ────────────────────────────────────────────────────────
    let mut java: Vec<JavaFile> = Vec::new();
    let mut stmts: Vec<StmtRef> = Vec::new();
    let mut xml_entries: Vec<String> = Vec::new();
    let mut table_acc: Vec<(String, String, usize)> = Vec::new();
    let mut parsed = 0usize;
    for (rel, size) in &files {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let lang = lang_for(Path::new(name));
        if *size > OUTLINE_FILE_MAX || is_secret_name(name) || !matches!(lang, Lang::Java | Lang::Xml | Lang::Sql) {
            continue;
        }
        if parsed >= opts.max_parsed || started.elapsed() >= opts.deadline || cancel.load(Ordering::Relaxed) {
            partial.push(format!("분석 파일·시간 상한 — {parsed}개까지 분석"));
            break;
        }
        let Ok(bytes) = fs::read(root.join(rel)) else { continue };
        if is_binary(&bytes) {
            continue;
        }
        parsed += 1;
        let o = outline(lang, &bytes);
        match lang {
            Lang::Java => {
                let tops: Vec<&OutlineItem> =
                    o.items.iter().filter(|i| i.depth == 0 && matches!(i.kind, "class" | "interface" | "enum" | "record")).collect();
                let types: Vec<String> = tops.iter().filter_map(|i| type_name_of(&i.signature)).collect();
                let dao = tops.iter().any(|i| type_name_of(&i.signature).is_some_and(|n| is_dao_like(&n, &i.signature)));
                let strings = if dao { java_strings(&bytes) } else { Vec::new() };
                java.push(JavaFile { rel: rel.clone(), outline: o, types, strings });
            }
            Lang::Xml => {
                let ns = o.notes.first().and_then(|n| n.split("namespace=").nth(1)).unwrap_or("").to_string();
                for it in &o.items {
                    if it.kind == "statement" {
                        let id = it.signature.split("id=").nth(1).and_then(|x| x.split_whitespace().next()).unwrap_or("?");
                        bump_tables(&mut table_acc, &it.tables);
                        stmts.push(StmtRef {
                            rel: rel.clone(),
                            ns: ns.clone(),
                            id: id.to_string(),
                            tables: it.tables.clone(),
                            line: it.line,
                            end_line: it.end_line,
                        });
                    } else if it.entry {
                        xml_entries.push(format!("{}  ({rel}:L{})", it.signature, it.line));
                    }
                }
            }
            _ => {
                for it in &o.items {
                    bump_tables(&mut table_acc, &it.tables);
                }
            }
        }
    }

    // ── Type index ───────────────────────────────────────────────────────────
    let mut decls: HashMap<String, TypeDecl> = HashMap::new();
    for (fi, jf) in java.iter().enumerate() {
        let mut current: Option<String> = None;
        for it in &jf.outline.items {
            if it.depth == 0 && matches!(it.kind, "class" | "interface" | "enum" | "record") {
                current = type_name_of(&it.signature);
                if let Some(name) = &current {
                    decls.entry(name.clone()).or_insert(TypeDecl {
                        file: fi,
                        kind: it.kind,
                        entry: it.entry,
                        implements: implemented(&it.signature),
                        fields: Vec::new(),
                    });
                }
            } else if it.depth == 1 && it.kind == "field" {
                if let (Some(c), Some(t)) = (current.as_ref(), field_type_of(&it.signature)) {
                    if let Some(d) = decls.get_mut(c) {
                        if d.file == fi && !d.fields.contains(&t) {
                            d.fields.push(t);
                        }
                    }
                }
            }
        }
    }

    // ── Entry points / routes ────────────────────────────────────────────────
    let mut routes: Vec<String> = Vec::new();
    for jf in &java {
        let mut base = String::new();
        let mut cls = String::new();
        for it in &jf.outline.items {
            if it.depth == 0 && matches!(it.kind, "class" | "interface") {
                cls = type_name_of(&it.signature).unwrap_or_default();
                base = mapping_of(&it.signature).map(|(_, p)| p).unwrap_or_default();
            } else if it.depth == 1 && it.entry && it.kind == "method" {
                let m = method_name_of(&it.signature);
                let at = format!("({}:L{})", jf.rel, it.line);
                if let Some((verb, p)) = mapping_of(&it.signature) {
                    routes.push(format!("{verb} {} → {cls}.{m}  {at}", join_paths(&base, &p)));
                } else if it.signature.contains("@Scheduled") {
                    routes.push(format!("SCHEDULED → {cls}.{m}  {at}"));
                } else if it.signature.contains("static void main(") {
                    routes.push(format!("MAIN → {cls}.{m}  {at}"));
                } else if it.signature.contains("ActionMapping") {
                    routes.push(format!("STRUTS → {cls}.{m}  {at}"));
                } else {
                    routes.push(format!("LISTENER → {cls}.{m}  {at}"));
                }
            }
        }
    }
    routes.extend(xml_entries);

    // ── Hubs: in-degree by simple name (imports + field types), X+XImpl merged ─
    let key_of = |n: &str| n.strip_suffix("Impl").unwrap_or(n).to_string();
    let mut indeg: HashMap<String, usize> = HashMap::new();
    for jf in &java {
        let mut refs: HashSet<String> = HashSet::new();
        for imp in &jf.outline.imports {
            let n = imp.trim_end_matches(';').rsplit('.').next().unwrap_or("");
            if decls.contains_key(n) {
                refs.insert(key_of(n));
            }
        }
        for t in &jf.types {
            if let Some(d) = decls.get(t) {
                refs.extend(d.fields.iter().filter(|f| decls.contains_key(*f)).map(|f| key_of(f)));
            }
        }
        for t in &jf.types {
            refs.remove(&key_of(t));
        }
        for r in refs {
            *indeg.entry(r).or_default() += 1;
        }
    }
    let mut hubs: Vec<(String, usize)> = indeg.into_iter().filter(|(_, c)| *c >= 2).collect();
    hubs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let hub_decl = |n: &str| decls.get(n).or_else(|| decls.get(&format!("{n}Impl")));
    let hub_lines: Vec<String> = hubs
        .iter()
        .take(15)
        .map(|(n, c)| {
            let imp = if decls.contains_key(&format!("{n}Impl")) && decls.contains_key(n) { "(+Impl)" } else { "" };
            let path = hub_decl(n).map(|d| java[d.file].rel.as_str()).unwrap_or("");
            format!("- {n}{imp} — {c}개 파일이 참조  ({path})")
        })
        .collect();

    // ── Focus terms ──────────────────────────────────────────────────────────
    let terms: Vec<String> = opts.focus_terms.iter().map(|t| t.trim().to_uppercase()).filter(|t| t.chars().count() >= 3).collect();
    let hit = |s: &str| {
        let u = s.to_uppercase();
        terms.iter().any(|t| u.contains(t.as_str()))
    };
    let mut focus_lines: Vec<String> = Vec::new();
    let mut focus_files: Vec<String> = Vec::new();
    let mut focus_stmts: Vec<usize> = Vec::new();
    if !terms.is_empty() {
        for (si, s) in stmts.iter().enumerate() {
            if hit(&s.id) || s.tables.iter().any(|(t, _)| hit(t)) {
                focus_stmts.push(si);
                let t: Vec<String> = s.tables.iter().take(4).map(|(n, c)| format!("{n} {c}")).collect();
                let key = if s.ns.is_empty() { s.id.clone() } else { format!("{}.{}", s.ns, s.id) };
                focus_lines.push(format!("- 문장 {key} [{}]  ({})", t.join(", "), s.rel));
            }
        }
        let mut names: Vec<&String> = decls.keys().filter(|n| hit(n)).collect();
        names.sort();
        for n in names {
            let d = &decls[n];
            focus_lines.push(format!("- {} {n}  ({})", d.kind, java[d.file].rel));
            focus_files.push(java[d.file].rel.clone());
        }
        focus_lines.extend(routes.iter().filter(|r| hit(r)).map(|r| format!("- 라우트 {r}")));
    }

    // ── Dependency chains ────────────────────────────────────────────────────
    let mut by_ns: HashMap<String, Vec<usize>> = HashMap::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for (i, s) in stmts.iter().enumerate() {
        if s.ns.is_empty() {
            continue;
        }
        by_ns.entry(s.ns.rsplit('.').next().unwrap_or(&s.ns).to_string()).or_default().push(i);
        by_key.insert(format!("{}.{}", s.ns, s.id), i);
    }
    let ctx = ChainCtx { decls: &decls, java: &java, stmts: &stmts, by_ns, by_key };
    let mut entries: Vec<&String> = decls.iter().filter(|(_, d)| d.entry).map(|(n, _)| n).collect();
    entries.sort_by_key(|n| (!hit(n) && !focus_files.contains(&java[decls[*n].file].rel), (*n).clone()));
    let mut chain_lines: Vec<String> = Vec::new();
    for n in entries.iter().take(10) {
        chain_lines.push(format!("▶ {n}  ({})", java[decls[*n].file].rel));
        ctx.walk(n, 1, &mut HashSet::new(), &mut chain_lines);
    }

    // ── Tables ───────────────────────────────────────────────────────────────
    table_acc.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    let table_lines: Vec<String> = table_acc.iter().take(25).map(|(t, c, n)| format!("- {t} {c} · 문장 {n}개")).collect();

    // ── Key file outlines: focus hits, entry points, then hubs ───────────────
    let mut key_files: Vec<String> = Vec::new();
    let candidates = focus_files
        .iter()
        .cloned()
        .chain(entries.iter().map(|n| java[decls[*n].file].rel.clone()))
        .chain(hubs.iter().filter_map(|(n, _)| hub_decl(n).map(|d| java[d.file].rel.clone())));
    let (key_max, bodies_max) = opts.detail.counts();
    for f in candidates {
        if !key_files.contains(&f) {
            key_files.push(f);
        }
        if key_files.len() >= key_max {
            break;
        }
    }

    // ── Render ───────────────────────────────────────────────────────────────
    let budget = opts.budget.max(2000);
    // Section weights are written at the 12k reference total (Extended's double
    // at its 24k budget).
    let [w_dir, w_focus, w_routes, w_chain, w_hubs, w_tables, w_outlines, w_bodies] = opts.detail.weights();
    let scale = |n: usize| n * budget / 12_000;
    let focus_only = opts.detail == MapDetail::Focus;
    let mut out = format!(
        "# 저장소 지도 — {}\n파일 {}개 (분석: Java {}개, 매퍼 SQL 문장 {}개){}\n\n",
        root.display(),
        files.len(),
        java.len(),
        stmts.len(),
        if partial.is_empty() { String::new() } else { format!(" — 부분 분석: {}", partial.join(", ")) }
    );
    let mut stack_parts: Vec<String> = stack.iter().map(|s| s.to_string()).collect();
    if jsp > 0 {
        stack_parts.push(format!("JSP 화면 {jsp}개"));
    }
    if !stack_parts.is_empty() || !build_files.is_empty() {
        let shown: Vec<&str> = build_files.iter().take(4).map(String::as_str).collect();
        out.push_str(&format!(
            "## 기술 스택\n- {}{}\n\n",
            if stack_parts.is_empty() { "(프레임워크 단서 없음)".to_string() } else { stack_parts.join(" · ") },
            if shown.is_empty() { String::new() } else { format!("  (빌드/설정: {})", shown.join(", ")) }
        ));
    }
    if !focus_only {
        let mut tl = Vec::new();
        tree_lines(&dirs, "", "", 0, &mut tl);
        out.push_str(&clip_section("디렉터리 (파일 수 · 주요 확장자)", &tl, scale(w_dir)));
    }
    out.push_str(&clip_section("요청 관련 (키워드 일치)", &focus_lines, scale(w_focus)));
    if !focus_only {
        out.push_str(&clip_section("진입점·라우트", &routes, scale(w_routes)));
    }
    out.push_str(&clip_section("의존 체인 (진입점 → 서비스 → DAO/Mapper ⇢ SQL 문장 [테이블 CRUD])", &chain_lines, scale(w_chain)));
    if !focus_only {
        out.push_str(&clip_section("허브 (프로젝트 안에서 가장 많이 참조되는 타입)", &hub_lines, scale(w_hubs)));
        out.push_str(&clip_section("테이블 (매퍼·SQL 기준 CRUD, 참조 문장 수)", &table_lines, scale(w_tables)));
    }
    // Bodies of the SQL statements the request names (D91) — what a FabriX
    // turn would otherwise read with one lookup round each.
    if bodies_max > 0 && !focus_stmts.is_empty() {
        let shown: Vec<&StmtRef> = focus_stmts.iter().take(bodies_max).map(|i| &stmts[*i]).collect();
        let per = scale(w_bodies) / shown.len();
        let mut section = String::from("## 요청 관련 SQL 문장 본문 (발췌 — 이미 첨부되었으니 다시 읽지 않아도 됨)\n");
        for st in &shown {
            if let Some(body) = stmt_body(root, st, per) {
                section.push_str(&body);
            }
        }
        if focus_stmts.len() > shown.len() {
            section.push_str(&format!("…(일치 문장 {}개 중 {}개만 첨부)\n", focus_stmts.len(), shown.len()));
        }
        out.push_str(&section);
        out.push('\n');
    }
    let outline_budget = scale(w_outlines) / key_files.len().max(1);
    let mut outlines = String::new();
    for f in &key_files {
        if let Some(jf) = java.iter().find(|j| &j.rel == f) {
            outlines.push_str(&render_outline(&jf.outline, f, outline_budget.max(400)));
            outlines.push('\n');
        }
    }
    if !outlines.is_empty() {
        out.push_str("## 핵심 파일 아웃라인\n");
        out.push_str(&outlines);
    }
    if out.chars().count() > budget {
        let kept: String = out.chars().take(budget).collect();
        out = format!("{kept}\n…(저장소 지도 잘림)\n");
    }
    out
}

/// One SQL statement's text for the map: a header with its key and range, then
/// up to `STMT_BODY_LINES` lines within `budget` characters. `None` when the
/// file can no longer be read.
fn stmt_body(root: &Path, st: &StmtRef, budget: usize) -> Option<String> {
    let bytes = fs::read(root.join(&st.rel)).ok()?;
    let text = lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let from = st.line.max(1);
    let to = st.end_line.unwrap_or(from).max(from).min(lines.len()).min(from + STMT_BODY_LINES - 1);
    if from > lines.len() {
        return None;
    }
    let key = if st.ns.is_empty() { st.id.clone() } else { format!("{}.{}", st.ns, st.id) };
    let mut out = format!("### {key}  ({}:L{from}-{to})\n```xml\n", st.rel);
    let mut used = 0usize;
    for line in &lines[from - 1..to] {
        let n = line.chars().count() + 1;
        if used + n > budget.max(300) {
            out.push_str("…\n");
            break;
        }
        out.push_str(line);
        out.push('\n');
        used += n;
    }
    let full_end = st.end_line.unwrap_or(to);
    out.push_str("```\n");
    if full_end > to {
        out.push_str(&format!("…(문장 나머지 L{}-{full_end})\n", to + 1));
    }
    Some(out)
}

/// Tauri command: the repository map of a codebase folder (D88), injected by
/// the codebase-step preflight for every agent (CLI agents can't call the
/// app's tools). `detail` = "extended" / "focus" renders the larger or the
/// request-only variant for FabriX (D91). Off the IPC thread; bounded by
/// `MapOpts` (≈6 s).
#[tauri::command]
pub async fn codebase_map(root: String, focus_terms: Option<Vec<String>>, detail: Option<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(root.trim());
        if !path.is_dir() {
            return Err(format!("코드베이스 폴더를 찾을 수 없습니다: {root}"));
        }
        let opts = MapOpts::for_detail(detail.as_deref(), focus_terms.unwrap_or_default());
        Ok(repo_map(path, &opts, &AtomicBool::new(false)))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// References / callers (D89): the first step of a blast-radius trace
// ---------------------------------------------------------------------------

const REF_MAX: usize = 200;
const REF_PER_FILE: usize = 20;
const REF_FILE_MAX: u64 = 1024 * 1024;

struct RefHit {
    line: usize,
    text: String,
    enclosing: Option<String>,
    definition: bool,
    in_string: bool,
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Byte offsets of whole-word occurrences of `needle` in `hay`.
fn word_hits(hay: &[u8], needle: &[u8], ignore_case: bool) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() || hay.len() < needle.len() {
        return out;
    }
    let eq = |a: &[u8]| if ignore_case { a.eq_ignore_ascii_case(needle) } else { a == needle };
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if eq(&hay[i..i + needle.len()]) {
            let before = i == 0 || !is_word_byte(hay[i - 1]);
            let after = i + needle.len() == hay.len() || !is_word_byte(hay[i + needle.len()]);
            if before && after {
                out.push(i);
                i += needle.len();
                continue;
            }
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Symbol lookup (D91): `read_file(symbol)` reads one declaration's body in the
// same call that names it, instead of outline → read-by-range (two rounds).
// ---------------------------------------------------------------------------

/// Words after which a name is being declared (`class X`, `TABLE T`, `bean b`…).
const DECL_KEYWORDS: &[&str] = &[
    "class", "interface", "enum", "record", "@interface", "table", "view", "procedure", "function", "package",
    "body", "trigger", "sequence", "synonym", "index", "bean", "servlet", "filter", "action",
];

/// How strongly `sig` declares `name`: 4/3 = declared (the name right before
/// `(`, after `id=` or after a declaring keyword — exact case / any case),
/// 2/1 = merely mentioned, 0 = absent. Non-identifier names (Korean headings)
/// match as a case-insensitive substring (1).
fn decl_score(sig: &str, name: &str) -> u8 {
    if !name.bytes().all(is_word_byte) {
        return u8::from(sig.to_lowercase().contains(&name.to_lowercase()));
    }
    let mut best = 0;
    for at in word_hits(sig.as_bytes(), name.as_bytes(), true) {
        let before = sig[..at].trim_end();
        let rest = sig[at + name.len()..].trim_start();
        let prev = before.rsplit(char::is_whitespace).next().unwrap_or("").to_ascii_lowercase();
        let declared = rest.starts_with('(') || before.ends_with("id=") || DECL_KEYWORDS.contains(&prev.as_str());
        let exact = &sig[at..at + name.len()] == name;
        best = best.max(match (declared, exact) {
            (true, true) => 4,
            (true, false) => 3,
            (false, true) => 2,
            (false, false) => 1,
        });
    }
    best
}

/// Indexes of the outline items that best match `name`, in file order (≤ `max`).
/// A qualifier (`Class.method`, `namespace.id`) and a trailing `()` are
/// dropped; declarations win over mere mentions.
pub fn symbol_items(o: &Outline, name: &str, max: usize) -> Vec<usize> {
    let name = name.trim().trim_end_matches("()");
    let bare = name.rsplit('.').next().unwrap_or("").trim();
    if bare.is_empty() {
        return Vec::new();
    }
    let scored: Vec<(u8, usize)> = o
        .items
        .iter()
        .enumerate()
        .filter(|(_, it)| it.kind != "more")
        .map(|(i, it)| (decl_score(&it.signature, bare), i))
        .filter(|(s, _)| *s > 0)
        .collect();
    let best = scored.iter().map(|(s, _)| *s).max().unwrap_or(0);
    scored.into_iter().filter(|(s, _)| *s == best).map(|(_, i)| i).take(max).collect()
}

/// The 1-based line span to show for item `idx` of a file with `lines`: from
/// its start — widened over the annotation/comment lines right above it — to
/// its end. A Markdown heading runs to the next heading of the same or a higher
/// level; an item without a known end is just its line.
pub fn item_span(o: &Outline, idx: usize, lines: &[&str]) -> (usize, usize) {
    let it = &o.items[idx];
    let total = lines.len().max(1);
    let mut start = it.line.clamp(1, total);
    for _ in 0..20 {
        if start <= 1 {
            break;
        }
        let t = lines.get(start - 2).map(|l| l.trim()).unwrap_or("");
        let lead = ["@", "/*", "*", "//", "--", "<!--"].iter().any(|p| t.starts_with(p));
        if !lead {
            break;
        }
        start -= 1;
    }
    let end = match it.end_line {
        Some(e) => e,
        None if it.kind == "heading" => o.items[idx + 1..]
            .iter()
            .find(|n| n.kind == "heading" && n.depth <= it.depth)
            .map(|n| n.line.saturating_sub(1))
            .unwrap_or(total),
        None => it.line,
    };
    (start, end.clamp(start, total))
}

/// The innermost outline item whose range covers `line`.
fn enclosing_item(o: &Outline, line: usize) -> Option<&OutlineItem> {
    o.items
        .iter()
        .filter(|it| it.kind != "more" && it.line <= line && it.end_line.unwrap_or(it.line) >= line)
        .max_by_key(|it| (it.depth, it.line))
}

/// Find the references of `symbol` under `roots` (folders or files), grouped
/// by file, each with the enclosing method/statement — the callers, i.e. the
/// first ring of the blast radius. `Class.method` narrows to files that
/// mention `Class` (plus mapper XMLs whose namespace ends with it); hits in
/// comments are dropped, hits inside string literals are flagged (eGov DAOs
/// call SQL statements by `"namespace.id"` strings). ALL-CAPS symbols (table
/// names) match case-insensitively.
pub fn find_references(
    roots: &[std::path::PathBuf],
    symbol: &str,
    name_filter: &dyn Fn(&str) -> bool,
    skip_file: &dyn Fn(&Path) -> bool,
    label: &dyn Fn(&Path) -> String,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let symbol = symbol.trim().trim_end_matches("()");
    if symbol.is_empty() {
        return Err("symbol 인자가 필요합니다 (예: OrderService.find, selectOrders, TB_ORDER).".into());
    }
    // `a.b.C.method` → qualifier `C`, name `method`; a full statement id
    // `ns.id` also stays searchable as a whole (string-literal calls).
    let (qual, name) = match symbol.rsplit_once('.') {
        Some((q, n)) if !n.is_empty() => (Some(q.rsplit('.').next().unwrap_or(q).to_string()), n.to_string()),
        _ => (None, symbol.to_string()),
    };
    let ignore_case = name.chars().any(|c| c.is_ascii_alphabetic()) && name == name.to_uppercase();
    let needle = name.as_bytes();

    let mut per_file: Vec<(String, Vec<RefHit>, usize)> = Vec::new(); // label, hits, total
    let mut total = 0usize;
    let mut scanned = 0usize;
    let mut capped = false;
    let mut seen_files: HashSet<std::path::PathBuf> = HashSet::new();
    let mut visit = |p: &Path, size: u64| -> bool {
        if size > REF_FILE_MAX || skip_file(p) || !seen_files.insert(p.to_path_buf()) {
            return true;
        }
        let fname = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !name_filter(&fname) {
            return true;
        }
        let Ok(bytes) = fs::read(p) else { return true };
        if is_binary(&bytes) {
            return true;
        }
        scanned += 1;
        let hits = word_hits(&bytes, needle, ignore_case);
        if hits.is_empty() {
            return true;
        }
        let lang = lang_for(p);
        if let Some(q) = &qual {
            let mentions = !word_hits(&bytes, q.as_bytes(), false).is_empty();
            let own = fname.split('.').next() == Some(q.as_str());
            if !mentions && !own {
                return true;
            }
        }
        let mask = (lang == Lang::Java).then(|| lex_java(&bytes).0);
        let outline_o = matches!(lang, Lang::Java | Lang::Xml | Lang::Sql).then(|| outline(lang, &bytes));
        let lines = Lines::new(&bytes);
        let mut file_hits: Vec<RefHit> = Vec::new();
        let mut file_total = 0usize;
        let mut last_line = 0usize;
        for pos in hits {
            let class = mask.as_ref().map(|m| m[pos]).unwrap_or(CODE);
            if class == COMMENT {
                continue;
            }
            let line = lines.of(pos);
            if line == last_line {
                continue; // one row per line
            }
            last_line = line;
            file_total += 1;
            total += 1;
            if file_hits.len() >= REF_PER_FILE {
                continue;
            }
            let start = lines.0[line - 1];
            let end = lines.0.get(line).copied().unwrap_or(bytes.len());
            let text = trunc(lossy(&bytes[start..end]).trim(), 160);
            let enc = outline_o.as_ref().and_then(|o| enclosing_item(o, line));
            let definition = enc.is_some_and(|it| {
                it.line == line && (it.signature.contains(&format!(" {name}(")) || it.signature.contains(&format!("id={name}")) || it.signature.ends_with(&format!(" {name}")))
            });
            file_hits.push(RefHit {
                line,
                text,
                enclosing: enc.filter(|_| !definition).map(|it| trunc(&it.signature, 90)),
                definition,
                in_string: class == STR,
            });
        }
        if file_total > 0 {
            per_file.push((label(p), file_hits, file_total));
        }
        if total >= REF_MAX {
            capped = true;
            return false;
        }
        true
    };
    let mut canceled = false;
    for r in roots {
        if r.is_file() {
            let size = fs::metadata(r).map(|m| m.len()).unwrap_or(0);
            if !visit(r, size) {
                break;
            }
        } else {
            match walk_files_with(r, cancel, WALK_MAX, &|_| false, &mut visit) {
                WalkStop::Canceled => {
                    canceled = true;
                    break;
                }
                WalkStop::Visitor => break,
                _ => {}
            }
        }
    }
    if canceled {
        return Err("취소되었습니다.".into());
    }
    if per_file.is_empty() {
        return Ok(format!("'{symbol}' 참조를 찾지 못했습니다 (파일 {scanned}개 확인). 철자·대소문자를 확인하거나 search_text로 넓게 찾아보세요.\n"));
    }
    per_file.sort_by(|a, b| a.0.cmp(&b.0));
    let defs = per_file.iter().flat_map(|(_, h, _)| h.iter()).filter(|h| h.definition).count();
    let strs = per_file.iter().flat_map(|(_, h, _)| h.iter()).filter(|h| h.in_string).count();
    let mut out = format!(
        "'{symbol}' 참조 — 파일 {}개 · 위치 {total}건 (정의 {defs}, 문자열 {strs}; 주석 제외){}\n",
        per_file.len(),
        if capped { format!(" — 상한 {REF_MAX}건에서 중단") } else { String::new() }
    );
    let radius: Vec<String> = per_file
        .iter()
        .map(|(l, h, n)| {
            let short = l.rsplit(['/', '\\']).next().unwrap_or(l);
            if h.iter().any(|x| x.definition) { format!("{short}({n}, 정의)") } else { format!("{short}({n})") }
        })
        .collect();
    out.push_str(&format!("영향 파일(Blast Radius 1단계): {}\n\n", radius.join(", ")));
    for (l, hits, n) in &per_file {
        out.push_str(l);
        out.push('\n');
        for h in hits {
            let tag = if h.definition {
                "[정의] ".to_string()
            } else {
                h.enclosing.as_ref().map(|e| format!("[{e}] ")).unwrap_or_default()
            };
            let s = if h.in_string { " (문자열)" } else { "" };
            out.push_str(&format!("  L{} {tag}{}{s}\n", h.line, h.text));
        }
        if *n > hits.len() {
            out.push_str(&format!("  …(이 파일 {}건 더)\n", n - hits.len()));
        }
    }
    out.push_str("재귀 추적: 위 [감싸는 메서드]의 이름으로 find_references를 다시 호출하면 다음 단계 호출자를 찾습니다.\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn java(src: &str) -> Outline {
        outline(Lang::Java, src.as_bytes())
    }

    fn find<'a>(o: &'a Outline, needle: &str) -> &'a OutlineItem {
        o.items
            .iter()
            .find(|i| i.signature.contains(needle))
            .unwrap_or_else(|| panic!("no item containing {needle:?} in {:#?}", o.items))
    }

    fn sigs(o: &Outline) -> Vec<String> {
        o.items.iter().map(|i| i.signature.clone()).collect()
    }

    #[test]
    fn java_spring_controller() {
        let src = r#"package com.sds.order.web;

import java.util.List;
import org.springframework.web.bind.annotation.*;

/**
 * 주문 API 컨트롤러. 주문 조회/등록을 담당한다.
 */
@RestController
@RequestMapping("/api/orders")
@RequiredArgsConstructor
public class OrderController {
    private final OrderService orderService;
    @Autowired private CodeService codeService;

    /** 주문 단건 조회. */
    @GetMapping("/{id}")
    public ResponseEntity<List<OrderDto>> get(
            @PathVariable("id") Long id,
            @RequestParam(required = false) final String type) throws NotFoundException {
        if (id == null) { throw new NotFoundException("x {"); }
        return ResponseEntity.ok(orderService.find(id, s -> { return s; }));
    }

    @PostMapping
    public void create(@RequestBody @Valid OrderDto dto) {
        orderService.create(dto);
    }
}
"#;
        let o = java(src);
        assert!(!o.degraded, "{o:#?}");
        assert_eq!(o.notes, vec!["package com.sds.order.web".to_string()]);
        assert_eq!(o.imports.len(), 2);
        let class = find(&o, "class OrderController");
        assert!(class.signature.starts_with("@RestController @RequestMapping(\"/api/orders\") public class OrderController"), "{}", class.signature);
        assert!(!class.signature.contains("RequiredArgsConstructor"));
        assert!(class.entry && class.kind == "class" && class.depth == 0);
        assert_eq!(class.line, 9);
        assert_eq!(class.end_line, Some(29));
        assert_eq!(class.doc.as_deref(), Some("주문 API 컨트롤러"));
        assert_eq!(find(&o, "orderService").kind, "field");
        assert_eq!(find(&o, "codeService").signature, "@Autowired private CodeService codeService");
        let get = find(&o, " get(");
        assert!(get.entry && get.kind == "method" && get.depth == 1);
        assert!(get.signature.contains("@PathVariable(\"id\") Long id"), "{}", get.signature);
        assert!(get.signature.contains("@RequestParam(required = false) String type"), "{}", get.signature);
        assert!(get.signature.contains("throws NotFoundException"));
        assert_eq!(get.doc.as_deref(), Some("주문 단건 조회"));
        assert_eq!((get.line, get.end_line), (17, Some(23)));
        let create = find(&o, "create(");
        assert!(create.signature.contains("@PostMapping public void create(@RequestBody OrderDto dto)"), "{}", create.signature);
        assert_eq!(o.items.len(), 5, "{:#?}", sigs(&o));
    }

    #[test]
    fn java_lexing_torture() {
        let src = "public class Torture {\n    static final String A = \"{\";\n    static final char B = '}';\n    static final char C = '\\'';\n    static final String D = \"//not comment\";\n    String url = \"http://x/*y*/\";\n    String block = \"\"\"\n        { \"a\": 1 } \\\"\"\" still\n        \"\"\";\n    /**/ int afterEmptyComment;\n    // } stray brace in comment\n    /* { */ void m() { String s = \"}\"; }\n    int last;\n}\n";
        let o = java(src);
        assert!(!o.degraded, "{o:#?}");
        for f in ["String A", "char B", "char C", "String D", "String url", "String block", "int afterEmptyComment", "int last"] {
            assert_eq!(find(&o, f).kind, "field", "{f}");
        }
        assert_eq!(find(&o, "void m()").kind, "method");
        assert_eq!(find(&o, "class Torture").end_line, Some(14));
        assert_eq!(o.items.len(), 10, "{:#?}", sigs(&o));
    }

    #[test]
    fn java_nesting_enums_initializers() {
        let src = r#"public class Outer {
    private Runnable r = new Runnable() {
        public void run() { System.out.println("in"); }
    };
    int[] arr = {1, 2, 3};
    static { init(); }
    { instanceInit(); }
    @SuppressWarnings({"a", "b"})
    public static class Inner {
        void innerMethod() {}
    }
    enum Color {
        RED, GREEN("g") {
            @Override String code() { return "G"; }
        }, BLUE;
        private final String c;
        Color() { this("x"); }
        Color(String c) { this.c = c; }
        String code() { return c; }
    }
    void outer() { Runnable x = () -> { int y = 1; }; class Local { void z() {} } }
}
"#;
        let o = java(src);
        assert!(!o.degraded, "{o:#?}");
        assert_eq!(find(&o, "Runnable r").signature, "private Runnable r");
        assert_eq!(find(&o, "arr").signature, "int[] arr");
        let inner = find(&o, "class Inner");
        assert_eq!((inner.depth, inner.signature.contains("SuppressWarnings")), (1, false));
        assert_eq!(find(&o, "innerMethod").depth, 2);
        let consts: Vec<&str> = o.items.iter().filter(|i| i.kind == "const").map(|i| i.signature.as_str()).collect();
        assert_eq!(consts, vec!["RED", "GREEN", "BLUE"]);
        assert_eq!(o.items.iter().filter(|i| i.kind == "ctor").count(), 2);
        assert_eq!(find(&o, "String c").kind, "field");
        assert!(o.items.iter().all(|i| !i.signature.contains("Local") && !i.signature.contains(" z(")));
        assert_eq!(find(&o, "void outer()").kind, "method");
    }

    #[test]
    fn java_generics_records_sealed_annotations() {
        let src = r#"public sealed interface Shape permits Circle, Square {
    double area();
    default String name() { return "shape"; }
}
public record Point<T extends Comparable<T>>(T x, T y) implements Serializable {
    public Point {
        Objects.requireNonNull(x);
    }
    static <K, V extends List<K>> Map<K, V> group(Map<K, V> in) { return in; }
}
public non-sealed class Circle implements Shape {
    public double area() { return 0; }
}
@interface Tag {
    String[] tags() default {};
    int value();
}
"#;
        let o = java(src);
        assert!(!o.degraded, "{o:#?}");
        assert_eq!(find(&o, "sealed interface Shape permits Circle, Square").kind, "interface");
        assert_eq!(find(&o, "double area()").kind, "method");
        assert_eq!(find(&o, "record Point").kind, "record");
        assert_eq!(find(&o, "public Point").kind, "ctor");
        let g = find(&o, "group(");
        assert_eq!(g.kind, "method");
        assert!(g.signature.contains("<K, V extends List<K>> Map<K, V> group(Map<K, V> in)"), "{}", g.signature);
        assert_eq!(find(&o, "non-sealed class Circle").kind, "class");
        assert_eq!(find(&o, "@interface Tag").kind, "annotation");
        assert!(find(&o, "tags()").signature.contains("default"));
        assert_eq!(find(&o, "int value()").kind, "method");
    }

    #[test]
    fn java_mapper_interface_keeps_param_bindings() {
        let o = java("@Mapper\npublic interface OrderMapper {\n    List<Order> selectOrders(@Param(\"custId\") String custId, @Param(\"from\") Date from);\n    int insertOrder(Order order);\n}\n");
        assert!(find(&o, "interface OrderMapper").signature.starts_with("@Mapper"));
        assert!(find(&o, "selectOrders").signature.contains("@Param(\"custId\") String custId"));
        assert_eq!(find(&o, "insertOrder").kind, "method");
    }

    #[test]
    fn java_egov_and_struts_entry_points() {
        let src = r#"@Service("orderService")
public class OrderServiceImpl extends EgovAbstractServiceImpl implements OrderService {
    @Resource(name = "orderDAO")
    private OrderDAO orderDAO;
}
public class OrderAction extends DispatchAction {
    public ActionForward list(ActionMapping mapping, ActionForm form, HttpServletRequest req, HttpServletResponse res) throws Exception { return null; }
}
"#;
        let o = java(src);
        let svc = find(&o, "class OrderServiceImpl");
        assert!(svc.signature.starts_with("@Service(\"orderService\")") && svc.signature.contains("extends EgovAbstractServiceImpl"));
        assert!(!svc.entry);
        assert_eq!(find(&o, "orderDAO").signature, "@Resource(name = \"orderDAO\") private OrderDAO orderDAO");
        assert!(find(&o, "class OrderAction").entry);
        assert!(find(&o, "list(").entry);
    }

    #[test]
    fn java_crlf_lines() {
        let o = java("class A {\r\n  void m() {\r\n  }\r\n}\r\n");
        assert_eq!((find(&o, "class A").line, find(&o, "class A").end_line), (1, Some(4)));
        assert_eq!((find(&o, "void m()").line, find(&o, "void m()").end_line), (2, Some(3)));
    }

    #[test]
    fn java_invalid_utf8_and_unbalanced_input() {
        let bad = b"/** \xff\xfe broken */\npublic class Bad { void m() {} }\n";
        let o = outline(Lang::Java, bad);
        assert_eq!(find(&o, "class Bad").doc, None);
        assert_eq!(o.items.len(), 2);
        let open = java("public class Broken {\n void m() {\n");
        assert!(open.degraded && open.items.len() == 2);
        let extra = java("}\n}\nclass X {}\n");
        assert!(extra.degraded);
        assert_eq!(find(&extra, "class X").kind, "class");
    }

    #[test]
    fn mybatis_mapper_statements_and_tables() {
        let src = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE mapper PUBLIC "-//mybatis.org//DTD Mapper 3.0//EN" "http://mybatis.org/dtd/mybatis-3-mapper.dtd">
<mapper namespace="com.sds.order.OrderMapper">
  <resultMap id="orderMap" type="com.sds.order.Order">
    <id property="id" column="ORDER_ID"/>
  </resultMap>
  <sql id="baseCols">o.ORDER_ID, o.CUST_ID</sql>
  <!-- <select id="oldSelect">SELECT * FROM OLD_TABLE</select> -->
  <select id="selectOrders" parameterType="map" resultMap="orderMap">
    SELECT <include refid="baseCols"/>, c.NAME
      FROM ORDERS o, CUSTOMER c
     WHERE o.CUST_ID = c.CUST_ID
     <if test="status != null and cnt > 0">AND o.STATUS = #{status}</if>
     AND o.ORDER_DT <![CDATA[ >= ]]> #{fromDt}
     AND EXTRACT(YEAR FROM o.ORDER_DT) = 2024
  </select>
  <select id="selectWithSub" resultType="int">
    WITH recent AS (SELECT * FROM ORDERS WHERE ROWNUM &lt; 10)
    SELECT COUNT(*) FROM recent r LEFT OUTER JOIN ORDER_ITEM i ON r.ORDER_ID = i.ORDER_ID
     WHERE r.X IN (SELECT 1 FROM DUAL)
  </select>
  <insert id="insertOrder" parameterType="Order">
    <selectKey keyProperty="id" resultType="long" order="BEFORE">SELECT SEQ_ORDER.NEXTVAL FROM SEQ_SOURCE</selectKey>
    INSERT INTO ORDERS (ORDER_ID, CUST_ID) VALUES (#{id}, #{custId})
  </insert>
  <update id="mergeOrder">
    MERGE INTO ORDER_SUMMARY s USING (SELECT CUST_ID FROM ORDERS) o ON (s.CUST_ID = o.CUST_ID)
    WHEN MATCHED THEN UPDATE SET s.CNT = s.CNT + 1
    WHEN NOT MATCHED THEN INSERT (CUST_ID, CNT) VALUES (o.CUST_ID, 1)
  </update>
  <delete id="deleteOrder">DELETE FROM SCOTT.ORDERS WHERE ORDER_ID = #{id}</delete>
</mapper>
"#;
        let o = outline(Lang::Xml, src.as_bytes());
        assert!(!o.degraded, "{o:#?}");
        assert_eq!(o.notes[0], "MyBatis mapper — namespace=com.sds.order.OrderMapper");
        assert!(o.items.iter().all(|i| !i.signature.contains("oldSelect")), "commented-out statement ignored");
        assert_eq!(find(&o, "resultMap id=orderMap").signature, "resultMap id=orderMap type=com.sds.order.Order");
        let sel = find(&o, "id=selectOrders");
        assert_eq!(sel.signature, "select id=selectOrders param=map result=orderMap");
        let doc = sel.doc.as_deref().unwrap();
        assert!(doc.contains("테이블: ORDERS(R), CUSTOMER(R)") && doc.contains("include: baseCols"), "{doc}");
        assert!(!doc.contains("ORDER_DT"), "EXTRACT(… FROM col) is not a table: {doc}");
        assert_eq!((sel.line, sel.end_line), (9, Some(16)));
        let sub = find(&o, "id=selectWithSub").doc.clone().unwrap();
        assert!(sub.contains("ORDERS(R)") && sub.contains("ORDER_ITEM(R)"), "{sub}");
        assert!(!sub.contains("RECENT") && !sub.contains("DUAL"), "{sub}");
        let ins = find(&o, "id=insertOrder").doc.clone().unwrap();
        assert_eq!(ins, "테이블: ORDERS(C)", "selectKey body is skipped");
        let merge = find(&o, "id=mergeOrder").doc.clone().unwrap();
        assert!(merge.contains("ORDER_SUMMARY(U)") && merge.contains("ORDERS(R)"), "{merge}");
        assert_eq!(find(&o, "id=deleteOrder").doc.as_deref(), Some("테이블: SCOTT.ORDERS(D)"));
        let crud = o.notes.iter().find(|n| n.starts_with("테이블 CRUD")).unwrap();
        assert!(crud.contains("ORDERS(CR)"), "{crud}");
    }

    #[test]
    fn ibatis_sqlmap() {
        let src = r#"<!DOCTYPE sqlMap PUBLIC "-//iBATIS.com//DTD SQL Map 2.0//EN" "http://ibatis.apache.org/dtd/sql-map-2.dtd">
<sqlMap namespace="Order">
  <select id="getOrder" parameterClass="string" resultClass="order">
    SELECT * FROM TB_ORDER WHERE ORDER_ID = #orderId#
    <dynamic prepend="AND"><isNotEmpty property="status">STATUS = #status:VARCHAR#</isNotEmpty></dynamic>
  </select>
  <procedure id="callCalc" parameterClass="map">{call PKG_ORDER.CALC(?, ?)}</procedure>
</sqlMap>
"#;
        let o = outline(Lang::Xml, src.as_bytes());
        assert_eq!(o.notes[0], "iBATIS sqlMap — namespace=Order");
        let get = find(&o, "id=getOrder");
        assert_eq!(get.signature, "select id=getOrder param=string result=order");
        assert_eq!(get.doc.as_deref(), Some("테이블: TB_ORDER(R)"));
        assert_eq!(find(&o, "id=callCalc").doc.as_deref(), Some("호출: PKG_ORDER.CALC"));
    }

    #[test]
    fn webxml_struts_and_beans() {
        let web = r#"<web-app>
  <context-param><param-name>contextConfigLocation</param-name><param-value>/WEB-INF/config/*.xml</param-value></context-param>
  <servlet><servlet-name>action</servlet-name><servlet-class>org.springframework.web.servlet.DispatcherServlet</servlet-class></servlet>
  <servlet-mapping><servlet-name>action</servlet-name><url-pattern>*.do</url-pattern></servlet-mapping>
  <filter><filter-name>enc</filter-name><filter-class>org.springframework.web.filter.CharacterEncodingFilter</filter-class></filter>
</web-app>"#;
        let o = outline(Lang::Xml, web.as_bytes());
        let s = find(&o, "servlet action");
        assert!(s.entry && s.signature.ends_with("DispatcherServlet"), "{}", s.signature);
        assert!(find(&o, "servlet-mapping action: *.do").entry);
        assert!(find(&o, "filter enc").signature.contains("CharacterEncodingFilter"));
        assert!(find(&o, "context-param contextConfigLocation").signature.contains("/WEB-INF/config/*.xml"));

        let struts = r#"<struts-config>
  <form-beans><form-bean name="orderForm" type="com.x.OrderForm"/></form-beans>
  <action-mappings>
    <action path="/order" type="com.x.OrderAction" name="orderForm" parameter="method">
      <forward name="list" path="/list.jsp"/>
    </action>
  </action-mappings>
</struts-config>"#;
        let o = outline(Lang::Xml, struts.as_bytes());
        let a = find(&o, "action path=/order");
        assert!(a.entry && a.signature.contains("→ com.x.OrderAction form=orderForm parameter=method"), "{}", a.signature);
        assert_eq!(find(&o, "form-bean orderForm").signature, "form-bean orderForm → com.x.OrderForm");

        let beans = r#"<beans xmlns="http://www.springframework.org/schema/beans">
  <context:component-scan base-package="com.sds"/>
  <bean id="orderService" class="com.x.OrderServiceImpl"><property name="dao" ref="orderDAO"/></bean>
</beans>"#;
        let o = outline(Lang::Xml, beans.as_bytes());
        assert!(o.notes.iter().any(|n| n == "component-scan base-package=com.sds"));
        assert_eq!(find(&o, "bean orderService").signature, "bean orderService → com.x.OrderServiceImpl");
    }

    #[test]
    fn xml_unclosed_and_unknown() {
        let o = outline(Lang::Xml, b"<mapper namespace=\"x\"><select id=\"a\">SELECT * FROM T_A");
        assert!(o.degraded);
        assert_eq!(find(&o, "id=a").doc.as_deref(), Some("테이블: T_A(R)"));
        let frag = outline(Lang::Xml, b"<select id=\"find\">SELECT * FROM ORDER_TABLE</select>\n");
        assert_eq!(frag.notes[0], "SQL 매퍼 조각(root=<select>)");
        assert_eq!(find(&frag, "select id=find").doc.as_deref(), Some("테이블: ORDER_TABLE(R)"));
        let u = outline(Lang::Xml, b"<root><item/><item/><cfg a='1'/></root>");
        assert!(u.approximate && u.notes[0].contains("item×2") && u.notes[0].contains("cfg×1"), "{:?}", u.notes);
    }

    #[test]
    fn sql_ddl_with_comments() {
        let src = "CREATE TABLE TB_ORDER (\n    ORDER_ID   NUMBER(10)     NOT NULL,\n    CUST_ID    VARCHAR2(20 BYTE),\n    STATUS     CHAR(1) DEFAULT 'N',\n    CONSTRAINT PK_TB_ORDER PRIMARY KEY (ORDER_ID)\n);\nCOMMENT ON TABLE TB_ORDER IS '주문';\nCOMMENT ON COLUMN TB_ORDER.ORDER_ID IS '주문번호';\nCREATE INDEX IX_ORDER_CUST ON TB_ORDER (CUST_ID);\nCREATE SEQUENCE SEQ_ORDER START WITH 1;\n";
        let o = outline(Lang::Sql, src.as_bytes());
        let t = find(&o, "TABLE TB_ORDER");
        assert_eq!(t.signature, "TABLE TB_ORDER (PK: ORDER_ID)");
        assert_eq!((t.doc.as_deref(), t.line, t.end_line), (Some("주문"), 1, Some(6)));
        let id = find(&o, "ORDER_ID NUMBER(10)");
        assert_eq!((id.kind, id.depth, id.doc.as_deref(), id.line), ("column", 1, Some("주문번호"), 2));
        assert_eq!(find(&o, "CUST_ID").signature, "CUST_ID VARCHAR2(20 BYTE)");
        assert_eq!(find(&o, "STATUS").signature, "STATUS CHAR(1)");
        assert!(o.items.iter().all(|i| !i.signature.starts_with("CONSTRAINT")));
        assert_eq!(find(&o, "INDEX IX_ORDER_CUST").signature, "INDEX IX_ORDER_CUST ON TB_ORDER(CUST_ID)");
        assert_eq!(find(&o, "SEQUENCE").signature, "SEQUENCE SEQ_ORDER");
    }

    #[test]
    fn plsql_package_spec_and_body() {
        let src = "CREATE OR REPLACE PACKAGE PKG_ORDER AS\n  PROCEDURE P_CREATE(p_id IN NUMBER, p_cust IN VARCHAR2);\n  FUNCTION F_COUNT(p_cust IN VARCHAR2) RETURN NUMBER;\nEND PKG_ORDER;\n/\nCREATE OR REPLACE PACKAGE BODY PKG_ORDER AS\n  PROCEDURE P_CREATE(p_id IN NUMBER, p_cust IN VARCHAR2) IS\n    v_cnt NUMBER;\n  BEGIN\n    SELECT COUNT(*) INTO v_cnt FROM TB_CUSTOMER WHERE CUST_ID = p_cust;\n    IF v_cnt = 0 THEN\n      RAISE_APPLICATION_ERROR(-20001, 'no; cust');\n    END IF;\n    FOR r IN (SELECT * FROM TB_ITEM) LOOP\n      INSERT INTO TB_ORDER (ORDER_ID, CUST_ID) VALUES (p_id, p_cust);\n    END LOOP;\n  END P_CREATE;\n  FUNCTION F_COUNT(p_cust IN VARCHAR2) RETURN NUMBER IS\n    v NUMBER;\n  BEGIN\n    SELECT COUNT(*) INTO v FROM TB_ORDER WHERE CUST_ID = p_cust;\n    RETURN v;\n  END F_COUNT;\nEND PKG_ORDER;\n/\n";
        let o = outline(Lang::Sql, src.as_bytes());
        let spec = &o.items[0];
        assert_eq!((spec.signature.as_str(), spec.end_line), ("PACKAGE PKG_ORDER", Some(4)));
        assert_eq!(o.items[1].signature, "PROCEDURE P_CREATE(p_id IN NUMBER, p_cust IN VARCHAR2)");
        assert_eq!(o.items[2].signature, "FUNCTION F_COUNT(p_cust IN VARCHAR2) RETURN NUMBER");
        assert_eq!(o.items[3].signature, "PACKAGE BODY PKG_ORDER");
        let p = &o.items[4];
        assert_eq!((p.line, p.end_line), (7, Some(17)));
        let doc = p.doc.as_deref().unwrap();
        assert_eq!(doc, "테이블: TB_CUSTOMER(R), TB_ITEM(R), TB_ORDER(C)");
        assert_eq!(o.items[5].doc.as_deref(), Some("테이블: TB_ORDER(R)"));
        assert_eq!(o.items.len(), 6, "{:#?}", sigs(&o));
        assert!(o.notes.iter().any(|n| n.contains("TB_ORDER(CR)")), "{:?}", o.notes);
    }

    #[test]
    fn sql_standalone_routines_trigger_view() {
        let src = "CREATE OR REPLACE PROCEDURE P_CLEAN(p_days IN NUMBER) AS\nBEGIN\n  DELETE FROM TB_LOG WHERE REG_DT < SYSDATE - p_days;\n  UPDATE TB_STAT SET CNT = 0;\nEND;\n/\nCREATE OR REPLACE TRIGGER TRG_ORDER_BI BEFORE INSERT ON TB_ORDER FOR EACH ROW\nBEGIN :NEW.REG_DT := SYSDATE; END;\n/\nCREATE OR REPLACE VIEW V_ORDER AS SELECT o.*, c.NAME FROM TB_ORDER o JOIN TB_CUSTOMER c ON o.CUST_ID = c.CUST_ID;\n";
        let o = outline(Lang::Sql, src.as_bytes());
        let p = find(&o, "PROCEDURE P_CLEAN");
        assert_eq!(p.signature, "PROCEDURE P_CLEAN(p_days IN NUMBER)");
        assert_eq!(p.doc.as_deref(), Some("테이블: TB_LOG(D), TB_STAT(U)"));
        assert_eq!(find(&o, "TRIGGER").signature, "TRIGGER TRG_ORDER_BI BEFORE INSERT ON TB_ORDER");
        assert_eq!(find(&o, "VIEW V_ORDER").doc.as_deref(), Some("참조: TB_ORDER(R), TB_CUSTOMER(R)"));
    }

    #[test]
    fn sql_lexing_strings_and_comments() {
        let src = "-- comment with ; and CREATE TABLE FAKE\nSELECT 'it''s; CREATE TABLE X' AS a, q'[a;b CREATE TABLE Y]' FROM \"Mixed Table\" t;\n";
        let o = outline(Lang::Sql, src.as_bytes());
        assert!(o.items.is_empty());
        assert!(o.notes[0].contains("CREATE) 없음"));
        assert_eq!(o.notes[1], "테이블: Mixed Table(R)");
    }

    #[test]
    fn sql_tables_patterns() {
        let t = sql_tables(
            "SELECT * FROM A, B b2 WHERE 1=1; UPDATE C SET x = (SELECT y FROM D); SELECT z FROM J FOR UPDATE; \
             INSERT INTO E SELECT * FROM F; TRUNCATE TABLE G; SELECT EXTRACT(YEAR FROM H.DT) FROM I",
        );
        let names: Vec<String> = t.iter().map(|(n, c)| format!("{n}:{c}")).collect();
        assert_eq!(names, vec!["A:R", "B:R", "C:U", "D:R", "J:R", "E:C", "F:R", "G:D", "I:R"]);
    }

    #[test]
    fn markdown_headings_skip_fences() {
        let o = outline(Lang::Markdown, b"# Title\ntext\n```\n# not a heading\n```\n## Sub\n");
        assert_eq!(sigs(&o), vec!["Title", "Sub"]);
        assert_eq!(o.items[1].depth, 1);
    }

    #[test]
    fn generic_fallback_is_marked_approximate() {
        let o = outline(Lang::Other, b"export function a() {\n  return 1;\n}\nclass B {}\n");
        assert!(o.approximate);
        assert_eq!(sigs(&o), vec!["export function a()", "class B {}"]);
    }

    #[test]
    fn render_shows_ranges_entry_marks_and_respects_budget() {
        let o = java("@RestController\npublic class C {\n  @GetMapping(\"/a\")\n  public String a() { return \"\"; }\n}\n");
        let r = render_outline(&o, "C.java", 4000);
        assert!(r.starts_with("C.java — Java 아웃라인(본문 제외): 항목 2개"), "{r}");
        assert!(r.contains("▶ L1-5 @RestController public class C"), "{r}");
        assert!(r.contains("  ▶ L3-4 @GetMapping(\"/a\") public String a()"), "{r}");
        assert!(r.contains("read_file(path, start_line="));
        let small = render_outline(&o, "C.java", 300);
        assert!(small.contains("출력 상한") || small.chars().count() <= 300, "{small}");
    }

    /// A small legacy-style repository: Spring controller → service (iface +
    /// impl) → MyBatis mapper interface + XML, and an eGov DAO calling an
    /// iBATIS statement by its "namespace.id" string; build output skipped.
    fn sample_repo(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("ow-codemap-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let w = |rel: &str, body: &str| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        };
        w("pom.xml", "<project><dependencies><dependency><artifactId>spring-webmvc</artifactId></dependency><dependency><groupId>egovframework.rte</groupId></dependency><dependency><artifactId>mybatis</artifactId></dependency></dependencies></project>");
        let j = "src/main/java/com/sds/order";
        w(&format!("{j}/web/OrderController.java"), "package com.sds.order.web;\nimport com.sds.order.service.OrderService;\nimport com.sds.order.common.CommonUtil;\n@RestController\n@RequestMapping(\"/api/orders\")\npublic class OrderController {\n    @Autowired private OrderService orderService;\n    @GetMapping(\"/{id}\")\n    public OrderDto get(@PathVariable Long id) { return orderService.find(id); }\n    @PostMapping\n    public void create(@RequestBody OrderDto dto) { orderService.create(dto); }\n}\n");
        w(&format!("{j}/service/OrderService.java"), "package com.sds.order.service;\npublic interface OrderService {\n    OrderDto find(Long id);\n    void create(OrderDto dto);\n}\n");
        w(&format!("{j}/service/impl/OrderServiceImpl.java"), "package com.sds.order.service.impl;\nimport com.sds.order.common.CommonUtil;\n@Service\npublic class OrderServiceImpl implements OrderService {\n    @Autowired private OrderMapper orderMapper;\n    @Resource(name = \"orderDAO\") private OrderDAO orderDAO;\n    public OrderDto find(Long id) { return orderMapper.selectOrder(id); }\n    public void create(OrderDto d) { orderDAO.insert(d); }\n}\n");
        w(&format!("{j}/mapper/OrderMapper.java"), "package com.sds.order.mapper;\n@Mapper\npublic interface OrderMapper {\n    OrderDto selectOrder(Long id);\n}\n");
        w(&format!("{j}/dao/OrderDAO.java"), "package com.sds.order.dao;\nimport com.sds.order.common.CommonUtil;\n@Repository(\"orderDAO\")\npublic class OrderDAO extends EgovAbstractDAO {\n    public void insert(OrderDto d) { insert(\"orderDAO.insertItem\", d); }\n}\n");
        w(&format!("{j}/common/CommonUtil.java"), "package com.sds.order.common;\npublic class CommonUtil {\n    public static String trim(String s) { return s; }\n}\n");
        w("src/main/resources/mapper/OrderMapper.xml", "<mapper namespace=\"com.sds.order.mapper.OrderMapper\">\n  <select id=\"selectOrder\" resultType=\"OrderDto\">SELECT * FROM TB_ORDER o JOIN TB_CUSTOMER c ON o.CUST_ID = c.CUST_ID WHERE o.ID = #{id}</select>\n</mapper>\n");
        w("src/main/resources/sqlmap/Order_SQL.xml", "<sqlMap namespace=\"orderDAO\">\n  <insert id=\"insertItem\">INSERT INTO TB_ORDER_ITEM (ID) VALUES (#id#)</insert>\n</sqlMap>\n");
        w("src/main/webapp/WEB-INF/web.xml", "<web-app><servlet-mapping><servlet-name>action</servlet-name><url-pattern>*.do</url-pattern></servlet-mapping></web-app>");
        w("src/main/webapp/WEB-INF/jsp/order/list.jsp", "<html/>");
        w("build/generated/Fake.java", "@RestController public class FakeController { @GetMapping(\"/fake\") public void f() {} }");
        w("target/classes/Gen.java", "@RestController public class GenController {}");
        w(".env", "SECRET=1");
        root
    }

    #[test]
    fn repo_map_orients_a_legacy_codebase() {
        let root = sample_repo("map");
        let opts = MapOpts { focus_terms: vec!["TB_ORDER_ITEM".into()], ..MapOpts::default() };
        let m = repo_map(&root, &opts, &AtomicBool::new(false));
        assert!(m.contains("# 저장소 지도"), "{m}");
        assert!(!m.contains("부분 분석"), "{m}");
        assert!(m.contains("Spring MVC") && m.contains("eGovFrame") && m.contains("MyBatis") && m.contains("JSP 화면 1개"), "{m}");
        assert!(m.contains("src/main/"), "single-child chains collapse: {m}");
        // Routes: class base path + method paths; build/target output is skipped.
        assert!(m.contains("GET /api/orders/{id} → OrderController.get"), "{m}");
        assert!(m.contains("POST /api/orders → OrderController.create"), "{m}");
        assert!(m.contains("servlet-mapping action: *.do"), "{m}");
        assert!(!m.contains("FakeController") && !m.contains("GenController"), "{m}");
        // Chain: controller → service iface → impl → mapper XML / DAO string id.
        assert!(m.contains("▶ OrderController"), "{m}");
        assert!(m.contains("→ OrderService → OrderServiceImpl"), "{m}");
        assert!(m.contains("→ OrderMapper ⇢ OrderMapper.xml: selectOrder [TB_ORDER R, TB_CUSTOMER R]"), "{m}");
        assert!(m.contains("→ OrderDAO ⇢ Order_SQL.xml: insertItem [TB_ORDER_ITEM C]"), "{m}");
        // Hubs, tables, focus hits, key outlines.
        assert!(m.contains("- CommonUtil — 3개 파일이 참조"), "{m}");
        assert!(m.contains("- TB_ORDER R · 문장 1개"), "{m}");
        assert!(m.contains("문장 orderDAO.insertItem [TB_ORDER_ITEM C]"), "{m}");
        assert!(m.contains("## 핵심 파일 아웃라인") && m.contains("OrderController.java — Java 아웃라인"), "{m}");
        assert!(!m.contains("SECRET"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn repo_map_detail_levels_for_fabrix() {
        let root = sample_repo("detail");
        let terms = vec!["TB_ORDER".to_string()];
        let std_map = repo_map(&root, &MapOpts::for_detail(None, terms.clone()), &AtomicBool::new(false));
        assert!(!std_map.contains("SQL 문장 본문"), "standard has no statement bodies");

        // Extended: statement bodies of the request-matching SQL + more key outlines.
        let ext = repo_map(&root, &MapOpts::for_detail(Some("extended"), terms.clone()), &AtomicBool::new(false));
        assert!(ext.contains("## 요청 관련 SQL 문장 본문"), "{ext}");
        assert!(ext.contains("### com.sds.order.mapper.OrderMapper.selectOrder  (src/main/resources/mapper/OrderMapper.xml:L2-2)"), "{ext}");
        assert!(ext.contains("SELECT * FROM TB_ORDER o JOIN TB_CUSTOMER"), "{ext}");
        assert!(ext.contains("### orderDAO.insertItem"), "TB_ORDER_ITEM contains the term: {ext}");
        // More key-file outlines when there are candidates (every Order* type hits).
        let outlines = |m: &str| m.matches("— Java 아웃라인").count();
        let wide = vec!["Order".to_string()];
        let std_wide = repo_map(&root, &MapOpts::for_detail(None, wide.clone()), &AtomicBool::new(false));
        let ext_wide = repo_map(&root, &MapOpts::for_detail(Some("extended"), wide), &AtomicBool::new(false));
        assert_eq!(outlines(&std_wide), 3, "{std_wide}");
        assert_eq!(outlines(&ext_wide), 6, "{ext_wide}");
        assert!(ext.contains("## 진입점·라우트") && ext.contains("## 허브"), "{ext}");
        assert!(ext.chars().count() <= 24_100);

        // Focus: request-specific sections only.
        let foc = repo_map(&root, &MapOpts::for_detail(Some("focus"), terms), &AtomicBool::new(false));
        assert!(foc.contains("## 요청 관련 (키워드 일치)") && foc.contains("## 의존 체인") && foc.contains("SQL 문장 본문"), "{foc}");
        assert!(!foc.contains("## 디렉터리") && !foc.contains("## 진입점·라우트") && !foc.contains("## 허브") && !foc.contains("## 테이블"), "{foc}");
        assert!(foc.chars().count() <= 12_100);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn repo_map_respects_limits_and_budget() {
        let root = sample_repo("limits");
        let opts = MapOpts { max_parsed: 1, budget: 2000, ..MapOpts::default() };
        let m = repo_map(&root, &opts, &AtomicBool::new(false));
        assert!(m.contains("부분 분석"), "{m}");
        assert!(m.chars().count() <= 2100, "{}", m.chars().count());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn signature_helpers() {
        assert_eq!(implemented("@Service public class A extends B implements C, D<E, F>"), vec!["C", "D"]);
        assert_eq!(implemented("public interface X extends Y, Z"), vec!["Y", "Z"]);
        assert_eq!(field_type_of("@Resource(name = \"x\") private List<Foo> foos").as_deref(), Some("List"));
        assert_eq!(field_type_of("private final OrderService orderService").as_deref(), Some("OrderService"));
        assert_eq!(field_type_of("int count"), None);
        assert_eq!(method_name_of("@GetMapping(\"/{id}\") public Dto get(@PathVariable Long id)"), "get");
        assert_eq!(mapping_of("@RequestMapping(value = \"/x\", method = RequestMethod.POST) public void a()"), Some(("POST".into(), "/x".into())));
        assert_eq!(mapping_of("@GetMapping public void b()"), Some(("GET".into(), String::new())));
        assert_eq!(join_paths("/api/", "/a"), "/api/a");
        assert_eq!(join_paths("", ""), "/");
    }

    fn refs(root: &Path, symbol: &str) -> String {
        find_references(&[root.to_path_buf()], symbol, &|_| true, &|_| false, &|p: &Path| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/"), &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn find_references_reports_callers_with_enclosing_symbols() {
        let root = sample_repo("refs");
        fs::write(
            root.join("src/main/java/com/sds/order/web/Legacy.java"),
            "package com.sds.order.web;\npublic class Legacy {\n    // orderService.find(1) — commented out\n    void old() { /* OrderService x; x.find(2); */ }\n}\n",
        )
        .unwrap();
        let r = refs(&root, "OrderService.find");
        assert!(r.contains("OrderController.java(1)"), "{r}");
        assert!(r.contains("L9 [@GetMapping(\"/{id}\") public OrderDto get(@PathVariable Long id)] public OrderDto get("), "{r}");
        assert!(r.contains("OrderService.java(1, 정의)") && r.contains("[정의] OrderDto find(Long id);"), "{r}");
        assert!(!r.contains("Legacy.java"), "comment-only hits are dropped: {r}");
        assert!(!r.contains("OrderMapper.java"), "files not mentioning the qualifier are skipped: {r}");

        // A statement id: the DAO's string call + the XML definition.
        let s = refs(&root, "insertItem");
        assert!(s.contains("OrderDAO.java") && s.contains("(문자열)"), "{s}");
        assert!(s.contains("Order_SQL.xml(1, 정의)"), "{s}");

        // ALL-CAPS table names match case-insensitively; none → a clear message.
        let t = refs(&root, "tb_order");
        assert!(t.contains("찾지 못했습니다"), "lower-case symbol stays case-sensitive: {t}");
        let t = refs(&root, "TB_ORDER");
        assert!(t.contains("OrderMapper.xml(1)"), "{t}");
        assert!(refs(&root, "nothingLikeThis").contains("찾지 못했습니다"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn word_hits_respect_boundaries() {
        assert_eq!(word_hits(b"find findAll refind find(", b"find", false), vec![0, 20]);
        assert_eq!(word_hits(b"FROM tb_order o", b"TB_ORDER", true), vec![5]);
    }

    #[test]
    fn lang_by_extension() {
        assert_eq!(lang_for(Path::new("A.java")), Lang::Java);
        assert_eq!(lang_for(Path::new("m/OrderMapper.XML")), Lang::Xml);
        assert_eq!(lang_for(Path::new("pkg.pkb")), Lang::Sql);
        assert_eq!(lang_for(Path::new("README.md")), Lang::Markdown);
        assert_eq!(lang_for(Path::new("x.ts")), Lang::Other);
    }

    #[test]
    fn symbol_items_prefer_declarations_and_spans_cover_bodies() {
        let src = "@Service\npublic class OrderService {\n  private OrderMapper orderMapper;\n\n  /**\n   * 주문 조회\n   */\n  @Transactional(readOnly = true)\n  public Order findOrder(Long id) {\n    return orderMapper.selectOrder(id);\n  }\n\n  public List<Order> findOrders(String cust) {\n    return orderMapper.selectOrders(cust);\n  }\n}\n";
        let o = java(src);
        let lines: Vec<&str> = src.lines().collect();
        // `Class.method` and a trailing `()` are tolerated; the declaration wins.
        let hits = symbol_items(&o, "OrderService.findOrder()", 3);
        assert_eq!(hits.len(), 1, "{:#?}", sigs(&o));
        assert!(o.items[hits[0]].signature.contains("findOrder(Long id)"));
        // The span is widened over the Javadoc above the annotation.
        assert_eq!(item_span(&o, hits[0], &lines), (5, 11));
        // A type name: the class declaration beats the mentions in signatures.
        let cls = symbol_items(&o, "OrderService", 3);
        assert_eq!(cls.len(), 1);
        assert_eq!(o.items[cls[0]].kind, "class");
        assert!(symbol_items(&o, "nothingHere", 3).is_empty());
        assert!(symbol_items(&o, " . ", 3).is_empty());

        // Mapper statements by id (namespace qualifier dropped).
        let xml = "<mapper namespace=\"com.x.OrderMapper\">\n  <!-- 주문 목록 -->\n  <select id=\"selectOrders\" resultType=\"Order\">\n    SELECT * FROM TB_ORDER\n    WHERE CUST_ID = #{cust}\n  </select>\n</mapper>\n";
        let xo = outline(Lang::Xml, xml.as_bytes());
        let xl: Vec<&str> = xml.lines().collect();
        let st = symbol_items(&xo, "com.x.OrderMapper.selectOrders", 3);
        assert_eq!(st.len(), 1, "{:#?}", sigs(&xo));
        assert_eq!(item_span(&xo, st[0], &xl), (2, 6));

        // SQL routines/tables are declared after their keyword (any case).
        let sql = "CREATE TABLE TB_ORDER (\n  ORDER_ID NUMBER(10)\n);\nCREATE OR REPLACE PROCEDURE P_CLEAN(p IN NUMBER) AS\nBEGIN\n  NULL;\nEND;\n/\n";
        let so = outline(Lang::Sql, sql.as_bytes());
        let t = symbol_items(&so, "tb_order", 3);
        assert_eq!(so.items[t[0]].kind, "table", "{:#?}", sigs(&so));
        assert!(so.items[symbol_items(&so, "P_CLEAN", 3)[0]].signature.starts_with("PROCEDURE"));

        // Markdown headings run to the next heading of the same level.
        let md = "# 계획\n## 범위\n내용 A\n### 세부\n내용 B\n## 일정\n내용 C\n";
        let mo = outline(Lang::Markdown, md.as_bytes());
        let ml: Vec<&str> = md.lines().collect();
        let h = symbol_items(&mo, "범위", 3);
        assert_eq!(item_span(&mo, h[0], &ml), (2, 5));
    }
}
