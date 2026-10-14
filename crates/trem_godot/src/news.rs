//! The News page's text (dec065): what the legacy client's `GetNews` /
//! `UI_UpdateNews` did with the file at `cl_newsURL`, minus libcurl.
//!
//! Legacy (tremulous-legacy `cl_ui.c:86-131`, `ui_main.c:4587-4650`):
//! download `http://tremulous.net/clientnews.txt`, read at most
//! `MAX_NEWS_STRING` (10000) bytes, show "Retrieving..." until it is there,
//! word-wrap it, split it on newlines and at 84 columns into at most 50 lines
//! and keep only printable characters. The text is only ever displayed: it is
//! never run as a command and never names a cvar.
//!
//! That address no longer serves news (a GitHub Pages 404 since the site
//! moved), so a failed fetch shows a calm note saying the page is a leftover
//! of the original game, with the address, instead of the legacy red errors.
//! It is still asked every session: should it serve a news file again (plain
//! text, `^2Title^7 text...` as the original did), its items are listed first,
//! above the release list. Anything else (an HTML or parking page, JSON,
//! binary or mostly unprintable bytes, empty, over 10000 bytes) is not news.

use crate::http::{HttpError, HttpJob, HttpOpts, HttpResult, Overflow};
use std::time::Duration;

/// q_shared.h `MAX_NEWS_STRING`
pub const MAX_NEWS_STRING: usize = 10000;
pub const MAX_NEWS_LINES: usize = 50;
pub const MAX_NEWS_LINEWIDTH: usize = 85;
/// cl_ui.c:103-104, the default of the archived cvar `cl_newsURL`
pub const DEFAULT_NEWS_URL: &str = "http://tremulous.net/clientnews.txt";
/// what the page says while the download runs (cl_ui.c:119)
pub const RETRIEVING: &str = "Retrieving...";

/// Keep only what the page may show: printable ASCII (`isprint`) and the
/// newline. Everything else (control characters, ESC, tabs, carriage
/// returns, bytes of 128 and up) is dropped, as the legacy line loop did.
pub fn printable(s: &[u8]) -> String {
    s.iter()
        .filter(|&&b| b == b'\n' || (0x20..0x7f).contains(&b))
        .map(|&b| b as char)
        .collect()
}

/// The news text of a downloaded body, or `None` if it is not news: empty
/// after cleaning, longer than `MAX_NEWS_STRING`, binary (a NUL byte, or
/// over a quarter of its bytes neither printable ASCII nor a line break), an
/// HTML or XML page (an error or parking page the server sent with status
/// 200, wherever its tags start), or JSON.
pub fn parse(body: &[u8]) -> Option<String> {
    if body.len() > MAX_NEWS_STRING || body.contains(&0) {
        return None;
    }
    let odd = body
        .iter()
        .filter(|&&b| !(b == b'\n' || b == b'\r' || b == b'\t' || (0x20..0x7f).contains(&b)))
        .count();
    if odd * 4 > body.len() {
        return None;
    }
    let lower = body.to_ascii_lowercase();
    let has = |tag: &[u8]| lower.windows(tag.len()).any(|w| w == tag);
    if [
        &b"<!doctype"[..],
        b"<html",
        b"<head",
        b"<body",
        b"<script",
        b"<meta",
        b"<title",
        b"<?xml",
    ]
    .iter()
    .any(|t| has(t))
    {
        return None;
    }
    let head = body.iter().find(|b| !b.is_ascii_whitespace());
    if matches!(head, Some(b'{') | Some(b'[')) && crate::json::parse(body).is_some() {
        return None;
    }
    let text = printable(body);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// The calm note for a news address that gave nothing. Says what the page is
/// and where it looked; not red, no "compile with CURL".
pub fn unavailable(url: &str, err: Option<&HttpError>) -> String {
    let url = printable(url.as_bytes()).replace('\n', "");
    match err {
        // the server answered, but not with news
        None
        | Some(HttpError::Status(_))
        | Some(HttpError::TooLarge)
        | Some(HttpError::NotText(_)) => {
            format!("News is a leftover from the original Tremulous. Its address, {url}, no longer serves news.")
        }
        // nothing came back (offline, DNS, timeout, bad cvar)
        Some(_) => format!("News is a leftover from the original Tremulous. Its address, {url}, could not be reached."),
    }
}

/// What the page shows for a finished download.
pub fn from_result(url: &str, r: &HttpResult) -> String {
    match r {
        Ok(body) => parse(body).unwrap_or_else(|| unavailable(url, None)),
        Err(e) => unavailable(url, Some(e)),
    }
}

/// `UI_UpdateNews`' line loop over the wrapped text: split on newlines and
/// at `MAX_NEWS_LINEWIDTH - 1` columns, at most `MAX_NEWS_LINES` lines,
/// printable characters only.
pub fn split_lines(wrapped: &str) -> Vec<String> {
    split_lines_max(wrapped, MAX_NEWS_LINES)
}

/// `split_lines` with its own line budget.
pub fn split_lines_max(wrapped: &str, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = vec![String::new()];
    for c in wrapped.chars() {
        let line_len = lines.last().map(|l| l.len()).unwrap_or(0);
        if line_len == MAX_NEWS_LINEWIDTH - 1 || c == '\n' {
            if lines.len() == max {
                break;
            }
            lines.push(String::new());
            if c != '\n' {
                lines.last_mut().unwrap().push(c);
            }
        } else if c.is_ascii_graphic() || c == ' ' {
            lines.last_mut().unwrap().push(c);
        }
    }
    lines
}

// ---------------------------------------------------------------- releases

/// The release list under the news (dec065): GitHub's REST API for the
/// project's releases, in the archived cvar `cl_newsReleasesURL`.
pub const DEFAULT_RELEASES_URL: &str =
    "https://api.github.com/repos/MeepingsCorner/Tremulous-Revamped/releases?per_page=10";
/// the second repository's releases (dec067), cvar `cl_newsReleasesURL2`
pub const DEFAULT_RELEASES_URL2: &str =
    "https://api.github.com/repos/MeepingsCorner/Tremulous-Revamped-Release/releases?per_page=10";
/// Used only when no API answers (rate limit, offline GitHub API): a static
/// `releases.json` in the API's shape (an array; an entry may add `"repo"`),
/// cvar `cl_newsReleasesStaticURL`. Proposed home: the root of the Release
/// repository's default branch, rewritten by its release workflow. Nothing is
/// hosted by this change; until the file exists the fetch 404s and the page
/// shows the unavailable line as before.
pub const DEFAULT_RELEASES_STATIC_URL: &str = "https://raw.githubusercontent.com/MeepingsCorner/Tremulous-Revamped-Release/main/releases.json";
pub const RELEASES_HEADING: &str = "^3Tremulous Revamped releases^7";
pub const RELEASES_UNAVAILABLE: &str = "Release list unavailable right now";
/// the API answer carries every asset of every release: allow 256 KiB
pub const MAX_RELEASES_BYTES: usize = 256 * 1024;
pub const MAX_RELEASES: usize = 10;
/// the longest first line of a release's text shown under its title (dec066):
/// the page wraps it onto a second line (the page is ~65 characters wide), so
/// a gap, a title, who and when, and two lines of text are five lines
const MAX_BLURB: usize = 130;
/// the release list's own line budget under the news part (the news file
/// keeps the legacy 50): the heading and ten entries of five lines
pub const MAX_RELEASE_LINES: usize = 1 + 5 * MAX_RELEASES;
/// two releases published in both repositories within this many seconds of
/// each other with the same tag, title or set of files are one release
pub const SAME_RELEASE_SECS: i64 = 30 * 86400;

/// What a download is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    News,
    Releases,
}

/// Start the download of `url` for `kind` (5 s, size-capped, redirects limited).
pub fn start(url: &str, kind: Kind) -> HttpJob {
    HttpJob::get(url, opts_for(kind))
}

/// The limits of a download for `kind`: the news file must be plain text of
/// at most `MAX_NEWS_STRING` bytes (a longer one is not news, not cut short).
pub fn opts_for(kind: Kind) -> HttpOpts {
    match kind {
        Kind::News => HttpOpts {
            max_bytes: MAX_NEWS_STRING,
            overflow: Overflow::Fail,
            text_only: true,
            ..HttpOpts::default()
        },
        Kind::Releases => HttpOpts {
            timeout: Duration::from_secs(5),
            max_bytes: MAX_RELEASES_BYTES,
            overflow: Overflow::Fail,
            accept: Some("application/vnd.github+json"),
            ..HttpOpts::default()
        },
    }
}

/// Remote text for one display line: printable ASCII only, no newlines, no
/// `^` (a colour code the release text did not ask the page for).
fn plain(s: &str) -> String {
    let mut out = String::new();
    // a tab is a gap, not nothing: "a<TAB>b" must not become "ab" (the news
    // file itself stays legacy-exact and drops tabs in `printable`)
    let clean = printable(s.replace('\t', " ").as_bytes());
    let mut it = clean.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\n' => {}
            // a colour code: the caret and its digit or letter go
            '^' => {
                if it.peek().is_some_and(|n| n.is_ascii_alphanumeric()) {
                    it.next();
                }
            }
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

/// Markdown to plain text for one line: links keep their label, emphasis,
/// code ticks, heading and list marks go.
pub fn strip_markdown(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line
        .trim_start()
        .trim_start_matches(['#', '>', '-', '*', '+'])
        .trim_start();
    while let Some(i) = rest.find('[') {
        let after = &rest[i + 1..];
        match after.find("](") {
            Some(j) if after[..j].find(']').is_none() => match after[j + 2..].find(')') {
                Some(k) => {
                    out.push_str(&rest[..i]);
                    out.push_str(&after[..j]);
                    rest = &after[j + 2 + k + 1..];
                }
                None => break,
            },
            _ => {
                out.push_str(&rest[..i + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    plain(&out.replace(['`', '*'], ""))
}

/// Lower-case letters and digits only: "4K Texture-Pack" -> "4ktexturepack".
fn fold(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Does this line only restate the release's title? It is the title, or it
/// starts with it ("4K texture pack for Tremulous Revamped ..." under the
/// title "4K texture pack").
fn restates(line: &str, title: &str) -> bool {
    let (l, t) = (line.to_ascii_lowercase(), title.to_ascii_lowercase());
    !t.is_empty()
        && l.starts_with(&t)
        && l[t.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_ascii_alphanumeric())
}

/// The first line of a release text that says something the title does not,
/// as plain text, cut to `MAX_BLURB` characters.
fn blurb(body: &str, title: &str) -> String {
    for l in body.lines() {
        let t = strip_markdown(l);
        if !t.is_empty() && !restates(&t, title) {
            return if t.len() > MAX_BLURB {
                // cut at a word when there is one in the second half
                let cut = &t[..MAX_BLURB - 3];
                let cut = match cut.rfind(' ') {
                    Some(i) if i > MAX_BLURB / 2 => &cut[..i],
                    _ => cut,
                };
                format!("{}...", cut.trim_end())
            } else {
                t
            };
        }
    }
    String::new()
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Unix seconds and the `YYYY-MM-DD` part of a GitHub timestamp
/// (`2026-10-10T12:42:52Z`).
fn parse_ts(ts: &str) -> Option<(i64, &str)> {
    let date = ts.get(..10)?;
    let b = date.as_bytes();
    if !(b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()))
    {
        return None;
    }
    let n = |r: std::ops::Range<usize>| ts.get(r).and_then(|x| x.parse::<i64>().ok());
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let hms = if ts.len() >= 19 && ts.as_bytes()[10] == b'T' {
        n(11..13).unwrap_or(0) * 3600 + n(14..16).unwrap_or(0) * 60 + n(17..19).unwrap_or(0)
    } else {
        0
    };
    Some((days_from_civil(y, m, d) * 86400 + hms, date))
}

/// The date line under a title: who, the date, and how long ago, as
/// "MeepingsCorner released this on 2026-10-10 (4 days ago)"; a release over
/// a year old just has its date.
fn date_line(who: &str, date: Option<(&str, String)>) -> String {
    match date {
        Some((_, rel)) if rel.starts_with("on ") => format!("{who} {rel}"),
        Some((d, rel)) => format!("{who} on {d} ({rel})"),
        None => who.to_string(),
    }
}

/// How long ago, in the releases page's words: today, yesterday, N days ago
/// (under a week), N weeks ago (under 30 days), N months ago (under a year),
/// else "on YYYY-MM-DD".
pub fn relative_time(then: i64, date: &str, now: i64) -> String {
    // rounded to the nearest day, as the releases page does
    let days = (now - then + 43200).div_euclid(86400);
    match days {
        i64::MIN..=0 => "today".to_string(),
        1 => "yesterday".to_string(),
        2..=6 => format!("{days} days ago"),
        7..=29 => {
            let w = days / 7;
            format!("{w} week{} ago", if w == 1 { "" } else { "s" })
        }
        30..=364 => {
            let m = days / 30;
            format!("{m} month{} ago", if m == 1 { "" } else { "s" })
        }
        _ => format!("on {date}"),
    }
}

/// One release of the list.
pub struct Release {
    ts: i64,
    title: String,
    pre: bool,
    /// the date line ("<author> released this on <date> (<how long ago>)")
    when: String,
    blurb: String,
    repo: String,
    tag: String,
    /// the sorted sha256 digests of its files (GitHub's `assets[].digest`)
    files: Vec<String>,
    /// which source it came from; on a duplicate the higher one is kept
    rank: usize,
}

impl Release {
    /// The same release published twice (in both repositories, or listed
    /// twice in one answer): within `SAME_RELEASE_SECS` of each other (or
    /// undated) and the same tag, the same title (letters and digits,
    /// ignoring case), or the same set of files.
    fn same_as(&self, o: &Release) -> bool {
        let close = self.ts == 0 || o.ts == 0 || (self.ts - o.ts).abs() <= SAME_RELEASE_SECS;
        close
            && ((!self.tag.is_empty() && self.tag.eq_ignore_ascii_case(&o.tag))
                || (!fold(&self.title).is_empty() && fold(&self.title) == fold(&o.title))
                || (!self.files.is_empty() && self.files == o.files))
    }
}

/// Each release once: of a release found more than once (see
/// `Release::same_as`) the one of the higher `rank` is kept (the Release
/// repository, the second address), else the first one listed.
fn dedupe(mut list: Vec<Release>) -> Vec<Release> {
    // stable: equal ranks keep the order they came in
    list.sort_by(|a, b| b.rank.cmp(&a.rank));
    let mut kept: Vec<Release> = Vec::new();
    for r in list {
        if !kept.iter().any(|k| k.same_as(&r)) {
            kept.push(r);
        }
    }
    kept
}

/// The repository name of a GitHub API address (`/repos/OWNER/REPO/...`), else "".
pub fn repo_of(url: &str) -> String {
    let Some(i) = url.find("/repos/") else {
        return String::new();
    };
    let mut it = url[i + 7..].split('/');
    let _owner = it.next();
    it.next()
        .map(|r| plain(r.split('?').next().unwrap_or("")))
        .unwrap_or_default()
}

/// The releases in a JSON list (GitHub's API answer or the static file),
/// drafts and untitled ones left out, at most `MAX_RELEASES`. `repo` names
/// where they came from unless an entry says (`"repo"`). `now`: unix seconds.
/// `None` if the body is not a JSON list.
fn parse_entries(body: &[u8], now: i64, repo: &str) -> Option<Vec<Release>> {
    parse_ranked(body, now, repo, 0)
}

/// `parse_entries` for the source of preference `rank` (see `dedupe`).
fn parse_ranked(body: &[u8], now: i64, repo: &str, rank: usize) -> Option<Vec<Release>> {
    let v = crate::json::parse(body)?;
    let list = v.as_array()?;
    let mut out = Vec::new();
    for r in list {
        if out.len() == MAX_RELEASES {
            break;
        }
        let get = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("");
        if r.get("draft").and_then(|x| x.as_bool()).unwrap_or(false) {
            continue;
        }
        let tag = plain(get("tag_name"));
        let name = plain(get("name"));
        let title = if name.is_empty() { tag } else { name };
        if title.is_empty() {
            continue;
        }
        let pre = r
            .get("prerelease")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let login = plain(
            r.get("author")
                .and_then(|a| a.get("login"))
                .and_then(|x| x.as_str())
                .unwrap_or(""),
        );
        let ts = if get("published_at").is_empty() {
            get("created_at")
        } else {
            get("published_at")
        };
        let parsed = parse_ts(ts);
        let who = if login.is_empty() {
            "Released".to_string()
        } else {
            format!("{login} released this")
        };
        let when = date_line(&who, parsed.map(|(t, d)| (d, relative_time(t, d, now))));
        let own = plain(get("repo"));
        let mut files: Vec<String> = r
            .get("assets")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.get("digest").and_then(|d| d.as_str()))
                    .filter(|d| !d.is_empty())
                    .map(|d| d.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        out.push(Release {
            ts: parsed.map(|(t, _)| t).unwrap_or(0),
            blurb: blurb(get("body"), &title),
            title,
            pre,
            when,
            repo: if own.is_empty() {
                repo.to_string()
            } else {
                own
            },
            tag: get("tag_name").trim().to_string(),
            files,
            rank,
        });
    }
    Some(out)
}

/// The list text, each release once (`dedupe`), newest first, in the
/// wording of the releases page: the heading, then per release a blank line,
/// its name (else tag) in green, "<author> released this on <date> (<how
/// long ago>)", and the first line of its text that is not the title again.
fn render_releases(list: Vec<Release>) -> String {
    let mut list = dedupe(list);
    // stable: entries without a date keep the order they came in
    list.sort_by(|a, b| b.ts.cmp(&a.ts));
    let mut out = String::from(RELEASES_HEADING);
    for r in list.iter().take(MAX_RELEASES) {
        out.push_str("\n\n^2");
        out.push_str(&r.title);
        out.push_str("^7");
        if r.pre {
            out.push_str(" (pre-release)");
        }
        out.push('\n');
        out.push_str(&r.when);
        if !r.blurb.is_empty() {
            out.push_str("\n  ");
            out.push_str(&r.blurb);
        }
    }
    if list.is_empty() {
        out.push_str("\nNo releases yet.");
    }
    out
}

/// The release list text for one JSON list, no repo shown. `None` if the
/// body is not a JSON list.
pub fn parse_releases(body: &[u8], now: i64) -> Option<String> {
    parse_entries(body, now, "").map(render_releases)
}

/// What the page shows for a finished release list download.
pub fn releases_from_result(r: &HttpResult, now: i64) -> String {
    match r {
        Ok(body) => parse_releases(body, now).unwrap_or_else(|| RELEASES_UNAVAILABLE.to_string()),
        Err(_) => RELEASES_UNAVAILABLE.to_string(),
    }
}

/// The local clock, unix seconds.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Did this download give real content (kept for the session) or not
/// (tried again later)?
pub fn news_ok(r: &HttpResult) -> bool {
    matches!(r, Ok(b) if parse(b).is_some())
}

// ------------------------------------------------------------------ section

/// One part of the page: its download and the answer kept for the session. A
/// failed download is tried again after `RETRY_MS` (the client may have been
/// started offline).
#[derive(Default)]
pub struct Section {
    job: Option<HttpJob>,
    cache: Option<(String, Option<i32>)>,
}

pub const RETRY_MS: i32 = 60_000;

impl Section {
    /// One frame of this part. `begin`: the page was just opened. `start`
    /// begins the download, `render` turns its answer into (text, ok).
    /// Returns the text to show once there is one, `None` while retrieving.
    /// `timed_out`: the page's own backstop timer ran out.
    pub fn step(
        &mut self,
        begin: bool,
        now: i32,
        timed_out: bool,
        start: &dyn Fn() -> HttpJob,
        render: &dyn Fn(&HttpResult) -> (String, bool),
        timeout_text: &dyn Fn() -> String,
    ) -> Option<String> {
        if begin
            && self
                .cache
                .as_ref()
                .is_some_and(|(_, retry)| retry.is_some_and(|t| now >= t))
        {
            self.cache = None;
        }
        if let Some((t, _)) = &self.cache {
            return Some(t.clone());
        }
        if begin && self.job.is_none() {
            self.job = Some(start());
        }
        let answer = self.job.as_mut().and_then(|j| j.poll().cloned());
        let (text, ok) = match answer {
            Some(r) => render(&r),
            None if timed_out => (timeout_text(), false),
            None => return None,
        };
        self.job = None;
        self.cache = Some((
            text.clone(),
            if ok {
                None
            } else {
                Some(now.saturating_add(RETRY_MS))
            },
        ));
        Some(text)
    }
}

/// The release list part (dec067): the live API of each repository first
/// (`urls`, merged newest first, each item tagged with its repo); only when
/// none of them answers, the static file. A part that answered is kept for
/// the session; if one of the APIs failed it is tried again after
/// `RETRY_MS`, and a list nobody could give says `RELEASES_UNAVAILABLE`.
#[derive(Default)]
pub struct ReleasesSection {
    api: Vec<(String, HttpJob, Option<HttpResult>)>,
    fallback: Option<HttpJob>,
    cache: Option<(String, Option<i32>)>,
}

impl ReleasesSection {
    /// One frame. `fetch` starts a download; `clock`: unix seconds (for "3 days ago").
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        begin: bool,
        now: i32,
        timed_out: bool,
        fetch: &dyn Fn(&str) -> HttpJob,
        urls: &[String],
        static_url: &str,
        clock: i64,
    ) -> Option<String> {
        if begin
            && self
                .cache
                .as_ref()
                .is_some_and(|(_, retry)| retry.is_some_and(|t| now >= t))
        {
            self.cache = None;
        }
        if let Some((t, _)) = &self.cache {
            return Some(t.clone());
        }
        if begin && self.api.is_empty() && self.fallback.is_none() {
            self.api = urls.iter().map(|u| (repo_of(u), fetch(u), None)).collect();
        }
        for (_, job, res) in self.api.iter_mut() {
            if res.is_none() {
                *res = job.poll().cloned();
            }
        }
        let retry = Some(now.saturating_add(RETRY_MS));
        let mut done: Option<(String, Option<i32>)> = None;
        if !self.api.is_empty() && self.api.iter().all(|a| a.2.is_some()) {
            let mut all = Vec::new();
            let mut failed = false;
            let mut any_ok = false;
            // a later address is preferred on a duplicate (dec067: the
            // Release repository is the second one)
            for (rank, (repo, _, res)) in self.api.iter().enumerate() {
                match res
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .and_then(|b| parse_ranked(b, clock, repo, rank))
                {
                    Some(e) => {
                        any_ok = true;
                        all.extend(e);
                    }
                    None => failed = true,
                }
            }
            self.api.clear();
            if any_ok {
                done = Some((render_releases(all), if failed { retry } else { None }));
            } else {
                self.fallback = Some(fetch(static_url));
            }
        }
        if done.is_none() {
            if let Some(job) = self.fallback.as_mut() {
                if let Some(r) = job.poll().cloned() {
                    done = Some(match r.ok().and_then(|b| parse_entries(&b, clock, "")) {
                        Some(e) => (render_releases(e), retry),
                        None => (RELEASES_UNAVAILABLE.to_string(), retry),
                    });
                }
            }
        }
        if done.is_none() && timed_out {
            done = Some((RELEASES_UNAVAILABLE.to_string(), retry));
        }
        let (text, r) = done?;
        self.api.clear();
        self.fallback = None;
        self.cache = Some((text.clone(), r));
        Some(text)
    }
}

/// The whole page text: the news part, a blank line, the release list.
/// A part that is still downloading says "Retrieving...".
pub fn page_text(news: Option<&str>, releases: Option<&str>) -> String {
    format!(
        "{}\n\n{}",
        news.unwrap_or(RETRIEVING),
        releases.unwrap_or(RETRIEVING)
    )
}

/// The page's lines from its two wrapped parts: the news file first (its
/// legacy 50 lines), a blank line, then the release list with its own
/// budget, so a long news file never hides the releases.
pub fn page_lines(news_wrapped: &str, releases_wrapped: &str) -> Vec<String> {
    let mut lines = split_lines(news_wrapped);
    // one blank line between the parts, however the news text ends
    while lines.len() > 1 && lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.push(String::new());
    lines.extend(split_lines_max(releases_wrapped, MAX_RELEASE_LINES));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_news_passes_with_colour_codes_and_newlines() {
        let t = parse(b"^2Tremulous^7 news\nline two\n").unwrap();
        assert_eq!(t, "^2Tremulous^7 news\nline two\n");
    }

    #[test]
    fn unsafe_bytes_are_dropped() {
        // ESC, CR, TAB, NUL-free control bytes, DEL and bytes >= 128
        let t = parse(b"a\x1b[31mb\r\n\tc\x7fd\xc3\xa9e").unwrap();
        assert_eq!(t, "a[31mb\ncde");
    }

    #[test]
    fn a_tab_in_a_release_line_becomes_a_space_but_news_stays_legacy_exact() {
        assert_eq!(plain("v1.2\tfixes"), "v1.2 fixes");
        assert_eq!(blurb("Fixes\tand\ttweaks", "v1"), "Fixes and tweaks");
        assert_eq!(parse(b"a\tb").unwrap(), "ab");
    }

    #[test]
    fn commands_and_cvar_syntax_stay_inert_text() {
        // the text is shown, never run: the parser only filters characters
        let t = parse(b"quit; set sv_cheats 1\n\\rcon x\n$(echo hi)").unwrap();
        assert!(t.contains("quit; set sv_cheats 1"));
        assert!(t.contains("\\rcon x"));
    }

    #[test]
    fn binary_html_and_empty_bodies_are_not_news() {
        assert_eq!(parse(b"GIF89a\x00\x01"), None);
        assert_eq!(parse(b"<!DOCTYPE html><html>404</html>"), None);
        assert_eq!(parse(b"  \n <HTML><body>x"), None);
        assert_eq!(parse(b"<?xml version=\"1.0\"?>"), None);
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"\n \x1b\x07 \r\n"), None);
    }

    #[test]
    fn a_news_file_over_max_news_string_is_not_news() {
        assert!(parse(&vec![b'x'; MAX_NEWS_STRING]).is_some());
        assert_eq!(parse(&vec![b'x'; MAX_NEWS_STRING + 1]), None);
        // and the download itself fails at the cap, plain text only
        let o = opts_for(Kind::News);
        assert!(o.text_only && o.overflow == Overflow::Fail);
        assert!(!opts_for(Kind::Releases).text_only);
        assert_eq!(
            crate::http::apply_cap(vec![b'x'; MAX_NEWS_STRING + 1], &o),
            Err(HttpError::TooLarge)
        );
    }

    /// the shape of the original file (tremulous.net/clientnews.txt, text/plain,
    /// archived 2010-06-15): items as "^2Title^7 text"
    const LEGACY_NEWS: &str = "^2Balance Updates^7 December 13th 2009 - Official Servers got some minor balance updates today.\n^2Community Spotlight #1^7 Check out the Tremulous Home page.\n";

    #[test]
    fn a_legacy_shaped_news_file_is_news() {
        assert_eq!(parse(LEGACY_NEWS.as_bytes()).as_deref(), Some(LEGACY_NEWS));
        // a few non-ASCII bytes (an accent) do not reject it; they are dropped
        let t = parse("Servers are back up after the move, merci a tous. Caf\u{e9}.".as_bytes())
            .unwrap();
        assert!(t.ends_with("Caf.") && !t.contains('\u{e9}'), "{t}");
        // a line that starts with a bracket is still text
        assert!(parse(b"[Update] servers are back").is_some());
    }

    #[test]
    fn html_parking_json_and_garbage_are_not_news() {
        for bad in [
            &b"Domain for sale!\n<script src=x.js></script>"[..],
            b"\n\n  <HEAD><TITLE>Parked</TITLE></HEAD>",
            b"Moved: <a href=x>here</a><body>",
            b"{\"message\":\"Not Found\"}",
            b"[1, 2, 3]",
            b"\xff\xfeU\x00T\x00",
            &[0xc3, 0xa9, 0xc3, 0xa9, b'a', b'b', b'c', 0xe2, 0x82, 0xac][..],
            b"   \n\t \r\n",
        ] {
            assert_eq!(parse(bad), None, "{:?}", String::from_utf8_lossy(bad));
            assert!(
                from_result(DEFAULT_NEWS_URL, &Ok(bad.to_vec())).contains("no longer serves news")
            );
        }
    }

    #[test]
    fn a_failed_download_is_a_calm_leftover_note_with_the_url() {
        let u = DEFAULT_NEWS_URL;
        for r in [
            Err(HttpError::Status(404)),
            Ok(b"<html>Not found</html>".to_vec()),
            Ok(Vec::new()),
            Err(HttpError::TooLarge),
            Err(HttpError::NotText("text/html".into())),
        ] {
            let s = from_result(u, &r);
            assert_eq!(s, "News is a leftover from the original Tremulous. Its address, http://tremulous.net/clientnews.txt, no longer serves news.");
        }
        for e in [
            HttpError::Timeout,
            HttpError::BadUrl,
            HttpError::Network("dns".into()),
        ] {
            let s = from_result(u, &Err(e));
            assert!(
                s.contains("leftover") && s.contains(u) && s.contains("could not be reached"),
                "{s}"
            );
        }
        for s in [
            from_result(u, &Err(HttpError::Status(404))),
            from_result(u, &Err(HttpError::Timeout)),
        ] {
            assert!(
                !s.contains("^1") && !s.contains("CURL") && !s.contains("Error"),
                "{s}"
            );
        }
    }

    #[test]
    fn the_url_in_the_note_is_cleaned() {
        let s = unavailable("http://x/\x1b[2J\nquit", Some(&HttpError::Timeout));
        assert!(s.contains("http://x/[2Jquit"), "{s}");
        assert!(!s.contains('\x1b'));
        assert_eq!(s.matches('\n').count(), 0);
    }

    #[test]
    fn good_news_is_shown_as_is() {
        let r: HttpResult = Ok(b"Version 1.3 is out.\n".to_vec());
        assert_eq!(from_result(DEFAULT_NEWS_URL, &r), "Version 1.3 is out.\n");
    }

    #[test]
    fn lines_split_on_newlines_and_at_the_width() {
        assert_eq!(split_lines("ab\ncd"), vec!["ab", "cd"]);
        let long = "x".repeat(200);
        let l = split_lines(&long);
        assert_eq!(l[0].len(), MAX_NEWS_LINEWIDTH - 1);
        assert_eq!(l[1].len(), MAX_NEWS_LINEWIDTH - 1);
        assert_eq!(l[2].len(), 200 - 2 * (MAX_NEWS_LINEWIDTH - 1));
        // at most 50 lines
        let many = "a\n".repeat(200);
        assert_eq!(split_lines(&many).len(), MAX_NEWS_LINES);
        // non-printable characters never reach a line
        assert_eq!(split_lines("a\x1bb\tc"), vec!["abc"]);
    }

    const SAMPLE: &str = r###"[
 {"tag_name":"v0.2.0","name":"Second ^1release","draft":false,"prerelease":true,"published_at":"2026-10-12T08:00:00Z",
  "author":{"login":"MeepingsCorner"},
  "assets":[{"name":"a.zip","size":5}],
  "body":"## What is new\n\n**Bold** news with [a link](https://example.org/x) and `code` here\nsecond line"},
 {"tag_name":"hd-textures-v1","name":"4K texture pack","draft":false,"prerelease":false,"published_at":"2026-10-10T12:42:52Z",
  "author":{"login":"MeepingsCorner"},
  "body":"4K texture pack for Tremulous Revamped (lossless).\u0007\n\nDownload both parts."},
 {"tag_name":"secret","name":"Draft","draft":true,"published_at":"2026-10-13T00:00:00Z","body":"x"},
 {"tag_name":"v0.1.0","name":"","draft":false,"prerelease":false,"created_at":"2026-10-01T00:00:00Z","body":""}
]"###;
    /// 2026-10-13 12:00:00 UTC
    const NOW: i64 = 1791892800;

    #[test]
    fn the_clock_maths_matches_known_timestamps() {
        assert_eq!(parse_ts("1970-01-01T00:00:00Z").unwrap().0, 0);
        assert_eq!(parse_ts("2000-03-01T00:00:00Z").unwrap().0, 951868800);
        assert_eq!(parse_ts("2026-10-13T12:00:00Z").unwrap().0, NOW);
        assert_eq!(parse_ts("2026-10-10T12:42:52Z").unwrap().1, "2026-10-10");
        assert_eq!(parse_ts("2026-13-10T00:00:00Z"), None);
        assert_eq!(parse_ts("nope"), None);
        assert_eq!(parse_ts(""), None);
    }

    #[test]
    fn relative_time_uses_the_releases_page_words() {
        let at = |ts: &str| {
            let (t, d) = parse_ts(ts).unwrap();
            relative_time(t, d, NOW)
        };
        assert_eq!(at("2026-10-13T11:00:00Z"), "today");
        assert_eq!(at("2026-10-13T00:30:00Z"), "today");
        assert_eq!(at("2026-10-13T23:00:00Z"), "today"); // a clock a little ahead
        assert_eq!(at("2026-10-12T08:00:00Z"), "yesterday");
        assert_eq!(at("2026-10-10T12:42:52Z"), "3 days ago");
        assert_eq!(at("2026-10-07T12:00:00Z"), "6 days ago");
        assert_eq!(at("2026-10-06T12:00:00Z"), "1 week ago");
        assert_eq!(at("2026-09-22T12:00:00Z"), "3 weeks ago");
        assert_eq!(at("2026-09-13T12:00:00Z"), "1 month ago");
        assert_eq!(at("2026-04-13T12:00:00Z"), "6 months ago");
        assert_eq!(at("2025-10-13T12:00:00Z"), "on 2025-10-13");
        assert_eq!(at("2024-02-29T12:00:00Z"), "on 2024-02-29");
    }

    #[test]
    fn releases_list_newest_first_in_the_releases_page_wording() {
        let t = parse_releases(SAMPLE.as_bytes(), NOW).unwrap();
        let want = "^3Tremulous Revamped releases^7\n\
\n^2Second release^7 (pre-release)\nMeepingsCorner released this on 2026-10-12 (yesterday)\n  What is new\n\
\n^24K texture pack^7\nMeepingsCorner released this on 2026-10-10 (3 days ago)\n  Download both parts.\n\
\n^2v0.1.0^7\nReleased on 2026-10-01 (1 week ago)";
        assert_eq!(t, want);
        assert!(!t.contains("Draft") && !t.contains("secret"));
    }

    #[test]
    fn the_date_line_has_the_date_and_how_long_ago() {
        let d = |ts: &str| {
            let (t, d) = parse_ts(ts).unwrap();
            date_line("m released this", Some((d, relative_time(t, d, NOW))))
        };
        assert_eq!(
            d("2026-10-13T06:00:00Z"),
            "m released this on 2026-10-13 (today)"
        );
        assert_eq!(
            d("2026-10-10T12:42:52Z"),
            "m released this on 2026-10-10 (3 days ago)"
        );
        assert_eq!(
            d("2026-09-22T12:00:00Z"),
            "m released this on 2026-09-22 (3 weeks ago)"
        );
        // over a year: the date alone, not "on 2025-10-13 (on 2025-10-13)"
        assert_eq!(d("2025-10-13T12:00:00Z"), "m released this on 2025-10-13");
        assert_eq!(date_line("Released", None), "Released");
        let j = br#"[{"tag_name":"t","name":"n","published_at":"2026-10-11T12:00:00Z","author":{"login":"MeepingsCorner"}}]"#;
        assert_eq!(
            parse_releases(j, NOW).unwrap().lines().nth(3),
            Some("MeepingsCorner released this on 2026-10-11 (2 days ago)")
        );
    }

    #[test]
    fn entries_are_separated_by_a_blank_line_and_titled_in_green() {
        let t = parse_releases(SAMPLE.as_bytes(), NOW).unwrap();
        let l: Vec<&str> = t.lines().collect();
        assert_eq!(l[0], "^3Tremulous Revamped releases^7");
        for (i, line) in l.iter().enumerate() {
            if line.starts_with("^2") {
                assert_eq!(l[i - 1], "", "a blank line before each title: {t}");
                assert!(
                    l[i + 1].contains(" on 2026-"),
                    "a date line under each title: {t}"
                );
            }
        }
        assert_eq!(l.iter().filter(|x| x.starts_with("^2")).count(), 3);
    }

    #[test]
    fn a_blurb_that_restates_the_title_gives_way_to_the_next_line() {
        // the live 4K texture pack: its text starts with its own title
        assert_eq!(
            blurb(
                "4K texture pack for Tremulous Revamped (lossless).\n\nDownload both parts.",
                "4K texture pack"
            ),
            "Download both parts."
        );
        assert_eq!(blurb("4K texture pack", "4K texture pack"), "");
        // a word that only begins like the title is not the title
        assert_eq!(blurb("v1.0x notes", "v1.0"), "v1.0x notes");
        assert_eq!(blurb("Fixes", ""), "Fixes");
    }

    #[test]
    fn releases_text_is_plain_and_safe() {
        let t = parse_releases(SAMPLE.as_bytes(), NOW).unwrap();
        let t = t.replace("^3", "").replace("^2", "").replace("^7", "");
        assert!(
            !t.contains('^')
                && !t.contains('\x07')
                && !t.contains('*')
                && !t.contains('`')
                && !t.contains("](")
        );
        assert_eq!(
            strip_markdown("**Bold** news with [a link](https://example.org/x) and `code`"),
            "Bold news with a link and code"
        );
        assert_eq!(strip_markdown("- [x] done ^1red"), "[x] done red");
        assert_eq!(strip_markdown("[broken(link"), "[broken(link");
        // commands in a title or author are just words; the only colour
        // codes are the page's own (heading, title)
        let j = br#"[{"tag_name":"t","name":"quit; set x 1\nexec evil.cfg","author":{"login":"a\u001b[2Jb"},"published_at":"2026-10-13T06:00:00Z"}]"#;
        assert_eq!(
            parse_releases(j, NOW).unwrap(),
            "^3Tremulous Revamped releases^7\n\n^2quit; set x 1exec evil.cfg^7\na[2Jb released this on 2026-10-13 (today)"
        );
    }

    #[test]
    fn long_blurbs_are_cut_late_and_the_list_is_capped() {
        let long = format!(
            r#"[{{"tag_name":"t","name":"n","body":"{}"}}]"#,
            "word ".repeat(40)
        );
        let t = parse_releases(long.as_bytes(), NOW).unwrap();
        let l = t.lines().nth(4).unwrap();
        assert!(l.len() <= 2 + MAX_BLURB && l.ends_with("..."), "{l}");
        // cut at a word, not inside one
        assert!(l.ends_with("word..."), "{l}");
        let one = format!(
            r#"[{{"tag_name":"t","name":"n","body":"{}"}}]"#,
            "x".repeat(200)
        );
        let t = parse_releases(one.as_bytes(), NOW).unwrap();
        assert_eq!(t.lines().nth(4).unwrap().len(), 2 + MAX_BLURB);
        let many: Vec<String> = (0..30)
            .map(|i| format!(r#"{{"tag_name":"v{i}","name":"r{i}"}}"#))
            .collect();
        let t = parse_releases(format!("[{}]", many.join(",")).as_bytes(), NOW).unwrap();
        assert_eq!(t.lines().count(), 1 + 3 * MAX_RELEASES);
        // the release list fits its own budget even with every entry having a two-line blurb
        assert_eq!(MAX_RELEASE_LINES, 1 + 5 * MAX_RELEASES);
        // dec066: the 4K texture pack line of the live page (86 characters) is shown whole
        let live = "4K texture pack for Tremulous Revamped (lossless .zstd-compressed texture set, 4096 px)";
        let j = format!(r#"[{{"tag_name":"t","name":"n","body":"{live}"}}]"#);
        assert_eq!(
            parse_releases(j.as_bytes(), NOW)
                .unwrap()
                .lines()
                .nth(4)
                .unwrap(),
            format!("  {live}")
        );
    }

    // ---- dec067: both repositories, live API first, static file as the fallback

    const REPO_A: &str =
        "https://api.github.com/repos/MeepingsCorner/Tremulous-Revamped/releases?per_page=10";
    const REPO_B: &str = "https://api.github.com/repos/MeepingsCorner/Tremulous-Revamped-Release/releases?per_page=10";
    const STATIC: &str = "https://example.org/releases.json";
    const A_JSON: &str = r#"[{"tag_name":"a2","name":"A two","published_at":"2026-10-12T08:00:00Z","author":{"login":"m"}},{"tag_name":"a1","name":"A one","published_at":"2026-10-01T08:00:00Z"}]"#;
    const B_JSON: &str = r#"[{"tag_name":"b1","name":"B one","published_at":"2026-10-10T08:00:00Z","author":{"login":"m"},"body":"Public build."}]"#;
    const STATIC_JSON: &str = r#"[{"tag_name":"s1","name":"Static one","repo":"Tremulous-Revamped-Release","published_at":"2026-10-09T08:00:00Z","author":{"login":"m"}}]"#;

    fn step_with(
        s: &mut ReleasesSection,
        now: i32,
        answers: &[(&str, Option<&str>)],
        asked: &std::cell::RefCell<Vec<String>>,
    ) -> Option<String> {
        let fetch = |u: &str| {
            asked.borrow_mut().push(u.to_string());
            match answers.iter().find(|(k, _)| *k == u).and_then(|(_, v)| *v) {
                Some(b) => HttpJob::ready(Ok(b.as_bytes().to_vec())),
                None => HttpJob::ready(Err(HttpError::Status(403))),
            }
        };
        s.step(
            true,
            now,
            false,
            &fetch,
            &[REPO_A.to_string(), REPO_B.to_string()],
            STATIC,
            NOW,
        )
    }

    #[test]
    fn repo_names_come_from_the_api_address() {
        assert_eq!(repo_of(REPO_A), "Tremulous-Revamped");
        assert_eq!(repo_of(REPO_B), "Tremulous-Revamped-Release");
        assert_eq!(repo_of("https://example.org/releases.json"), "");
    }

    #[test]
    fn both_apis_ok_merge_newest_first() {
        let mut s = ReleasesSection::default();
        let asked = std::cell::RefCell::new(Vec::new());
        let t = step_with(
            &mut s,
            0,
            &[
                (REPO_A, Some(A_JSON)),
                (REPO_B, Some(B_JSON)),
                (STATIC, Some(STATIC_JSON)),
            ],
            &asked,
        )
        .unwrap();
        let titles: Vec<&str> = t.lines().filter(|l| l.starts_with("^2")).collect();
        assert_eq!(titles, ["^2A two^7", "^2B one^7", "^2A one^7"], "{t}");
        assert!(t.contains("  Public build."));
        assert!(
            !t.contains("Static one"),
            "the static file is not asked while the API answers"
        );
        assert_eq!(asked.borrow().len(), 2);
        // kept for the session: no new downloads
        step_with(&mut s, 5_000_000, &[], &asked).unwrap();
        assert_eq!(asked.borrow().len(), 2);
    }

    #[test]
    fn one_api_failing_still_shows_the_other_and_retries_later() {
        let mut s = ReleasesSection::default();
        let asked = std::cell::RefCell::new(Vec::new());
        let t = step_with(
            &mut s,
            1000,
            &[(REPO_A, Some(A_JSON)), (STATIC, Some(STATIC_JSON))],
            &asked,
        )
        .unwrap();
        assert!(
            t.contains("^2A two^7") && !t.contains("B one") && !t.contains("Static one"),
            "{t}"
        );
        step_with(&mut s, 1000 + RETRY_MS - 1, &[], &asked).unwrap();
        assert_eq!(asked.borrow().len(), 2);
        let t = step_with(
            &mut s,
            1000 + RETRY_MS,
            &[(REPO_A, Some(A_JSON)), (REPO_B, Some(B_JSON))],
            &asked,
        )
        .unwrap();
        assert!(t.contains("^2B one^7"), "{t}");
    }

    #[test]
    fn both_apis_failing_use_the_static_file() {
        let mut s = ReleasesSection::default();
        let asked = std::cell::RefCell::new(Vec::new());
        let t = step_with(&mut s, 0, &[(STATIC, Some(STATIC_JSON))], &asked).unwrap();
        assert!(
            t.contains("^2Static one^7")
                && t.contains("m released this on 2026-10-09 (4 days ago)"),
            "{t}"
        );
        assert_eq!(asked.borrow().as_slice(), [REPO_A, REPO_B, STATIC]);
    }

    #[test]
    fn all_three_failing_leave_the_unavailable_line_and_try_again_later() {
        let mut s = ReleasesSection::default();
        let asked = std::cell::RefCell::new(Vec::new());
        assert_eq!(
            step_with(&mut s, 0, &[], &asked).as_deref(),
            Some(RELEASES_UNAVAILABLE)
        );
        // a static file that is not a list is the same
        let mut s2 = ReleasesSection::default();
        assert_eq!(
            step_with(&mut s2, 0, &[(STATIC, Some("<html>"))], &asked).as_deref(),
            Some(RELEASES_UNAVAILABLE)
        );
        step_with(&mut s, RETRY_MS - 1, &[], &asked).unwrap();
        let n = asked.borrow().len();
        step_with(&mut s, RETRY_MS, &[(REPO_A, Some(A_JSON))], &asked).unwrap();
        assert!(asked.borrow().len() > n);
    }

    #[test]
    fn a_download_that_never_answers_times_out_calmly() {
        let mut s = ReleasesSection::default();
        let chans: Vec<_> = (0..2)
            .map(|_| std::sync::mpsc::channel::<HttpResult>())
            .collect();
        let rxs = std::cell::RefCell::new(
            chans
                .into_iter()
                .map(|(tx, rx)| (tx, Some(rx)))
                .collect::<Vec<_>>(),
        );
        let fetch = |_: &str| {
            HttpJob::pending(
                rxs.borrow_mut()
                    .iter_mut()
                    .find_map(|c| c.1.take())
                    .expect("a channel"),
            )
        };
        let urls = [REPO_A.to_string(), REPO_B.to_string()];
        assert_eq!(s.step(true, 0, false, &fetch, &urls, STATIC, NOW), None);
        assert_eq!(
            s.step(false, 11_000, true, &fetch, &urls, STATIC, NOW)
                .as_deref(),
            Some(RELEASES_UNAVAILABLE)
        );
    }

    #[test]
    fn empty_and_broken_release_answers() {
        assert_eq!(
            parse_releases(b"[]", NOW).unwrap(),
            "^3Tremulous Revamped releases^7\nNo releases yet."
        );
        for bad in [
            &b"{\"message\":\"rate limit\"}"[..],
            b"<html>",
            b"",
            b"[{\"tag_name\":",
            b"null",
        ] {
            assert_eq!(parse_releases(bad, NOW), None);
            assert_eq!(
                releases_from_result(&Ok(bad.to_vec()), NOW),
                RELEASES_UNAVAILABLE
            );
        }
        assert_eq!(
            releases_from_result(&Err(HttpError::Status(403)), NOW),
            "Release list unavailable right now"
        );
        assert_eq!(
            releases_from_result(&Err(HttpError::Timeout), NOW),
            RELEASES_UNAVAILABLE
        );
    }

    #[test]
    fn the_page_joins_both_parts() {
        assert_eq!(page_text(Some("A"), Some("B")), "A\n\nB");
        assert_eq!(page_text(None, Some("B")), "Retrieving...\n\nB");
    }

    #[test]
    fn a_long_news_file_comes_first_and_never_hides_the_releases() {
        let news = "item\n".repeat(80);
        let l = page_lines(&news, "^3Tremulous Revamped releases^7\n\n^2v1^7");
        assert_eq!(l[0], "item");
        assert_eq!(l[MAX_NEWS_LINES - 1], "item");
        assert_eq!(
            &l[MAX_NEWS_LINES..],
            ["", "^3Tremulous Revamped releases^7", "", "^2v1^7"]
        );
        let l = page_lines("^2News^7 text", "R");
        assert_eq!(l, ["^2News^7 text", "", "R"]);
        // a news text ending in newlines still has one blank line before the list
        assert_eq!(page_lines("A\n\n", "R"), ["A", "", "R"]);
    }

    // ---- the same release in both repositories is listed once

    /// the live answers of 2026-10-14 (trimmed to the fields the page reads):
    /// the source repository's "4K texture pack" and the Release repository's
    /// v1.0, which ships the same two texture files (same sha256) plus the game
    const LIVE_SRC: &str = r#"[{"tag_name":"hd-textures-v1","name":"4K texture pack","published_at":"2026-10-10T12:42:52Z","author":{"login":"MeepingsCorner"},
      "assets":[{"name":"tremulous-revamped-hd-textures-part1of2.zip","digest":"sha256:a3fb"},{"name":"tremulous-revamped-hd-textures-part2of2.zip","digest":"sha256:3ca0"}],
      "body":"4K texture pack for Tremulous Revamped (lossless, zstd-compressed mips; see README).\n\nDownload both parts and unzip them into the same folder, so you get one `hd` folder."}]"#;
    const LIVE_REL: &str = r#"[{"tag_name":"v1.0","name":"Tremulous Revamped v1.0 (Windows x64)","published_at":"2026-10-10T13:02:38Z","author":{"login":"MeepingsCorner"},
      "assets":[{"name":"a","digest":"sha256:a3fb"},{"name":"b","digest":"sha256:3ca0"},{"name":"Tremulous-Revamped-win64.zip","digest":"sha256:effa"}],
      "body":"Download the game from [Releases](https://github.com/MeepingsCorner/Tremulous-Revamped-Release/releases/latest).\n\n1. Download it."}]"#;

    fn both(src: &str, rel: &str) -> String {
        let mut s = ReleasesSection::default();
        let asked = std::cell::RefCell::new(Vec::new());
        step_with(
            &mut s,
            0,
            &[(REPO_A, Some(src)), (REPO_B, Some(rel))],
            &asked,
        )
        .unwrap()
    }

    #[test]
    fn the_live_answers_show_each_release_once_with_the_4k_pack_named_once() {
        let t = both(LIVE_SRC, LIVE_REL);
        assert_eq!(t.matches("4K texture pack").count(), 1, "{t}");
        assert_eq!(t.matches("Tremulous Revamped v1.0").count(), 1, "{t}");
        let want = "^3Tremulous Revamped releases^7\n\
\n^2Tremulous Revamped v1.0 (Windows x64)^7\nMeepingsCorner released this on 2026-10-10 (3 days ago)\n  Download the game from Releases.\n\
\n^24K texture pack^7\nMeepingsCorner released this on 2026-10-10 (3 days ago)\n  Download both parts and unzip them into the same folder, so you get one hd folder.";
        assert_eq!(t, want);
    }

    #[test]
    fn a_release_published_in_both_repositories_is_listed_once_from_the_release_repo() {
        // same tag
        let a = r#"[{"tag_name":"v1.1","name":"v1.1 source","published_at":"2026-10-12T08:00:00Z","body":"From the source repo."}]"#;
        let b = r#"[{"tag_name":"V1.1","name":"Tremulous Revamped v1.1","published_at":"2026-10-12T09:00:00Z","body":"From the release repo."}]"#;
        let t = both(a, b);
        assert!(
            t.contains("From the release repo.") && !t.contains("From the source repo."),
            "{t}"
        );
        // same title (letters and digits, any case), other tags
        let a = r#"[{"tag_name":"hd-v2","name":"4K Texture-Pack","published_at":"2026-10-11T08:00:00Z","body":"src"}]"#;
        let b = r#"[{"tag_name":"tex2","name":"4k texture pack","published_at":"2026-10-12T08:00:00Z","body":"rel"}]"#;
        let t = both(a, b);
        assert_eq!(t.matches("^2").count(), 1, "{t}");
        assert!(t.contains("rel") && !t.contains("  src"), "{t}");
        // the same files under different names and tags
        let a = r#"[{"tag_name":"x","name":"Pack","published_at":"2026-10-11T08:00:00Z","assets":[{"digest":"sha256:1"},{"digest":"sha256:2"}]}]"#;
        let b = r#"[{"tag_name":"y","name":"Textures","published_at":"2026-10-11T09:00:00Z","assets":[{"digest":"sha256:2"},{"digest":"SHA256:1"}]}]"#;
        let t = both(a, b);
        assert!(t.contains("^2Textures^7") && !t.contains("^2Pack^7"), "{t}");
    }

    #[test]
    fn different_releases_stay_apart() {
        // the same tag a year apart, a superset of files, different everything
        let a = r#"[{"tag_name":"v1","name":"One","published_at":"2025-09-01T08:00:00Z","assets":[{"digest":"sha256:1"}]},
                    {"tag_name":"p","name":"Pack","published_at":"2026-10-10T08:00:00Z","assets":[{"digest":"sha256:1"}]}]"#;
        let b = r#"[{"tag_name":"v1","name":"One again","published_at":"2026-10-10T08:00:00Z","assets":[{"digest":"sha256:1"},{"digest":"sha256:9"}]}]"#;
        let t = both(a, b);
        assert_eq!(t.matches("^2").count(), 3, "{t}");
        // no tag, title or files at all: nothing to call the same
        let a = r#"[{"name":"A","published_at":"2026-10-10T08:00:00Z"}]"#;
        let b = r#"[{"name":"B","published_at":"2026-10-10T08:00:00Z"}]"#;
        assert_eq!(both(a, b).matches("^2").count(), 2);
    }

    #[test]
    fn a_static_file_listing_a_release_twice_shows_it_once() {
        let j = br#"[{"tag_name":"v1","name":"One","published_at":"2026-10-10T08:00:00Z","repo":"Tremulous-Revamped-Release"},
                     {"tag_name":"v1","name":"One","published_at":"2026-10-10T08:00:00Z","repo":"Tremulous-Revamped"}]"#;
        assert_eq!(
            parse_releases(j, NOW).unwrap().matches("^2One^7").count(),
            1
        );
    }

    fn render(r: &HttpResult) -> (String, bool) {
        (from_result(DEFAULT_NEWS_URL, r), news_ok(r))
    }

    #[test]
    fn a_section_downloads_once_and_keeps_good_answers_for_the_session() {
        let mut s = Section::default();
        let starts = std::cell::Cell::new(0);
        let start = || {
            starts.set(starts.get() + 1);
            HttpJob::ready(Ok(b"News!".to_vec()))
        };
        let t = || "timeout".to_string();
        assert_eq!(
            s.step(true, 0, false, &start, &render, &t).as_deref(),
            Some("News!")
        );
        // opened again much later: still the kept answer, no second download
        assert_eq!(
            s.step(true, 10_000_000, false, &start, &render, &t)
                .as_deref(),
            Some("News!")
        );
        assert_eq!(starts.get(), 1);
    }

    #[test]
    fn a_failed_section_tries_again_only_after_a_minute() {
        let mut s = Section::default();
        let starts = std::cell::Cell::new(0);
        let start = || {
            starts.set(starts.get() + 1);
            HttpJob::ready(Err(HttpError::Status(404)))
        };
        let t = || "timeout".to_string();
        let first = s.step(true, 1000, false, &start, &render, &t).unwrap();
        assert!(first.contains("no longer serves news"));
        s.step(true, 1000 + RETRY_MS - 1, false, &start, &render, &t)
            .unwrap();
        assert_eq!(starts.get(), 1);
        s.step(true, 1000 + RETRY_MS, false, &start, &render, &t)
            .unwrap();
        assert_eq!(starts.get(), 2);
    }

    #[test]
    fn a_section_that_never_answers_says_retrieving_then_times_out_calmly() {
        let mut s = Section::default();
        // a job whose request never answers
        let (tx, rx) = std::sync::mpsc::channel::<HttpResult>();
        let _keep = tx;
        let rx = std::cell::RefCell::new(Some(rx));
        let start = || HttpJob::pending(rx.borrow_mut().take().unwrap());
        let t = || "timed out calmly".to_string();
        assert_eq!(s.step(true, 0, false, &start, &render, &t), None);
        assert_eq!(s.step(false, 100, false, &start, &render, &t), None);
        assert_eq!(
            s.step(false, 10_001, true, &start, &render, &t).as_deref(),
            Some("timed out calmly")
        );
    }
}
