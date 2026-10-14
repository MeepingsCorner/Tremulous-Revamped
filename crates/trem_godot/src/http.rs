//! A small HTTP GET for the client (dec065), on Godot's own HTTP
//! client (`HTTPClient`) and its mbedTLS, so the build gains no networking or
//! TLS crate: the engine already ships both, with its bundled certificate
//! list (`TLSOptions.client()` verifies the server against it).
//!
//! The legacy client did its transfers with libcurl (`cl_curl.c`); this is
//! the port's counterpart. `HttpJob::get` starts a request, `HttpJob::poll`
//! (called once a frame, on the main thread) advances it without ever
//! blocking and hands back the finished result. Limits: a timeout for the
//! whole request, a redirect limit (an https request never follows a
//! redirect to plain http), and a cap on the body that either fails the
//! request (`Overflow::Fail`, for files) or cuts the body short
//! (`Overflow::Truncate`, for text such as the news).
//!
//! First user: the News page (`news.rs`). Map downloads (`sv_dlURL`) could use
//! the same job later; they are not wired to it.

use godot::classes::http_client::{Method, Status};
use godot::classes::{HttpClient, TlsOptions};
use godot::prelude::*;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// The client's name on the wire.
pub const USER_AGENT: &str = concat!("Tremulous Revamped/", env!("CARGO_PKG_VERSION"));

/// What to do with a body longer than `max_bytes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    /// the request fails with `HttpError::TooLarge`
    Fail,
    /// the first `max_bytes` bytes are the result
    Truncate,
}

#[derive(Clone, Copy, Debug)]
pub struct HttpOpts {
    /// for the whole request (connect, send, redirects, body)
    pub timeout: Duration,
    pub max_bytes: usize,
    pub overflow: Overflow,
    pub max_redirects: u32,
    /// the `Accept` header, if any
    pub accept: Option<&'static str>,
    /// only a plain-text answer is wanted: a `Content-Type` other than
    /// `text/plain` fails the request (`HttpError::NotText`); no header is fine
    pub text_only: bool,
}

impl Default for HttpOpts {
    fn default() -> Self {
        HttpOpts {
            timeout: Duration::from_secs(5),
            max_bytes: 64 * 1024,
            overflow: Overflow::Fail,
            max_redirects: 3,
            accept: None,
            text_only: false,
        }
    }
}

/// Why a request produced no body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// not an `http://` or `https://` URL
    BadUrl,
    /// the server answered with this status (404, 500, ...)
    Status(u16),
    Timeout,
    /// DNS, connect, TLS (including a certificate that does not verify),
    /// protocol or read error (the text is for logs)
    Network(String),
    /// the body was longer than `max_bytes` (`Overflow::Fail`)
    TooLarge,
    /// `text_only` and the server named another type (an HTML error page)
    NotText(String),
}

/// Is this `Content-Type` (if the server sent one) plain text?
pub fn is_plain_text(content_type: Option<&str>) -> bool {
    match content_type {
        None => true,
        Some(t) => t
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("text/plain"),
    }
}

pub type HttpResult = Result<Vec<u8>, HttpError>;

/// A request in flight (or an already known answer, for tests).
pub struct HttpJob {
    rx: Option<Receiver<HttpResult>>,
    done: Option<HttpResult>,
    live: Option<Box<Live>>,
}

impl HttpJob {
    /// Start `GET url`. Nothing happens on the network until the first `poll`.
    pub fn get(url: &str, opts: HttpOpts) -> HttpJob {
        let Some(u) = Url::parse(url) else {
            return HttpJob::ready(Err(HttpError::BadUrl));
        };
        let live = Live {
            url: u,
            opts,
            deadline: Instant::now() + opts.timeout,
            redirects: 0,
            client: None,
            phase: Phase::Connecting,
            body: Vec::new(),
        };
        HttpJob {
            rx: None,
            done: None,
            live: Some(Box::new(live)),
        }
    }

    /// A job waiting on this channel (tests: one that never answers).
    #[cfg(test)]
    pub fn pending(rx: Receiver<HttpResult>) -> HttpJob {
        HttpJob {
            rx: Some(rx),
            done: None,
            live: None,
        }
    }

    /// A job that has already finished with `r` (canned answers in tests).
    pub fn ready(r: HttpResult) -> HttpJob {
        HttpJob {
            rx: None,
            done: Some(r),
            live: None,
        }
    }

    /// The result once the request has finished, else `None`. Never blocks.
    pub fn poll(&mut self) -> Option<&HttpResult> {
        if self.done.is_none() {
            if let Some(live) = self.live.as_mut() {
                if let Some(r) = live.step() {
                    match &r {
                        Ok(b) => godot_print!("http: GET {} -> {} bytes", live.url.host, b.len()),
                        Err(e) => godot_print!("http: GET {} failed: {e:?}", live.url.host),
                    }
                    self.done = Some(r);
                    self.live = None; // drops (closes) the client
                }
            } else if let Some(rx) = &self.rx {
                match rx.try_recv() {
                    Ok(r) => self.done = Some(r),
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        self.done = Some(Err(HttpError::Network("worker ended".into())))
                    }
                }
            }
        }
        self.done.as_ref()
    }
}

/// Only http and https are fetched (no file:, ftp:, ...).
pub fn is_http_url(url: &str) -> bool {
    let l = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://"))
        && url.len() > 8
        && url.bytes().all(|b| b.is_ascii_graphic())
}

/// Cut or reject a body that is longer than the cap (`body` may be longer
/// than `max_bytes`; the reader stops soon after the cap).
pub fn apply_cap(mut body: Vec<u8>, opts: &HttpOpts) -> HttpResult {
    if body.len() > opts.max_bytes {
        match opts.overflow {
            Overflow::Fail => return Err(HttpError::TooLarge),
            Overflow::Truncate => body.truncate(opts.max_bytes),
        }
    }
    Ok(body)
}

// ---------------------------------------------------------------------- URLs

#[derive(Clone, Debug, PartialEq, Eq)]
struct Url {
    tls: bool,
    host: String,
    port: i32,
    /// path and query, always starting with `/` (no fragment)
    path: String,
}

impl Url {
    fn parse(url: &str) -> Option<Url> {
        if !is_http_url(url) {
            return None;
        }
        let (tls, rest) = match url.split_once("://")? {
            (s, r) if s.eq_ignore_ascii_case("https") => (true, r),
            (_, r) => (false, r),
        };
        let rest = rest.split('#').next().unwrap_or("");
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let (auth, tail) = rest.split_at(end);
        if auth.contains('@') {
            return None; // no user:password@ in addresses
        }
        let path = if tail.starts_with('/') {
            tail.to_string()
        } else {
            format!("/{tail}")
        };
        let (host, port) = if let Some(r) = auth.strip_prefix('[') {
            // [v6]:port
            let (h, after) = r.split_once(']')?;
            (h, after.strip_prefix(':').filter(|p| !p.is_empty()))
        } else {
            match auth.rsplit_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (auth, None),
            }
        };
        if host.is_empty() {
            return None;
        }
        let port = match port {
            Some(p) => p.parse::<u16>().ok().filter(|&p| p > 0)? as i32,
            None if tls => 443,
            None => 80,
        };
        Some(Url {
            tls,
            host: host.to_string(),
            port,
            path,
        })
    }

    /// The address a `Location` header points to, seen from this one.
    fn join(&self, loc: &str) -> Option<Url> {
        let scheme = if self.tls { "https" } else { "http" };
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let default_port = if self.tls { 443 } else { 80 };
        let origin = if self.port == default_port {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}:{}", self.port)
        };
        let scheme_end = loc.find([':', '/', '?', '#']);
        if scheme_end.is_some_and(|i| loc.as_bytes()[i] == b':') && !is_http_url(loc) {
            return None; // another scheme (ftp:, file:, ...)
        }
        let abs = if is_http_url(loc) {
            loc.to_string()
        } else if let Some(r) = loc.strip_prefix("//") {
            format!("{scheme}://{r}")
        } else if loc.starts_with('/') {
            format!("{origin}{loc}")
        } else {
            let dir = &self.path[..self.path.rfind('/').map(|i| i + 1).unwrap_or(1)];
            format!("{origin}{dir}{loc}")
        };
        Url::parse(&abs)
    }
}

// ------------------------------------------------------------------- the job

enum Phase {
    /// waiting for DNS, TCP and (https) the TLS handshake
    Connecting,
    /// request sent, waiting for the status line and headers
    Requesting,
    /// reading the body
    Reading,
}

struct Live {
    url: Url,
    opts: HttpOpts,
    deadline: Instant,
    redirects: u32,
    client: Option<Gd<HttpClient>>,
    phase: Phase,
    body: Vec<u8>,
}

/// What a `HTTPClient` status that is neither progress nor success means.
fn failure(s: Status) -> Option<HttpError> {
    let what = if s == Status::CANT_RESOLVE {
        "cannot resolve host"
    } else if s == Status::CANT_CONNECT {
        "cannot connect"
    } else if s == Status::CONNECTION_ERROR {
        "connection error"
    } else if s == Status::TLS_HANDSHAKE_ERROR {
        "TLS handshake failed (certificate not trusted, expired or wrong host?)"
    } else if s == Status::DISCONNECTED {
        "disconnected"
    } else {
        return None;
    };
    Some(HttpError::Network(what.to_string()))
}

/// Where a redirect from `cur` leads, or why it is not followed: a missing
/// or unusable `Location` (or one with another scheme) is the redirect
/// status itself, too many hops is a network error, and an HTTPS request
/// never follows a redirect to plain HTTP (http -> https is fine).
fn next_hop(
    cur: &Url,
    code: u16,
    location: Option<&str>,
    redirects: u32,
    max_redirects: u32,
) -> Result<Url, HttpError> {
    let Some(next) = location.and_then(|l| cur.join(l)) else {
        return Err(HttpError::Status(code));
    };
    if cur.tls && !next.tls {
        return Err(HttpError::Network(
            "redirect from https to http refused".into(),
        ));
    }
    if redirects >= max_redirects {
        return Err(HttpError::Network("too many redirects".into()));
    }
    Ok(next)
}

/// Was the whole body read? Only when the server said how long it is
/// (`Content-Length`, else the engine reports -1) and that much came.
fn body_complete(want: i64, got: usize) -> bool {
    want >= 0 && got as i64 == want
}

fn is_redirect(code: i64) -> bool {
    matches!(code, 301 | 302 | 303 | 307 | 308)
}

impl Live {
    /// Advance as far as the network allows right now; the result once done.
    fn step(&mut self) -> Option<HttpResult> {
        for _ in 0..32 {
            if Instant::now() >= self.deadline {
                return Some(Err(HttpError::Timeout));
            }
            if self.client.is_none() {
                match self.open() {
                    Ok(c) => self.client = Some(c),
                    Err(e) => return Some(Err(e)),
                }
                self.phase = Phase::Connecting;
                self.body.clear();
            }
            let client = self.client.as_mut()?;
            let _ = client.poll();
            let status = client.get_status();
            if let Some(e) = failure(status) {
                // a server that closes the connection right after the body
                // (HTTP/1.0, "Connection: close") leaves the engine in an
                // error state with the whole body already read
                if matches!(self.phase, Phase::Reading)
                    && body_complete(client.get_response_body_length(), self.body.len())
                {
                    return Some(apply_cap(std::mem::take(&mut self.body), &self.opts));
                }
                return Some(Err(e));
            }
            match self.phase {
                Phase::Connecting => {
                    if status != Status::CONNECTED {
                        return None; // still resolving / connecting
                    }
                    let mut h = PackedStringArray::new();
                    h.push(&GString::from(&format!("User-Agent: {USER_AGENT}")));
                    h.push(&GString::from(&format!(
                        "Accept: {}",
                        self.opts.accept.unwrap_or("*/*")
                    )));
                    let err =
                        client.request(Method::GET, &GString::from(self.url.path.as_str()), &h);
                    if err != godot::global::Error::OK {
                        return Some(Err(HttpError::Network(format!("request: {err:?}"))));
                    }
                    self.phase = Phase::Requesting;
                }
                Phase::Requesting => {
                    if status == Status::REQUESTING || !client.has_response() {
                        if status == Status::CONNECTED {
                            return Some(Err(HttpError::Network("no response".into())));
                        }
                        return None;
                    }
                    let code = client.get_response_code() as i64;
                    if is_redirect(code) {
                        let mut loc = None;
                        for h in client.get_response_headers().as_slice() {
                            let h = h.to_string();
                            if let Some((n, v)) = h.split_once(':') {
                                if n.trim().eq_ignore_ascii_case("location") {
                                    loc = Some(v.trim().to_string());
                                }
                            }
                        }
                        match next_hop(
                            &self.url,
                            code as u16,
                            loc.as_deref(),
                            self.redirects,
                            self.opts.max_redirects,
                        ) {
                            Ok(next) => self.url = next,
                            Err(e) => return Some(Err(e)),
                        }
                        self.redirects += 1;
                        self.client = None; // a fresh connection to the new address
                        continue;
                    }
                    if code >= 400 || code < 200 {
                        return Some(Err(HttpError::Status(code as u16)));
                    }
                    if self.opts.text_only {
                        let mut ty = None;
                        for h in client.get_response_headers().as_slice() {
                            let h = h.to_string();
                            if let Some((n, v)) = h.split_once(':') {
                                if n.trim().eq_ignore_ascii_case("content-type") {
                                    ty = Some(v.trim().to_string());
                                }
                            }
                        }
                        if !is_plain_text(ty.as_deref()) {
                            return Some(Err(HttpError::NotText(ty.unwrap_or_default())));
                        }
                    }
                    self.phase = Phase::Reading;
                }
                Phase::Reading => {
                    if status == Status::BODY {
                        let chunk = client.read_response_body_chunk();
                        if chunk.is_empty() {
                            return None; // nothing more buffered yet
                        }
                        self.body.extend_from_slice(chunk.as_slice());
                        if self.body.len() > self.opts.max_bytes {
                            return Some(apply_cap(std::mem::take(&mut self.body), &self.opts));
                        }
                    } else if status == Status::CONNECTED {
                        return Some(apply_cap(std::mem::take(&mut self.body), &self.opts));
                    } else {
                        return None;
                    }
                }
            }
        }
        None
    }

    /// A client connecting to the current address (verifying the server
    /// against the engine's certificate list for https).
    fn open(&self) -> Result<Gd<HttpClient>, HttpError> {
        let mut c = HttpClient::new_gd();
        let host = GString::from(self.url.host.as_str());
        let err = if self.url.tls {
            // never fall back to unverified TLS: no options object, no request
            let Some(tls) = TlsOptions::client() else {
                return Err(HttpError::Network("no TLS client options".into()));
            };
            c.connect_to_host_ex(&host)
                .port(self.url.port)
                .tls_options(Some(&tls))
                .done()
        } else {
            c.connect_to_host_ex(&host).port(self.url.port).done()
        };
        if err != godot::global::Error::OK {
            return Err(HttpError::Network(format!("connect: {err:?}")));
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_urls() {
        assert!(is_http_url("http://tremulous.net/clientnews.txt"));
        assert!(is_http_url("HTTPS://example.org/a"));
        assert!(!is_http_url("file:///etc/passwd"));
        assert!(!is_http_url("ftp://example.org/x"));
        assert!(!is_http_url("http://"));
        assert!(!is_http_url(""));
        assert!(!is_http_url("http://a b/c"));
        assert!(!is_http_url("http://a/\n"));
        assert!(!is_http_url("é"));
    }

    #[test]
    fn a_bad_url_fails_without_touching_the_engine() {
        let mut j = HttpJob::get("gopher://x/y", HttpOpts::default());
        assert_eq!(j.poll(), Some(&Err(HttpError::BadUrl)));
        let mut j = HttpJob::get("http://user:pw@x/y", HttpOpts::default());
        assert_eq!(j.poll(), Some(&Err(HttpError::BadUrl)));
    }

    #[test]
    fn urls_split_into_host_port_path() {
        let u = Url::parse("https://api.github.com/repos/a/b/releases?per_page=10#frag").unwrap();
        assert_eq!(
            (u.tls, u.host.as_str(), u.port, u.path.as_str()),
            (
                true,
                "api.github.com",
                443,
                "/repos/a/b/releases?per_page=10"
            )
        );
        let u = Url::parse("http://tremulous.net/clientnews.txt").unwrap();
        assert_eq!((u.tls, u.port), (false, 80));
        assert_eq!(Url::parse("http://h:8080").unwrap().path, "/");
        assert_eq!(Url::parse("http://h?x=1").unwrap().path, "/?x=1");
        assert_eq!(Url::parse("http://[::1]:99/p").unwrap().host, "::1");
        assert_eq!(Url::parse("http://[::1]:99/p").unwrap().port, 99);
        assert!(Url::parse("http://h:0/").is_none());
        assert!(Url::parse("http://h:99999/").is_none());
        assert!(Url::parse("http://:80/").is_none());
    }

    #[test]
    fn redirect_targets_are_resolved_against_the_current_address() {
        let u = Url::parse("http://a.org/x/y.txt").unwrap();
        assert_eq!(u.join("https://b.org/z").unwrap().host, "b.org");
        assert!(u.join("https://b.org/z").unwrap().tls);
        assert_eq!(u.join("/n").unwrap().path, "/n");
        assert_eq!(u.join("z.txt").unwrap().path, "/x/z.txt");
        assert_eq!(u.join("//c.org/q").unwrap().host, "c.org");
        assert!(u.join("ftp://c.org/q").is_none());
        let p = Url::parse("http://a.org:81/x").unwrap();
        assert_eq!(p.join("/n").unwrap().port, 81);
    }

    fn hop(from: &str, loc: &str) -> Result<Url, HttpError> {
        next_hop(&Url::parse(from).unwrap(), 302, Some(loc), 0, 3)
    }

    #[test]
    fn an_https_request_never_follows_a_redirect_to_plain_http() {
        let refused = Err(HttpError::Network(
            "redirect from https to http refused".into(),
        ));
        assert_eq!(hop("https://a.org/x", "http://b.org/y"), refused);
        assert_eq!(hop("https://a.org/x", "HTTP://b.org/y"), refused);
        // protocol-relative keeps https; relative paths stay on the same scheme
        assert!(hop("https://a.org/x", "//b.org/y").unwrap().tls);
        assert!(hop("https://a.org/x/y", "z").unwrap().tls);
        assert!(hop("https://a.org/x", "/z").unwrap().tls);
        assert!(hop("https://a.org/x", "https://b.org/y").unwrap().tls);
    }

    #[test]
    fn http_may_redirect_to_https_or_stay_on_http() {
        assert!(hop("http://a.org/x", "https://b.org/y").unwrap().tls);
        assert!(!hop("http://a.org/x", "http://b.org/y").unwrap().tls);
        assert!(!hop("http://a.org/x", "/y").unwrap().tls);
    }

    #[test]
    fn a_redirect_to_another_scheme_or_without_a_location_is_refused() {
        for from in ["http://a.org/x", "https://a.org/x"] {
            for loc in [
                "ftp://b.org/y",
                "file:///etc/passwd",
                "gopher://b/y",
                "javascript:alert(1)",
            ] {
                assert_eq!(
                    hop(from, loc),
                    Err(HttpError::Status(302)),
                    "{from} -> {loc}"
                );
            }
            assert_eq!(
                next_hop(&Url::parse(from).unwrap(), 302, None, 0, 3),
                Err(HttpError::Status(302))
            );
        }
    }

    #[test]
    fn a_relative_location_is_resolved_against_the_current_address() {
        let u = hop("https://a.org:8443/d/e.txt?q=1", "f.txt?r=2").unwrap();
        assert_eq!(
            (u.tls, u.host.as_str(), u.port, u.path.as_str()),
            (true, "a.org", 8443, "/d/f.txt?r=2")
        );
        let u = hop("http://a.org/d/e.txt", "/top").unwrap();
        assert_eq!(
            (u.host.as_str(), u.port, u.path.as_str()),
            ("a.org", 80, "/top")
        );
    }

    #[test]
    fn the_redirect_limit_counts_every_hop() {
        let cur = Url::parse("https://a.org/x").unwrap();
        assert!(next_hop(&cur, 302, Some("/y"), 2, 3).is_ok());
        assert_eq!(
            next_hop(&cur, 302, Some("/y"), 3, 3),
            Err(HttpError::Network("too many redirects".into()))
        );
    }

    #[test]
    fn the_timeout_and_the_cap_still_apply_after_redirects() {
        // the deadline is set once for the whole request, so a job that has
        // already followed redirects times out without touching the engine
        let opts = HttpOpts::default();
        let mut live = Live {
            url: Url::parse("https://a.org/y").unwrap(),
            opts,
            deadline: Instant::now() - Duration::from_secs(1),
            redirects: 2,
            client: None,
            phase: Phase::Connecting,
            body: Vec::new(),
        };
        assert_eq!(live.step(), Some(Err(HttpError::Timeout)));
        // the cap is checked on the final body with the same options
        let small = HttpOpts {
            max_bytes: 3,
            ..opts
        };
        assert_eq!(
            apply_cap(b"abcd".to_vec(), &small),
            Err(HttpError::TooLarge)
        );
    }

    #[test]
    fn ready_jobs_answer_at_once_and_keep_answering() {
        let mut j = HttpJob::ready(Ok(b"hi".to_vec()));
        assert_eq!(j.poll(), Some(&Ok(b"hi".to_vec())));
        assert_eq!(j.poll(), Some(&Ok(b"hi".to_vec())));
    }

    #[test]
    fn the_size_cap_fails_or_truncates() {
        let mut o = HttpOpts {
            max_bytes: 4,
            ..HttpOpts::default()
        };
        assert_eq!(apply_cap(b"abcd".to_vec(), &o), Ok(b"abcd".to_vec()));
        assert_eq!(apply_cap(b"abcde".to_vec(), &o), Err(HttpError::TooLarge));
        o.overflow = Overflow::Truncate;
        assert_eq!(apply_cap(b"abcde".to_vec(), &o), Ok(b"abcd".to_vec()));
    }

    #[test]
    fn engine_failure_states_map_to_network_errors() {
        for s in [
            Status::CANT_RESOLVE,
            Status::CANT_CONNECT,
            Status::CONNECTION_ERROR,
            Status::TLS_HANDSHAKE_ERROR,
        ] {
            assert!(matches!(failure(s), Some(HttpError::Network(_))));
        }
        assert!(failure(Status::BODY).is_none());
        assert!(failure(Status::CONNECTED).is_none());
        assert!(is_redirect(302) && !is_redirect(304) && !is_redirect(200));
    }

    #[test]
    fn a_body_is_whole_only_when_its_length_was_announced_and_reached() {
        assert!(body_complete(193, 193));
        assert!(body_complete(0, 0));
        assert!(!body_complete(193, 100));
        // no Content-Length: a cut connection cannot be told from the end
        assert!(!body_complete(-1, 100));
    }

    #[test]
    fn only_plain_text_passes_the_text_only_check() {
        assert!(is_plain_text(None));
        assert!(is_plain_text(Some("text/plain")));
        assert!(is_plain_text(Some("Text/Plain; charset=utf-8")));
        assert!(!is_plain_text(Some("text/html; charset=utf-8")));
        assert!(!is_plain_text(Some("application/json")));
        assert!(!is_plain_text(Some("application/octet-stream")));
        assert!(!is_plain_text(Some("")));
    }

    #[test]
    fn the_user_agent_names_the_client() {
        assert!(USER_AGENT.starts_with("Tremulous Revamped/"));
    }
}
