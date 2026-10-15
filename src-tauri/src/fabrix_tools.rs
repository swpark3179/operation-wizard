//! Read-only file tools for the FabriX tool-calling loop (D84).
//!
//! FabriX is an HTTP API with no filesystem of its own. The app exposes six
//! read-only tools over the OpenAI `tools` protocol so the model can actually
//! read the codebase (lifting the pre-D84 "remote agents can't read the
//! codebase" limitation, D67): `list_dir`, `find_files`, `search_text`,
//! `outline_file` (the skeleton-first view from `codemap.rs` — D87),
//! `find_references` (callers / first blast-radius ring — D89) and
//! `read_file`. There is deliberately no write/exec tool — document artifacts
//! are still persisted by the app (ChatPanel → `write_file`, D67).
//!
//! Every tool-loop round is one billable FabriX call against a per-minute
//! licence quota (D91), so the lookup tools also take **batches** —
//! `read_file.files`, `outline_file.paths`, `search_text.queries`,
//! `find_references.symbols` — and `read_file(symbol)` reads a declaration's
//! body directly (outline line range), letting one round do the work of many.
//!
//! Every path is confined to a sandbox: the run's `cwd` + `RunArgs.extra_dirs`
//! (codebase / skill folders / knowledge artifacts root). Roots and targets are
//! both `fs::canonicalize`d (Windows `\\?\` form, real casing, 8.3 short names
//! expanded) and compared with `Path::starts_with` (whole components), so `..`,
//! junctions and short-name aliases cannot escape. Walks skip symlinks/junctions
//! and the shared noise dirs (`files::SKIP_DIRS`). Text is decoded as lossy
//! UTF-8 (legacy EUC-KR sources may garble Korean — a documented limitation).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use serde_json::{json, Value};

use crate::codemap;
use crate::files::{is_binary, is_secret_name, walk_files, WalkStop, SKIP_DIRS};

/// Max characters of one tool result handed back to the model.
pub const RESULT_CHARS: usize = 15_000;
/// Max characters of one batched tool result (shared by its items, each item
/// at most `RESULT_CHARS` and at least `BATCH_ITEM_MIN`) — D91.
pub const BATCH_RESULT_CHARS: usize = 36_000;
const BATCH_ITEM_MIN: usize = 3_000;
/// Items one batched call processes (the rest are reported as skipped).
const READ_BATCH_MAX: usize = 8;
const OUTLINE_BATCH_MAX: usize = 10;
const SEARCH_BATCH_MAX: usize = 8;
const REF_BATCH_MAX: usize = 5;
/// Declarations one `read_file(symbol)` returns when the name is ambiguous.
const SYMBOL_MAX: usize = 3;
const LIST_MAX: usize = 500;
const FIND_MAX: usize = 200;
const SEARCH_MAX: usize = 200;
/// `search_text` skips files larger than this.
const SEARCH_FILE_MAX: u64 = 1024 * 1024;
/// `read_file` refuses files larger than this.
const READ_MAX_BYTES: u64 = 10 * 1024 * 1024;
/// Default (and maximum) number of lines one `read_file` call returns.
const READ_LINES: usize = 2000;
/// Max characters of one matched line shown by `search_text`.
const LINE_PREVIEW: usize = 300;

/// The OpenAI `tools` array advertised to the model (JSON Schema parameters).
pub fn tool_specs() -> Value {
    json!([
        { "type": "function", "function": {
            "name": "list_dir",
            "description": "폴더의 하위 항목(폴더/파일)을 나열합니다. List the entries of a directory.",
            "parameters": { "type": "object", "properties": {
                "path": { "type": "string", "description": "폴더 경로(절대경로 또는 작업 폴더 기준 상대경로). 생략하면 작업 폴더." }
            }, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "find_files",
            "description": "파일 이름 패턴으로 파일을 재귀 검색합니다(대소문자 무시). Find files by name recursively.",
            "parameters": { "type": "object", "properties": {
                "pattern": { "type": "string", "description": "파일명 와일드카드(* ?), 예: *Mapper.xml, *.java, Order*. 와일드카드가 없으면 이름에 그 문자열이 포함된 파일을 찾습니다." },
                "path": { "type": "string", "description": "검색할 폴더. 생략하면 작업 폴더." }
            }, "required": ["pattern"] }
        }},
        { "type": "function", "function": {
            "name": "search_text",
            "description": "파일 내용에서 문자열을 재귀 검색합니다(대소문자 무시, 정규식 아님). 결과 형식: '경로:줄번호: 내용'. 여러 검색어는 queries 배열로 한 번에 검색하세요(파일을 한 번만 훑음). Search file contents.",
            "parameters": { "type": "object", "properties": {
                "query": { "type": "string", "description": "찾을 문자열" },
                "queries": { "type": "array", "items": { "type": "string" }, "maxItems": SEARCH_BATCH_MAX,
                    "description": "여러 검색어를 한 번에(결과는 검색어별로 묶임). query 대신 사용." },
                "path": { "type": "string", "description": "검색할 폴더 또는 파일. 생략하면 작업 폴더." },
                "file_pattern": { "type": "string", "description": "파일명 와일드카드 필터, 예: *.xml" }
            }, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "outline_file",
            "description": "소스 파일의 스켈레톤(클래스·메서드·필드 선언부, 핵심 어노테이션, Javadoc 첫 문장, 줄 범위 — 본문 제외)을 보여줍니다. Java·MyBatis/iBATIS 매퍼 XML(문장 id·참조 테이블 CRUD)·web.xml·struts-config·Spring XML·SQL/PL-SQL(테이블 컬럼·패키지 멤버)·Markdown 지원, 그 외는 근사. 파일 본문을 읽기 전에 먼저 쓰고, 여러 파일은 paths 배열로 한 번에 보세요. 본문은 read_file의 symbol(이름) 또는 start_line/max_lines로 필요한 부분만 읽습니다. Outline source files (signatures without bodies).",
            "parameters": { "type": "object", "properties": {
                "path": { "type": "string", "description": "파일 경로(절대경로 또는 허용 폴더 기준 상대경로)" },
                "paths": { "type": "array", "items": { "type": "string" }, "maxItems": OUTLINE_BATCH_MAX,
                    "description": "여러 파일을 한 번에. path 대신 사용." }
            }, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "find_references",
            "description": "심볼(클래스·메서드·필드·테이블명·매퍼 문장 id)을 참조하는 위치를 파일별로 찾고, 각 위치를 감싸는 메서드/SQL 문장과 정의 여부를 표시합니다(호출자·영향 범위 파악). 주석 속 일치는 제외, 문자열 속 일치는 '(문자열)'로 표시. 'Class.method' 형식이면 Class를 언급하는 파일로 좁힙니다. 대문자 심볼(테이블명)은 대소문자 무시. 여러 심볼은 symbols 배열로 한 번에. Find references / callers of symbols.",
            "parameters": { "type": "object", "properties": {
                "symbol": { "type": "string", "description": "찾을 심볼, 예: OrderService.findOrder, selectOrders, TB_ORDER" },
                "symbols": { "type": "array", "items": { "type": "string" }, "maxItems": REF_BATCH_MAX,
                    "description": "여러 심볼을 한 번에. symbol 대신 사용." },
                "path": { "type": "string", "description": "검색할 폴더 또는 파일. 생략하면 모든 허용 폴더." },
                "file_pattern": { "type": "string", "description": "파일명 와일드카드 필터, 예: *.java" }
            }, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "read_file",
            "description": "텍스트 파일을 줄번호와 함께 읽습니다. symbol을 주면 그 메서드·생성자·클래스·매퍼 문장 id·SQL 프로시저/테이블·Markdown 헤딩의 본문만 읽고, 아니면 start_line/max_lines 범위를 읽습니다. 여러 파일·범위·심볼은 files 배열로 한 번에 읽으세요(호출 횟수 절약). Read text files, whole declarations by name, or line ranges.",
            "parameters": { "type": "object", "properties": {
                "path": { "type": "string", "description": "파일 경로(절대경로 또는 작업 폴더 기준 상대경로)" },
                "symbol": { "type": "string", "description": "본문을 읽을 선언 이름, 예: findOrder, OrderService.findOrder, selectOrders, P_CLEAN" },
                "start_line": { "type": "integer", "description": "시작 줄(1부터). 기본 1." },
                "max_lines": { "type": "integer", "description": "읽을 최대 줄 수. 기본·최대 2000." },
                "files": { "type": "array", "maxItems": READ_BATCH_MAX,
                    "description": "여러 파일/범위/심볼을 한 번에 읽기. 각 항목은 {path, symbol?, start_line?, max_lines?}. path 대신 사용.",
                    "items": { "type": "object", "properties": {
                        "path": { "type": "string" },
                        "symbol": { "type": "string" },
                        "start_line": { "type": "integer" },
                        "max_lines": { "type": "integer" }
                    }, "required": ["path"] } }
            }, "required": [] }
        }}
    ])
}

/// Result of one tool call: the text handed back to the model + an error flag
/// (shown as an error row in the chat timeline).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutcome {
    fn ok(content: String) -> Self {
        Self { content, is_error: false }
    }
    fn err(content: String) -> Self {
        Self { content, is_error: true }
    }
}

/// Strip the Windows verbatim prefix (`\\?\C:\…` → `C:\…`, `\\?\UNC\srv\share`
/// → `\\srv\share`) so paths shown to the model are ones it can pass back.
pub fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
}

/// Truncate to `max` characters (not bytes), appending a Korean note with the
/// original length when cut.
pub fn clip_chars(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max).collect();
    format!("{kept}\n…(잘림: 전체 {total}자 중 {max}자)")
}

/// Case-insensitive wildcard match (`*` = any run, `?` = one char). A pattern
/// without wildcards matches when the name *contains* it.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.trim().to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    if p.is_empty() {
        return true;
    }
    if !p.iter().any(|c| *c == '*' || *c == '?') {
        let needle: String = p.iter().collect();
        let hay: String = t.iter().collect();
        return hay.contains(&needle);
    }
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// The set of folders one run may read. Built once per run from `cwd` +
/// `extra_dirs`; folders that don't exist are dropped.
pub struct Sandbox {
    /// Canonical roots, de-duplicated; the cwd (if it exists) comes first.
    roots: Vec<PathBuf>,
    /// Canonical cwd — the base for relative paths (never the process cwd).
    base: Option<PathBuf>,
    /// Canonical app home (`~/.operation-wizard`) whose `settings.json*` holds
    /// the API keys — never readable even when a root is very broad.
    deny_dir: Option<PathBuf>,
}

impl Sandbox {
    pub fn new(cwd: &str, extra_dirs: &[String]) -> Self {
        let canon = |s: &str| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            fs::canonicalize(s).ok().filter(|p| p.is_dir())
        };
        let base = canon(cwd);
        let mut roots: Vec<PathBuf> = Vec::new();
        for p in base.iter().cloned().chain(extra_dirs.iter().filter_map(|d| canon(d))) {
            if !roots.contains(&p) {
                roots.push(p);
            }
        }
        let deny_dir = crate::ow_home().ok().and_then(|h| fs::canonicalize(h).ok());
        Self { roots, base, deny_dir }
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// The canonical roots (cwd first).
    pub(crate) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Display form of the roots (for the system prompt and error messages).
    pub fn root_labels(&self) -> Vec<String> {
        self.roots.iter().map(|r| display_path(r)).collect()
    }

    /// Files the model must never read: `.env*` secrets and the app's own
    /// `settings.json*` (it stores the FabriX/RAG keys in plain text).
    fn denied(&self, p: &Path) -> bool {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if is_secret_name(&name) {
            return true;
        }
        if let Some(home) = &self.deny_dir {
            if p.parent() == Some(home.as_path()) && name.starts_with("settings.json") {
                return true;
            }
        }
        false
    }

    /// Resolve a model-supplied path (absolute, or relative to the cwd root —
    /// falling back to the other roots in order when it doesn't exist there, so
    /// a codebase-relative path from an outline resolves too; `None`/blank →
    /// the cwd root) to a canonical path inside a root.
    fn resolve(&self, raw: Option<&str>) -> Result<PathBuf, String> {
        let base = self.base.as_ref().or(self.roots.first());
        let target = match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None => base.cloned().ok_or("작업 폴더가 없습니다 — 경로를 지정하세요.")?,
            Some(p) => {
                let p = Path::new(p);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    let first = base
                        .ok_or("작업 폴더가 없어 상대경로를 해석할 수 없습니다 — 절대경로를 쓰세요.")?
                        .join(p);
                    if first.exists() {
                        first
                    } else {
                        self.roots.iter().map(|r| r.join(p)).find(|c| c.exists()).unwrap_or(first)
                    }
                }
            }
        };
        let canon = fs::canonicalize(&target)
            .map_err(|_| format!("경로를 찾을 수 없습니다: {}", display_path(&target)))?;
        if !self.roots.iter().any(|r| canon.starts_with(r)) {
            return Err(format!(
                "허용된 폴더 밖의 경로입니다: {}\n허용 폴더: {}",
                display_path(&canon),
                self.root_labels().join(", ")
            ));
        }
        if self.denied(&canon) {
            return Err(format!("보안상 읽을 수 없는 파일입니다: {}", display_path(&canon)));
        }
        Ok(canon)
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

/// An integer argument; accepts a JSON number or a numeric string (models
/// sometimes quote numbers).
fn int_arg(args: &Value, key: &str) -> Option<usize> {
    let v = args.get(key)?;
    v.as_u64()
        .map(|n| n as usize)
        .or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as usize))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<usize>().ok()))
}

/// Run one tool call. Never panics; every failure becomes an error outcome the
/// model can read and react to (the run itself keeps going).
pub fn run_tool(name: &str, args: &Value, sb: &Sandbox, cancel: &AtomicBool) -> ToolOutcome {
    // `read_file` also accepts `paths` (an alias models reach for).
    let batch_keys: &[&str] = match name {
        "search_text" => &["queries"],
        "outline_file" => &["paths"],
        "find_references" => &["symbols"],
        "read_file" => &["files", "paths"],
        _ => &[],
    };
    let batched = batch_keys.iter().any(|k| list_arg(args, k).is_some_and(|items| items.len() > 1));
    let cap = if batched { BATCH_RESULT_CHARS } else { RESULT_CHARS };
    let res = match name {
        "list_dir" => list_dir(sb, args),
        "find_files" => find_files(sb, args, cancel),
        "search_text" => search_text(sb, args, cancel),
        "outline_file" => outline_file(sb, args, cancel),
        "find_references" => find_references(sb, args, cancel),
        "read_file" => read_file(sb, args, cancel),
        other => Err(format!(
            "'{other}' 도구는 없습니다. 사용할 수 있는 도구는 list_dir, find_files, search_text, outline_file, find_references, read_file(모두 읽기 전용)뿐입니다. \
             파일 저장은 앱이 처리하므로, 문서를 작성하는 단계라면 최종 답변에 문서 본문만 출력하세요."
        )),
    };
    match res {
        Ok(text) => ToolOutcome::ok(clip_chars(&text, cap)),
        Err(e) => ToolOutcome::err(clip_chars(&e, cap)),
    }
}

/// A batch argument (`files`/`paths`/`queries`/`symbols`) as a non-empty
/// array. Tolerates a JSON-encoded array string, which models sometimes send.
fn list_arg(args: &Value, key: &str) -> Option<Vec<Value>> {
    let v = args.get(key)?;
    let arr = match v {
        Value::Array(a) => a.clone(),
        Value::String(s) if s.trim_start().starts_with('[') => serde_json::from_str::<Vec<Value>>(s).ok()?,
        _ => return None,
    };
    Some(arr).filter(|a| !a.is_empty())
}

/// The non-blank strings of a batch argument.
fn str_items(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Per-item character cap of a batch of `n`.
fn batch_item_cap(n: usize) -> usize {
    (BATCH_RESULT_CHARS / n.max(1)).clamp(BATCH_ITEM_MIN, RESULT_CHARS)
}

/// Run `one` for each item (at most `max`), sharing the batch cap. One failing
/// item never fails the batch (its error is shown in place); the call is an
/// error only when every item failed.
fn run_batch<T>(
    items: &[T],
    max: usize,
    label: impl Fn(&T) -> String,
    one: impl Fn(&T, usize) -> Result<String, String>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let shown = &items[..items.len().min(max)];
    let cap = batch_item_cap(shown.len());
    let mut out = String::new();
    let mut failed = 0usize;
    for (i, it) in shown.iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("취소되었습니다.".to_string());
        }
        out.push_str(&format!("### [{}/{}] {}\n", i + 1, shown.len(), label(it)));
        match one(it, cap) {
            Ok(t) => out.push_str(&clip_chars(t.trim_end(), cap)),
            Err(e) => {
                failed += 1;
                out.push_str(&format!("오류: {e}"));
            }
        }
        out.push_str("\n\n");
    }
    if items.len() > shown.len() {
        out.push_str(&format!(
            "…(요청 {}개 중 앞의 {}개만 처리했습니다 — 나머지는 다음 호출에서 요청하세요)\n",
            items.len(),
            shown.len()
        ));
    }
    if failed == shown.len() {
        Err(out)
    } else {
        Ok(out)
    }
}

fn list_dir(sb: &Sandbox, args: &Value) -> Result<String, String> {
    let dir = sb.resolve(str_arg(args, "path"))?;
    if !dir.is_dir() {
        return Err(format!("폴더가 아닙니다: {} — 파일은 read_file로 읽으세요.", display_path(&dir)));
    }
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<(String, u64)> = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if ft.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                dirs.push(name);
            }
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            files.push((name, size));
        }
    }
    dirs.sort_by_key(|n| n.to_lowercase());
    files.sort_by_key(|(n, _)| n.to_lowercase());

    let mut out = format!("{} — 폴더 {}개, 파일 {}개\n", display_path(&dir), dirs.len(), files.len());
    let total = dirs.len() + files.len();
    let mut shown = 0usize;
    for d in &dirs {
        if shown >= LIST_MAX {
            break;
        }
        out.push_str(&format!("{d}/\n"));
        shown += 1;
    }
    for (f, size) in &files {
        if shown >= LIST_MAX {
            break;
        }
        out.push_str(&format!("{f}  ({size} bytes)\n"));
        shown += 1;
    }
    if total > shown {
        out.push_str(&format!("…(이하 {}개 생략)\n", total - shown));
    }
    Ok(out)
}

/// Trailing note for a walk that did not finish normally.
fn walk_note(stop: &WalkStop, cap: usize) -> Result<&'static str, String> {
    match stop {
        WalkStop::Done => Ok(""),
        WalkStop::Visitor => Ok(if cap == FIND_MAX {
            "…(최대 200건까지만 표시 — 패턴이나 경로를 좁혀 다시 검색하세요)\n"
        } else {
            "…(최대 200건까지만 표시 — 검색어·file_pattern·경로를 좁혀 다시 검색하세요)\n"
        }),
        WalkStop::Limit => Ok("…(탐색 항목 상한 도달 — 하위 폴더를 지정해 다시 검색하세요)\n"),
        WalkStop::Canceled => Err("취소되었습니다.".to_string()),
    }
}

fn find_files(sb: &Sandbox, args: &Value, cancel: &AtomicBool) -> Result<String, String> {
    let pattern = str_arg(args, "pattern").ok_or("pattern 인자가 필요합니다 (예: *Mapper.xml).")?;
    let root = sb.resolve(str_arg(args, "path"))?;
    if !root.is_dir() {
        return Err(format!("폴더가 아닙니다: {}", display_path(&root)));
    }
    let mut hits: Vec<String> = Vec::new();
    let stop = walk_files(&root, cancel, &mut |p, _| {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if wildcard_match(pattern, &name) {
            hits.push(display_path(p));
        }
        hits.len() < FIND_MAX
    });
    let note = walk_note(&stop, FIND_MAX)?;
    if hits.is_empty() {
        return Ok(format!("'{pattern}'와 일치하는 파일이 없습니다 ({}).\n{note}", display_path(&root)));
    }
    let mut out = format!("'{pattern}' 파일명 검색 — {}건 ({})\n", hits.len(), display_path(&root));
    for h in &hits {
        out.push_str(h);
        out.push('\n');
    }
    out.push_str(note);
    Ok(out)
}

fn search_text(sb: &Sandbox, args: &Value, cancel: &AtomicBool) -> Result<String, String> {
    let mut queries: Vec<String> = list_arg(args, "queries").map(|a| str_items(&a)).unwrap_or_default();
    if queries.is_empty() {
        queries.extend(str_arg(args, "query").map(str::to_string));
    }
    if queries.is_empty() {
        return Err("query 인자가 필요합니다 (여러 검색어는 queries 배열).".to_string());
    }
    let requested = queries.len();
    queries.truncate(SEARCH_BATCH_MAX);
    let needles: Vec<String> = queries.iter().map(|q| q.to_lowercase()).collect();
    let target = sb.resolve(str_arg(args, "path"))?;
    let file_pattern = str_arg(args, "file_pattern");
    // One walk for every query; each query keeps its own hit list and cap.
    let per_query = if needles.len() == 1 { SEARCH_MAX } else { (SEARCH_MAX / needles.len()).max(25) };

    let mut hits: Vec<Vec<String>> = vec![Vec::new(); needles.len()];
    let mut scanned = 0usize;
    let mut scan = |p: &Path, size: u64| -> bool {
        if size > SEARCH_FILE_MAX || sb.denied(p) {
            return true;
        }
        if let Some(fp) = file_pattern {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !wildcard_match(fp, &name) {
                return true;
            }
        }
        let Ok(bytes) = fs::read(p) else { return true };
        if is_binary(&bytes) {
            return true;
        }
        scanned += 1;
        let text = String::from_utf8_lossy(&bytes);
        for (i, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            for (q, needle) in needles.iter().enumerate() {
                if hits[q].len() < per_query && lower.contains(needle.as_str()) {
                    let shown: String = line.trim().chars().take(LINE_PREVIEW).collect();
                    hits[q].push(format!("{}:{}: {}", display_path(p), i + 1, shown));
                }
            }
            if hits.iter().all(|h| h.len() >= per_query) {
                return false;
            }
        }
        true
    };

    let stop = if target.is_file() {
        let size = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        if scan(&target, size) {
            WalkStop::Done
        } else {
            WalkStop::Visitor
        }
    } else {
        walk_files(&target, cancel, &mut scan)
    };
    let where_ = display_path(&target);

    if needles.len() == 1 {
        let query = &queries[0];
        let note = walk_note(&stop, SEARCH_MAX)?;
        let hits = &hits[0];
        if hits.is_empty() {
            return Ok(format!("'{query}' 검색 결과가 없습니다 (파일 {scanned}개 확인, {where_}).\n{note}"));
        }
        let mut out = format!("'{query}' 내용 검색 — {}건 (파일 {scanned}개 확인, {where_})\n", hits.len());
        for h in hits {
            out.push_str(h);
            out.push('\n');
        }
        out.push_str(note);
        return Ok(out);
    }

    let note = match stop {
        WalkStop::Canceled => return Err("취소되었습니다.".to_string()),
        WalkStop::Limit => "…(탐색 항목 상한 도달 — 하위 폴더를 지정해 다시 검색하세요)\n",
        _ => "",
    };
    let mut out = format!("검색어 {}개 내용 검색 (파일 {scanned}개 확인, {where_}, 검색어별 최대 {per_query}건)\n", queries.len());
    for (q, query) in queries.iter().enumerate() {
        let h = &hits[q];
        out.push_str(&format!("\n### '{query}' — {}건{}\n", h.len(), if h.len() >= per_query { " (상한 — 검색어·경로를 좁히세요)" } else { "" }));
        for line in h {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(note);
    if requested > queries.len() {
        out.push_str(&format!("…(검색어 {requested}개 중 앞의 {}개만 검색했습니다)\n", queries.len()));
    }
    Ok(out)
}

fn outline_file(sb: &Sandbox, args: &Value, cancel: &AtomicBool) -> Result<String, String> {
    if let Some(items) = list_arg(args, "paths") {
        let paths = str_items(&items);
        if paths.is_empty() {
            return Err("paths에는 파일 경로 문자열 배열을 넣으세요.".to_string());
        }
        return run_batch(&paths, OUTLINE_BATCH_MAX, |p| p.clone(), |p, cap| outline_one(sb, p, cap), cancel);
    }
    let path = str_arg(args, "path").ok_or("path 인자가 필요합니다 (여러 파일은 paths 배열).")?;
    outline_one(sb, path, RESULT_CHARS)
}

fn outline_one(sb: &Sandbox, raw: &str, cap: usize) -> Result<String, String> {
    let path = sb.resolve(Some(raw))?;
    if !path.is_file() {
        return Err(format!(
            "파일이 아닙니다: {} — 폴더 구조는 list_dir·find_files로 보세요.",
            display_path(&path)
        ));
    }
    let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if len > codemap::OUTLINE_FILE_MAX {
        return Err(format!(
            "파일이 너무 커서 아웃라인을 만들지 않습니다({len} bytes): {} — search_text로 위치를 찾고 read_file로 범위만 읽으세요.",
            display_path(&path)
        ));
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    if is_binary(&bytes) {
        return Err(format!("바이너리 파일은 아웃라인을 만들 수 없습니다: {}", display_path(&path)));
    }
    let o = codemap::outline(codemap::lang_for(&path), &bytes);
    Ok(codemap::render_outline(&o, &display_path(&path), cap.saturating_sub(200)))
}

fn find_references(sb: &Sandbox, args: &Value, cancel: &AtomicBool) -> Result<String, String> {
    let roots: Vec<PathBuf> = match str_arg(args, "path") {
        Some(p) => vec![sb.resolve(Some(p))?],
        None => sb.roots().to_vec(),
    };
    let pattern = str_arg(args, "file_pattern");
    let one = |symbol: &str| {
        codemap::find_references(
            &roots,
            symbol,
            &|name: &str| pattern.map(|fp| wildcard_match(fp, name)).unwrap_or(true),
            &|p: &Path| sb.denied(p),
            &|p: &Path| display_path(p),
            cancel,
        )
    };
    if let Some(items) = list_arg(args, "symbols") {
        let symbols = str_items(&items);
        if symbols.is_empty() {
            return Err("symbols에는 심볼 문자열 배열을 넣으세요.".to_string());
        }
        return run_batch(&symbols, REF_BATCH_MAX, |s| format!("'{s}' 참조"), |s, _| one(s), cancel);
    }
    let symbol = str_arg(args, "symbol")
        .ok_or("symbol 인자가 필요합니다 (예: OrderService.findOrder, selectOrders, TB_ORDER; 여러 개는 symbols 배열).")?;
    one(symbol)
}

/// One `read_file` request: a path plus either a declaration name or a range.
struct ReadSpec {
    path: String,
    symbol: Option<String>,
    start: Option<usize>,
    max: Option<usize>,
}

impl ReadSpec {
    /// From the call's own arguments, or a `files` item (an object, or a bare
    /// path string).
    fn from_value(v: &Value) -> Option<Self> {
        if let Some(p) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return Some(Self { path: p.to_string(), symbol: None, start: None, max: None });
        }
        Some(Self {
            path: str_arg(v, "path")?.to_string(),
            symbol: str_arg(v, "symbol").map(str::to_string),
            start: int_arg(v, "start_line"),
            max: int_arg(v, "max_lines"),
        })
    }

    fn label(&self) -> String {
        match &self.symbol {
            Some(sym) => format!("{} · {sym}", self.path),
            None => match self.start {
                Some(st) => format!("{} · {st}줄부터", self.path),
                None => self.path.clone(),
            },
        }
    }
}

fn read_file(sb: &Sandbox, args: &Value, cancel: &AtomicBool) -> Result<String, String> {
    if let Some(items) = list_arg(args, "files").or_else(|| list_arg(args, "paths")) {
        let specs: Vec<ReadSpec> = items.iter().filter_map(ReadSpec::from_value).collect();
        if specs.is_empty() {
            return Err("files의 각 항목에는 path가 필요합니다 (예: {\"path\": \"…\", \"symbol\": \"findOrder\"}).".to_string());
        }
        return run_batch(&specs, READ_BATCH_MAX, ReadSpec::label, |sp, cap| read_one(sb, sp, cap), cancel);
    }
    let spec = ReadSpec::from_value(args).ok_or("path 인자가 필요합니다 (여러 파일은 files 배열).")?;
    read_one(sb, &spec, RESULT_CHARS)
}

/// Numbered lines `from..=to` (1-based) within `budget` characters; returns the
/// text and the last line included (at least one line is always included).
fn numbered(lines: &[&str], from: usize, to: usize, budget: usize) -> (String, usize) {
    let mut body = String::new();
    let mut used = 0usize;
    let mut last = from - 1;
    for (i, line) in lines.iter().enumerate().take(to).skip(from - 1) {
        let row = format!("{:>5}\t{}\n", i + 1, line);
        let n = row.chars().count();
        if used + n > budget && last >= from {
            break;
        }
        body.push_str(&row);
        used += n;
        last = i + 1;
    }
    (body, last)
}

fn read_one(sb: &Sandbox, spec: &ReadSpec, cap: usize) -> Result<String, String> {
    let path = sb.resolve(Some(&spec.path))?;
    if !path.is_file() {
        return Err(format!("파일이 아닙니다: {} — 폴더는 list_dir로 보세요.", display_path(&path)));
    }
    let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if len > READ_MAX_BYTES {
        return Err(format!("파일이 너무 큽니다({len} bytes): {} — search_text로 필요한 부분을 찾으세요.", display_path(&path)));
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    if is_binary(&bytes) {
        return Err(format!("바이너리 파일은 읽을 수 없습니다: {}", display_path(&path)));
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 {
        return Ok(format!("{} — 빈 파일입니다.", display_path(&path)));
    }
    // Leave room for the header/footer inside the cap.
    let budget = cap.saturating_sub(400);
    if let Some(symbol) = &spec.symbol {
        return read_symbol(&path, &bytes, &lines, symbol, budget);
    }

    let start = spec.start.unwrap_or(1).max(1);
    let max = spec.max.unwrap_or(READ_LINES).clamp(1, READ_LINES);
    if start > total {
        return Err(format!("start_line {start}이(가) 파일 줄 수({total})보다 큽니다: {}", display_path(&path)));
    }
    let (body, last) = numbered(&lines, start, (start - 1 + max).min(total), budget);
    let mut out = format!("{} — 전체 {total}줄, {start}–{last}줄\n", display_path(&path));
    out.push_str(&body);
    if last < total {
        out.push_str(&format!(
            "…(계속 읽으려면 read_file(path, start_line={}))\n",
            last + 1
        ));
    }
    Ok(out)
}

/// `read_file(symbol)`: the body of the declaration(s) named `symbol`, located
/// with the outline's line ranges (`codemap::symbol_items` / `item_span`).
fn read_symbol(path: &Path, bytes: &[u8], lines: &[&str], symbol: &str, budget: usize) -> Result<String, String> {
    let label = display_path(path);
    if bytes.len() as u64 > codemap::OUTLINE_FILE_MAX {
        return Err(format!(
            "파일이 너무 커서 심볼 위치를 찾지 않습니다: {label} — search_text로 위치를 찾고 start_line/max_lines로 읽으세요."
        ));
    }
    let o = codemap::outline(codemap::lang_for(path), bytes);
    let found = codemap::symbol_items(&o, symbol, SYMBOL_MAX);
    if found.is_empty() {
        let names: Vec<String> = o
            .items
            .iter()
            .filter(|it| it.kind != "more" && it.kind != "field" && it.kind != "column")
            .take(15)
            .map(|it| format!("L{} {}", it.line, it.signature.chars().take(80).collect::<String>()))
            .collect();
        let mut e = format!("'{symbol}' 선언을 찾지 못했습니다: {label}");
        if !names.is_empty() {
            e.push_str(&format!("\n선언 목록(일부):\n{}", names.join("\n")));
        }
        e.push_str("\n이름을 고쳐 다시 요청하거나 start_line/max_lines로 읽으세요.");
        return Err(e);
    }
    let total = lines.len();
    let mut out = format!("{label} — '{symbol}' 선언 {}곳 (전체 {total}줄)\n", found.len());
    let per = budget / found.len();
    for idx in found {
        let (from, to) = codemap::item_span(&o, idx, lines);
        let (body, last) = numbered(lines, from, to, per.max(400));
        out.push_str(&format!("[L{from}-{to}] {}\n", o.items[idx].signature));
        out.push_str(&body);
        if last < to {
            out.push_str(&format!("…(이 선언의 나머지는 read_file(path, start_line={}, max_lines={}))\n", last + 1, to - last));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh temp tree:
    /// root/
    ///   README.md, Order.java, data.bin (binary), .env
    ///   src/OrderMapper.xml, src/util/Helper.java
    ///   node_modules/pkg/index.js (skipped)
    fn tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ow-fx-tools-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src").join("util")).unwrap();
        fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
        fs::write(root.join("README.md"), "# 주문 시스템\nORDER_TABLE 설명\n").unwrap();
        fs::write(root.join("Order.java"), "class Order {\n  // order_table\n}\n").unwrap();
        fs::write(root.join("data.bin"), [0u8, 1, 2, 3, b'o', b'r', b'd', b'e', b'r']).unwrap();
        fs::write(root.join(".env"), "SECRET=1\nORDER_TABLE=x\n").unwrap();
        fs::write(
            root.join("src").join("OrderMapper.xml"),
            "<select id=\"find\">SELECT * FROM ORDER_TABLE</select>\n",
        )
        .unwrap();
        fs::write(root.join("src").join("util").join("Helper.java"), "class Helper {}\n").unwrap();
        fs::write(root.join("node_modules").join("pkg").join("index.js"), "ORDER_TABLE\n").unwrap();
        root
    }

    fn sb(root: &Path) -> Sandbox {
        Sandbox::new(&root.to_string_lossy(), &[])
    }

    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn wildcard_matching() {
        assert!(wildcard_match("*Mapper.xml", "OrderMapper.xml"));
        assert!(wildcard_match("*mapper.XML", "OrderMapper.xml"), "case-insensitive");
        assert!(wildcard_match("Order*", "Order.java"));
        assert!(wildcard_match("?rder.java", "Order.java"));
        assert!(!wildcard_match("*.xml", "Order.java"));
        assert!(wildcard_match("mapper", "OrderMapper.xml"), "no wildcard → contains");
        assert!(wildcard_match("*", "anything"));
        assert!(!wildcard_match("a*b*c", "aXbY"));
        assert!(wildcard_match("a*b*c", "aXbYc"));
    }

    #[test]
    fn display_path_strips_verbatim_prefix() {
        assert_eq!(display_path(Path::new(r"\\?\C:\work\a.txt")), r"C:\work\a.txt");
        assert_eq!(display_path(Path::new(r"\\?\UNC\srv\share\a")), r"\\srv\share\a");
        assert_eq!(display_path(Path::new(r"C:\plain")), r"C:\plain");
    }

    #[test]
    fn clip_chars_counts_characters() {
        assert_eq!(clip_chars("가나다", 3), "가나다");
        let c = clip_chars("가나다라", 2);
        assert!(c.starts_with("가나\n"));
        assert!(c.contains("전체 4자 중 2자"));
    }

    #[test]
    fn sandbox_resolves_inside_and_rejects_escapes() {
        let root = tree("sandbox");
        let s = sb(&root);
        assert!(!s.is_empty());
        // Relative to the cwd root, and blank → the root itself.
        assert!(s.resolve(Some("src/OrderMapper.xml")).is_ok());
        assert_eq!(s.resolve(None).unwrap(), fs::canonicalize(&root).unwrap());
        // `..` escape is caught after canonicalization.
        let parent_escape = s.resolve(Some("..")).unwrap_err();
        assert!(parent_escape.contains("허용된 폴더 밖"), "{parent_escape}");
        // An absolute path outside every root is refused.
        let outside = std::env::temp_dir();
        assert!(s.resolve(Some(&outside.to_string_lossy())).unwrap_err().contains("허용된 폴더 밖"));
        // Missing path → a clear error, not a panic.
        assert!(s.resolve(Some("nope/missing.txt")).unwrap_err().contains("찾을 수 없습니다"));
        // Secrets are never readable.
        assert!(s.resolve(Some(".env")).unwrap_err().contains("보안상"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sandbox_extra_dirs_become_roots() {
        let a = tree("extra-a");
        let b = tree("extra-b");
        let s = Sandbox::new(&a.to_string_lossy(), &[b.to_string_lossy().into_owned(), "  ".into(), "Z:\\definitely\\missing".into()]);
        assert_eq!(s.root_labels().len(), 2, "blank / missing extra dirs are dropped");
        let in_b = b.join("README.md");
        assert!(s.resolve(Some(&in_b.to_string_lossy())).is_ok());
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    #[test]
    fn list_dir_skips_noise_dirs() {
        let root = tree("list");
        let out = run_tool("list_dir", &json!({}), &sb(&root), &no_cancel());
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("src/"));
        assert!(out.content.contains("README.md"));
        assert!(!out.content.contains("node_modules"), "noise dir hidden");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn find_files_matches_names_recursively() {
        let root = tree("find");
        let out = run_tool("find_files", &json!({ "pattern": "*.java" }), &sb(&root), &no_cancel());
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("Order.java"));
        assert!(out.content.contains("Helper.java"));
        assert!(!out.content.contains("index.js"));
        let none = run_tool("find_files", &json!({ "pattern": "*.cs" }), &sb(&root), &no_cancel());
        assert!(!none.is_error && none.content.contains("없습니다"));
        let missing = run_tool("find_files", &json!({}), &sb(&root), &no_cancel());
        assert!(missing.is_error);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn search_text_skips_binary_env_and_noise() {
        let root = tree("search");
        let out = run_tool("search_text", &json!({ "query": "order_table" }), &sb(&root), &no_cancel());
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("README.md:2:"), "{}", out.content);
        assert!(out.content.contains("Order.java:2:"));
        assert!(out.content.contains("OrderMapper.xml:1:"));
        assert!(!out.content.contains(".env"), "secret file never searched");
        assert!(!out.content.contains("index.js"), "noise dir skipped");
        assert!(!out.content.contains("data.bin"), "binary skipped");
        // file_pattern filter.
        let xml = run_tool(
            "search_text",
            &json!({ "query": "ORDER_TABLE", "file_pattern": "*.xml" }),
            &sb(&root),
            &no_cancel(),
        );
        assert!(xml.content.contains("OrderMapper.xml") && !xml.content.contains("README.md"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_file_numbers_lines_and_pages() {
        let root = tree("read");
        let long: String = (1..=30).map(|i| format!("line {i}\n")).collect();
        fs::write(root.join("long.txt"), long).unwrap();
        let s = sb(&root);

        let out = run_tool("read_file", &json!({ "path": "long.txt", "start_line": 10, "max_lines": 5 }), &s, &no_cancel());
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("전체 30줄, 10–14줄"));
        assert!(out.content.contains("   10\tline 10"));
        assert!(!out.content.contains("line 15"));
        assert!(out.content.contains("start_line=15"));

        // Numeric strings are accepted for integer args.
        let s2 = run_tool("read_file", &json!({ "path": "long.txt", "start_line": "29" }), &s, &no_cancel());
        assert!(s2.content.contains("29–30줄") && !s2.content.contains("start_line="));

        assert!(run_tool("read_file", &json!({ "path": "long.txt", "start_line": 99 }), &s, &no_cancel()).is_error);
        assert!(run_tool("read_file", &json!({ "path": "data.bin" }), &s, &no_cancel()).is_error);
        assert!(run_tool("read_file", &json!({ "path": "src" }), &s, &no_cancel()).is_error);
        assert!(run_tool("read_file", &json!({ "path": ".env" }), &s, &no_cancel()).is_error);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_tool_explains_the_app_saves_files() {
        let root = tree("unknown");
        let out = run_tool("write_file", &json!({ "path": "docs/plan.md" }), &sb(&root), &no_cancel());
        assert!(out.is_error);
        assert!(out.content.contains("읽기 전용") && out.content.contains("앱이 처리"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn canceled_walk_reports_cancel() {
        let root = tree("cancel");
        let canceled = AtomicBool::new(true);
        let out = run_tool("find_files", &json!({ "pattern": "*" }), &sb(&root), &canceled);
        assert!(out.is_error && out.content.contains("취소"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn outline_file_shows_the_skeleton() {
        let root = tree("outline");
        fs::write(
            root.join("src").join("OrderController.java"),
            "@RestController\npublic class OrderController {\n  @GetMapping(\"/o\")\n  public String list() {\n    return \"x\";\n  }\n}\n",
        )
        .unwrap();
        let out = run_tool("outline_file", &json!({ "path": "src/OrderController.java" }), &sb(&root), &no_cancel());
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("Java 아웃라인"), "{}", out.content);
        assert!(out.content.contains("▶ L1-7 @RestController public class OrderController"), "{}", out.content);
        assert!(out.content.contains("L3-6 @GetMapping(\"/o\") public String list()"), "{}", out.content);
        assert!(!out.content.contains("return"), "bodies are not shown");
        // Mapper XML → statements + tables.
        let xml = run_tool("outline_file", &json!({ "path": "src/OrderMapper.xml" }), &sb(&root), &no_cancel());
        assert!(xml.content.contains("select id=find") && xml.content.contains("ORDER_TABLE(R)"), "{}", xml.content);
        // Folders, binaries and secrets are refused.
        assert!(run_tool("outline_file", &json!({ "path": "src" }), &sb(&root), &no_cancel()).is_error);
        assert!(run_tool("outline_file", &json!({ "path": "data.bin" }), &sb(&root), &no_cancel()).is_error);
        assert!(run_tool("outline_file", &json!({ "path": ".env" }), &sb(&root), &no_cancel()).content.contains("보안상"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn find_references_tool_scans_all_roots_or_a_path() {
        let work = tree("refs-work");
        let code = tree("refs-code");
        fs::write(code.join("src").join("util").join("Helper.java"), "class Helper {\n  static int calc(int a) { return a; }\n}\n").unwrap();
        fs::write(code.join("Order.java"), "class Order {\n  int total() { return Helper.calc(1); }\n}\n").unwrap();
        let s = Sandbox::new(&work.to_string_lossy(), &[code.to_string_lossy().into_owned()]);
        let out = run_tool("find_references", &json!({ "symbol": "Helper.calc" }), &s, &no_cancel());
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("Order.java(1)") && out.content.contains("[int total()]"), "{}", out.content);
        assert!(out.content.contains("Helper.java(1, 정의)"), "{}", out.content);
        // Narrowed by file_pattern → only the definition file remains.
        let only = run_tool("find_references", &json!({ "symbol": "calc", "file_pattern": "Helper*" }), &s, &no_cancel());
        assert!(only.content.contains("Helper.java") && !only.content.contains("Order.java"), "{}", only.content);
        assert!(run_tool("find_references", &json!({}), &s, &no_cancel()).is_error);
        let _ = fs::remove_dir_all(&work);
        let _ = fs::remove_dir_all(&code);
    }

    #[test]
    fn relative_paths_fall_back_to_the_other_roots() {
        let work = tree("rel-work");
        let code = tree("rel-code");
        fs::create_dir_all(code.join("only").join("here")).unwrap();
        fs::write(code.join("only").join("here").join("Svc.java"), "class Svc {}\n").unwrap();
        let s = Sandbox::new(&work.to_string_lossy(), &[code.to_string_lossy().into_owned()]);
        // Exists only under the codebase root → resolved there.
        let p = s.resolve(Some("only/here/Svc.java")).unwrap();
        assert!(p.starts_with(fs::canonicalize(&code).unwrap()), "{}", p.display());
        // Exists in both → the cwd (first root) wins.
        let readme = s.resolve(Some("README.md")).unwrap();
        assert!(readme.starts_with(fs::canonicalize(&work).unwrap()));
        // Missing everywhere → the usual not-found error; `..` still can't escape.
        assert!(s.resolve(Some("nope.txt")).unwrap_err().contains("찾을 수 없습니다"));
        assert!(s.resolve(Some("..")).unwrap_err().contains("허용된 폴더 밖"));
        let _ = fs::remove_dir_all(&work);
        let _ = fs::remove_dir_all(&code);
    }

    #[test]
    fn read_file_batches_files_and_reads_symbols() {
        let root = tree("batch-read");
        fs::write(
            root.join("src").join("OrderService.java"),
            "public class OrderService {\n  /** 주문 조회 */\n  public Order findOrder(Long id) {\n    return dao.get(id);\n  }\n\n  public void cancel(Long id) {\n    dao.cancel(id);\n  }\n}\n",
        )
        .unwrap();
        let s = sb(&root);

        // One declaration by name — the body plus the Javadoc above it, nothing else.
        let one = run_tool("read_file", &json!({ "path": "src/OrderService.java", "symbol": "findOrder" }), &s, &no_cancel());
        assert!(!one.is_error, "{}", one.content);
        assert!(one.content.contains("[L2-5] public Order findOrder(Long id)"), "{}", one.content);
        assert!(one.content.contains("return dao.get(id);") && !one.content.contains("dao.cancel"), "{}", one.content);
        // Unknown names list the declarations instead of failing silently.
        let miss = run_tool("read_file", &json!({ "path": "src/OrderService.java", "symbol": "nope" }), &s, &no_cancel());
        assert!(miss.is_error && miss.content.contains("cancel(Long id)"), "{}", miss.content);

        // A batch mixes symbols, ranges and bare paths; one bad item doesn't fail the call.
        let batch = run_tool(
            "read_file",
            &json!({ "files": [
                { "path": "src/OrderService.java", "symbol": "OrderService.cancel" },
                { "path": "README.md", "start_line": 2 },
                "src/util/Helper.java",
                { "path": "missing.txt" }
            ] }),
            &s,
            &no_cancel(),
        );
        assert!(!batch.is_error, "{}", batch.content);
        assert!(batch.content.contains("### [1/4] src/OrderService.java · OrderService.cancel"), "{}", batch.content);
        assert!(batch.content.contains("dao.cancel(id);") && !batch.content.contains("dao.get(id)"));
        assert!(batch.content.contains("    2\tORDER_TABLE 설명") && batch.content.contains("class Helper {}"));
        assert!(batch.content.contains("### [4/4] missing.txt\n오류:"), "{}", batch.content);
        // A JSON-encoded array string is accepted too; all-failed → error.
        let all_bad = run_tool("read_file", &json!({ "files": "[\"nope1\", \"nope2\"]" }), &s, &no_cancel());
        assert!(all_bad.is_error && all_bad.content.contains("[2/2] nope2"), "{}", all_bad.content);
        // Over the item limit → the rest is reported, not dropped silently.
        let many: Vec<Value> = (0..10).map(|_| json!("README.md")).collect();
        let capped = run_tool("read_file", &json!({ "files": many }), &s, &no_cancel());
        assert!(capped.content.contains("요청 10개 중 앞의 8개만"), "{}", capped.content);
        // The `paths` alias is a batch too (shares the batch cap, all items shown).
        let alias = run_tool("read_file", &json!({ "paths": ["README.md", "Order.java"] }), &s, &no_cancel());
        assert!(alias.content.contains("### [2/2] Order.java") && alias.content.contains("class Order {"), "{}", alias.content);
        assert!(run_tool("read_file", &json!({}), &s, &no_cancel()).is_error);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn outline_search_and_references_take_batches() {
        let root = tree("batch-misc");
        let s = sb(&root);
        let out = run_tool("outline_file", &json!({ "paths": ["Order.java", "src/OrderMapper.xml", "nope.java"] }), &s, &no_cancel());
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("### [1/3] Order.java") && out.content.contains("class Order"), "{}", out.content);
        assert!(out.content.contains("ORDER_TABLE(R)") && out.content.contains("### [3/3] nope.java\n오류:"));

        // Several queries in one walk, grouped per query.
        let multi = run_tool("search_text", &json!({ "queries": ["order_table", "class Helper", "zzz_none"] }), &s, &no_cancel());
        assert!(!multi.is_error, "{}", multi.content);
        assert!(multi.content.contains("검색어 3개 내용 검색"), "{}", multi.content);
        assert!(multi.content.contains("### 'order_table' — 3건"), "{}", multi.content);
        assert!(multi.content.contains("### 'class Helper' — 1건") && multi.content.contains("Helper.java:1:"));
        assert!(multi.content.contains("### 'zzz_none' — 0건"));
        assert!(!multi.content.contains(".env"), "secrets stay excluded");
        // A one-element array keeps the single-query format.
        let single = run_tool("search_text", &json!({ "queries": ["class Helper"] }), &s, &no_cancel());
        assert!(single.content.starts_with("'class Helper' 내용 검색 — 1건"), "{}", single.content);
        assert!(run_tool("search_text", &json!({}), &s, &no_cancel()).is_error);

        let refs = run_tool("find_references", &json!({ "symbols": ["Order", "Helper"] }), &s, &no_cancel());
        assert!(!refs.is_error, "{}", refs.content);
        assert!(refs.content.contains("### [1/2] 'Order' 참조") && refs.content.contains("### [2/2] 'Helper' 참조"), "{}", refs.content);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn batch_caps_share_the_result_budget() {
        assert_eq!(batch_item_cap(1), RESULT_CHARS);
        assert_eq!(batch_item_cap(2), RESULT_CHARS, "two items still get the single cap each");
        assert_eq!(batch_item_cap(8), BATCH_RESULT_CHARS / 8);
        assert_eq!(batch_item_cap(100), BATCH_ITEM_MIN);
    }

    #[test]
    fn tool_specs_are_well_formed() {
        let specs = tool_specs();
        let names: Vec<&str> = specs
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["list_dir", "find_files", "search_text", "outline_file", "find_references", "read_file"]);
        for t in specs.as_array().unwrap() {
            assert_eq!(t["type"], "function");
            assert!(t["function"]["parameters"].is_object(), "parameters must be a JSON Schema object");
        }
    }
}
