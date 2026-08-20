//! The shared polite-fetch layer. **Every** internship source adapter goes through this and
//! no adapter builds its own `reqwest::Client`.
//!
//! That rule is not stylistic. Politeness is a property of the *process*, not of any one
//! request: a per-host rate limit only limits anything if every request to that host queues
//! behind the same limiter, and `robots.txt` only constrains anything if every request checks
//! the same cache. A second client anywhere in the tree silently opts out of both while
//! looking exactly like code that opted in. So the `reqwest::Client` is private to this module,
//! [`PoliteClient`] exposes no accessor that hands it back, and
//! `sources::adapters_do_not_build_their_own_http_client` reads the adapter sources at compile
//! time and fails the build if any of them so much as names `reqwest`.
//!
//! # What "polite" means here, concretely
//!
//! From the root `CLAUDE.md` § Scraping rules, which are binding and not defaults to re-weigh
//! per source:
//!
//! - **Identify honestly.** [`USER_AGENT`] names the project and links to it. There is no
//!   browser-fingerprint spoofing, no proxy rotation, no CAPTCHA solving, and no cookie jar —
//!   nothing here is capable of pretending to be something it is not.
//! - **Respect `robots.txt`.** Fetched once per host, cached for the life of the process, and
//!   consulted before every request. A disallowed path is not fetched at all, and the caller
//!   gets [`FetchError::RobotsDisallowed`] so it can record `Skipped` rather than `Failed` —
//!   a source we correctly declined to fetch is not a broken source.
//! - **Rate limit per host, not globally.** 485 Greenhouse boards are one host; a global
//!   limiter would serialize the entire run for no politeness benefit. See [`RateLimiter`].
//! - **Fail fast.** One attempt. A 403, a 429, a timeout or a parse failure ends that request,
//!   and there is deliberately no retry and no backoff loop anywhere in this file. A source
//!   that has decided to push back is a source to drop for this run and log.
//!
//! # Conditional GETs
//!
//! `raw.githubusercontent.com` answers `If-None-Match` with **304 and 0 bytes** against 10.8 MB
//! unconditionally (`docs/INTERNSHIP_SCRAPING.md` § A.2), which is the difference between
//! polling Simplify hourly being free and being rude. [`PoliteClient`] remembers each URL's
//! `ETag`/`Last-Modified` in memory and replays them automatically, so an adapter gets the
//! benefit without threading validators through its own code. The cache is per-process and
//! deliberately not persisted — persistence is database work, and the coordinator owns that.
//!
//! # The redirect trap
//!
//! `docs/INTERNSHIP_SCRAPING.md` § D.2: a **dead** Greenhouse job's public HTML URL redirects
//! to the board root with **HTTP 200**. Checking liveness by status code on a public URL
//! therefore concludes that every dead posting is alive, forever, with no error to alert on.
//! Two things here exist because of that: [`PoliteResponse::final_url`] is always recorded, and
//! [`PoliteResponse::redirected_away_from`] lets a caller treat "answered 200, but not at the
//! URL I asked for" as the closure signal it actually is. The real fix is to ask the API rather
//! than the HTML, which is what every adapter in `sources/` does.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep_until};

// ------------------------------------------------------------------------------------------
// Constants
// ------------------------------------------------------------------------------------------

/// How this collector identifies itself. Honest: it names the project, links to the source,
/// and says how to reach the operator. A site that wants to block us can, and that is the
/// point — a user agent we would be embarrassed to have logged is a user agent that is lying.
pub const USER_AGENT: &str =
    "personal-website-internship-collector/0.1 (+https://github.com/terrificjesse/personal-website; contact via GitHub)";

/// The `User-agent:` group we obey in `robots.txt`. We match the wildcard group, never a named
/// bot's group — claiming to be Googlebot to inherit its allowances is exactly the evasion the
/// project rules forbid.
pub const ROBOTS_USER_AGENT: &str = "*";

/// Minimum gap between two requests to the same host. Deliberately conservative: no vendor in
/// `docs/INTERNSHIP_SCRAPING.md` publishes a read rate limit, so this is chosen to be polite
/// rather than to sit just under a known ceiling. `robots.txt` `Crawl-delay` raises it per
/// host when a host asks for more (`api.lever.co` asks for 1s), never lowers it.
pub const DEFAULT_HOST_DELAY: Duration = Duration::from_millis(1_000);

/// Hard ceiling on one request, connect through last byte of body. `reqwest`'s own timeout,
/// so a hung socket cannot wedge a run.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Connect-phase ceiling, inside [`REQUEST_TIMEOUT`].
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Redirects to follow. Enough for the http→https and apex→www hops that are normal, few
/// enough that a redirect loop ends quickly.
pub const MAX_REDIRECTS: usize = 5;

/// Ceiling on a `robots.txt` body. A "robots.txt" measured in megabytes is a misrouted request
/// or a catch-all HTML page, and parsing it as rules would produce nonsense.
pub const MAX_ROBOTS_BYTES: usize = 512 * 1024;

// ------------------------------------------------------------------------------------------
// Errors
// ------------------------------------------------------------------------------------------

/// Why a fetch produced nothing.
///
/// The variants are split by **what the caller should record**, not by where in the stack the
/// failure happened. [`FetchError::RobotsDisallowed`] is the only one that means "we behaved
/// correctly and chose not to fetch", and it is the only one an adapter should turn into
/// [`SourceOutcome::Skipped`](crate::internships::models::SourceOutcome::Skipped).
#[derive(Debug, Clone)]
pub enum FetchError {
    /// `robots.txt` forbids this path for our user agent. Not an error; a correct refusal.
    RobotsDisallowed { url: String, rule: String },
    /// `robots.txt` could not be read (network failure, or a 5xx). Per RFC 9309 § 2.3.1.4 an
    /// unreachable `robots.txt` means *complete disallow* — we cannot verify permission, so we
    /// do not fetch. A 4xx (including the 401 `api.ashbyhq.com` actually returns) is
    /// "unavailable" under § 2.3.1.3 and permits fetching; that case never reaches here.
    RobotsUnreachable { host: String, detail: String },
    /// The host answered 403 or 429. **We stop rather than retrying harder.**
    Blocked {
        url: String,
        status: u16,
        retry_after: Option<String>,
    },
    /// Any other non-2xx.
    Status { url: String, status: u16 },
    /// Connection refused, DNS failure, TLS failure, timeout.
    Transport { url: String, detail: String },
    /// 2xx with a body we could not turn into what the caller asked for.
    Decode { url: String, detail: String },
}

impl FetchError {
    /// The URL or host the failure is about, for logging.
    pub fn target(&self) -> &str {
        match self {
            FetchError::RobotsDisallowed { url, .. }
            | FetchError::Blocked { url, .. }
            | FetchError::Status { url, .. }
            | FetchError::Transport { url, .. }
            | FetchError::Decode { url, .. } => url,
            FetchError::RobotsUnreachable { host, .. } => host,
        }
    }

    /// Whether this is a deliberate, correct refusal to fetch rather than a failure.
    ///
    /// Adapters branch on this to choose between
    /// [`SourceOutcome::Skipped`](crate::internships::models::SourceOutcome::Skipped) and
    /// [`SourceOutcome::Failed`](crate::internships::models::SourceOutcome::Failed), and the
    /// difference matters: a run-health panel that paints a robots-respecting skip red trains
    /// its reader to ignore red.
    pub fn is_refusal(&self) -> bool {
        matches!(self, FetchError::RobotsDisallowed { .. })
    }

    /// A 404 on a board list means the board is gone and the slug should be retired, which is
    /// a different decision from "this board is temporarily unreachable". Named rather than
    /// spelled `status == 404` at four call sites.
    pub fn is_not_found(&self) -> bool {
        matches!(self, FetchError::Status { status: 404, .. })
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::RobotsDisallowed { url, rule } => {
                write!(f, "robots.txt disallows {url} (rule: {rule})")
            }
            FetchError::RobotsUnreachable { host, detail } => write!(
                f,
                "robots.txt for {host} is unreachable ({detail}); treating as disallow per RFC 9309"
            ),
            FetchError::Blocked {
                url,
                status,
                retry_after,
            } => match retry_after {
                Some(after) => write!(f, "{url} returned {status} (retry-after: {after}); giving up for this run"),
                None => write!(f, "{url} returned {status}; giving up for this run"),
            },
            FetchError::Status { url, status } => write!(f, "{url} returned HTTP {status}"),
            FetchError::Transport { url, detail } => write!(f, "{url} failed: {detail}"),
            FetchError::Decode { url, detail } => write!(f, "{url} returned an unusable body: {detail}"),
        }
    }
}

impl std::error::Error for FetchError {}

// ------------------------------------------------------------------------------------------
// Responses
// ------------------------------------------------------------------------------------------

/// A body we actually fetched.
#[derive(Debug, Clone)]
pub struct PoliteResponse {
    pub status: u16,
    /// What we asked for.
    pub requested_url: String,
    /// Where we ended up. Different from `requested_url` means redirects were followed — see
    /// the module doc on the Greenhouse liveness trap.
    pub final_url: String,
    pub body: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl PoliteResponse {
    /// Whether the response came from a different path than the one requested.
    ///
    /// A dead Greenhouse job answers **200** from the board root. `status == 200` is therefore
    /// not evidence the posting exists; this is.
    pub fn redirected_away_from(&self, url: &str) -> bool {
        self.final_url != url
    }

    /// Parse the body as JSON.
    pub fn json(&self) -> Result<serde_json::Value, FetchError> {
        serde_json::from_str(&self.body).map_err(|error| FetchError::Decode {
            url: self.final_url.clone(),
            detail: error.to_string(),
        })
    }
}

/// The outcome of a conditional GET.
#[derive(Debug, Clone)]
pub enum Conditional {
    /// 304. The stored copy is current and the server sent no body — the whole point.
    NotModified { url: String },
    /// 200 with a body.
    Fetched(Box<PoliteResponse>),
}

/// Cache validators for one URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validators {
    pub fn is_empty(&self) -> bool {
        self.etag.is_none() && self.last_modified.is_none()
    }
}

// ------------------------------------------------------------------------------------------
// robots.txt
// ------------------------------------------------------------------------------------------

/// One `Allow:` or `Disallow:` line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RobotsRule {
    path: String,
    allow: bool,
}

/// The parsed `robots.txt` rules that apply to [`ROBOTS_USER_AGENT`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Robots {
    rules: Vec<RobotsRule>,
    /// `Crawl-delay`, when the host published one. Only ever *raises* our delay.
    crawl_delay: Option<Duration>,
}

impl Robots {
    /// A `robots.txt` that permits everything. What an "unavailable" (4xx) fetch means under
    /// RFC 9309 § 2.3.1.3, and what a host with no `robots.txt` at all means.
    pub fn allow_all() -> Self {
        Robots::default()
    }

    pub fn crawl_delay(&self) -> Option<Duration> {
        self.crawl_delay
    }

    /// Parse the wildcard user-agent group.
    ///
    /// Groups are keyed by consecutive `User-agent:` lines, so
    /// `User-agent: a` / `User-agent: *` / `Disallow: /x` puts `/x` in **both** groups. That is
    /// the specified behaviour and it is what `app.joinhandshake.com` relies on.
    pub fn parse(text: &str) -> Self {
        let mut rules = Vec::new();
        let mut crawl_delay = None;

        // Whether the group currently being read applies to us, and whether the previous
        // non-blank line was a `User-agent:` (which is what continues a group header).
        let mut in_our_group = false;
        let mut reading_group_header = false;

        for raw_line in text.lines() {
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((field, value)) = line.split_once(':') else {
                continue;
            };
            let field = field.trim().to_ascii_lowercase();
            let value = value.trim();

            match field.as_str() {
                "user-agent" => {
                    if !reading_group_header {
                        // A new group starts. Whether we are in it is decided from scratch.
                        in_our_group = false;
                        reading_group_header = true;
                    }
                    if value == ROBOTS_USER_AGENT {
                        in_our_group = true;
                    }
                }
                "disallow" | "allow" => {
                    reading_group_header = false;
                    if !in_our_group {
                        continue;
                    }
                    // `Disallow:` with an empty value means "nothing is disallowed" and is not
                    // a rule. `Allow:` with an empty value is meaningless; drop it too.
                    if value.is_empty() {
                        continue;
                    }
                    rules.push(RobotsRule {
                        path: value.to_string(),
                        allow: field == "allow",
                    });
                }
                "crawl-delay" => {
                    reading_group_header = false;
                    if in_our_group && let Ok(seconds) = value.parse::<f64>()
                        && seconds.is_finite()
                        && (0.0..=300.0).contains(&seconds)
                    {
                        crawl_delay = Some(Duration::from_secs_f64(seconds));
                    }
                }
                _ => {
                    reading_group_header = false;
                }
            }
        }

        Robots { rules, crawl_delay }
    }

    /// Whether `path` may be fetched, and which rule decided it.
    ///
    /// **Longest match wins, and `Allow` wins a tie.** Both halves matter:
    /// `app.joinhandshake.com` is `Disallow: /` with `Allow: /public`, so `/public/jobs/123` is
    /// permitted only if the longer `Allow` beats the shorter `Disallow`. Reading the file
    /// first-match-wins instead would refuse a source the research established is permitted.
    pub fn permits(&self, path: &str) -> Result<(), String> {
        let mut decision: Option<&RobotsRule> = None;

        for rule in &self.rules {
            if !path_matches(&rule.path, path) {
                continue;
            }
            let better = match decision {
                None => true,
                Some(current) => match rule.path.len().cmp(&current.path.len()) {
                    std::cmp::Ordering::Greater => true,
                    std::cmp::Ordering::Equal => rule.allow && !current.allow,
                    std::cmp::Ordering::Less => false,
                },
            };
            if better {
                decision = Some(rule);
            }
        }

        match decision {
            Some(rule) if !rule.allow => Err(format!("Disallow: {}", rule.path)),
            _ => Ok(()),
        }
    }
}

/// Whether a robots path pattern matches a request path.
///
/// Supports the two wildcards every major implementation does: `*` for any run of characters
/// and `$` anchoring the end. Everything else is a literal prefix match.
fn path_matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(stripped) => (stripped, true),
        None => (pattern, false),
    };

    let segments: Vec<&str> = pattern.split('*').collect();
    let mut cursor = 0usize;

    for (i, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            continue;
        }
        if i == 0 {
            // The first segment is anchored at the start of the path.
            if !path[cursor..].starts_with(segment) {
                return false;
            }
            cursor += segment.len();
            continue;
        }
        match path[cursor..].find(segment) {
            Some(offset) => cursor += offset + segment.len(),
            None => return false,
        }
    }

    if anchored {
        // With a trailing `$` the final segment has to land exactly at the end.
        return match segments.last() {
            Some(last) if !last.is_empty() => path.ends_with(last) && cursor == path.len(),
            _ => cursor == path.len(),
        };
    }

    true
}

// ------------------------------------------------------------------------------------------
// Per-host rate limiting
// ------------------------------------------------------------------------------------------

/// One slot per host, handed out in arrival order.
///
/// The lock is held only long enough to claim a slot and is **never** held across the sleep.
/// Holding it while waiting would make one slow host block every other host's requests, which
/// is the exact behaviour a per-host limiter exists to avoid.
#[derive(Debug, Default)]
struct RateLimiter {
    next_slot: Mutex<HashMap<String, Instant>>,
}

impl RateLimiter {
    /// Claim the next slot for `host` and wait for it.
    async fn acquire(&self, host: &str, delay: Duration) {
        let slot = {
            let mut slots = self.next_slot.lock().await;
            let now = Instant::now();
            let slot = slots.get(host).copied().unwrap_or(now).max(now);
            slots.insert(host.to_string(), slot + delay);
            slot
        };
        sleep_until(slot).await;
    }
}

// ------------------------------------------------------------------------------------------
// The client
// ------------------------------------------------------------------------------------------

/// The one HTTP client the internship collector has.
///
/// Cheap to clone (everything inside is behind an [`Arc`]), so the whole registry shares one
/// robots cache and one set of per-host rate-limit slots.
#[derive(Clone)]
pub struct PoliteClient {
    inner: Arc<Inner>,
}

struct Inner {
    /// Private, with no accessor. See the module doc.
    client: Client,
    robots: Mutex<HashMap<String, Result<Robots, String>>>,
    limiter: RateLimiter,
    validators: Mutex<HashMap<String, Validators>>,
    host_delay: Duration,
}

impl std::fmt::Debug for PoliteClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoliteClient")
            .field("host_delay", &self.inner.host_delay)
            .finish_non_exhaustive()
    }
}

impl PoliteClient {
    /// Build the client. Fails only if TLS cannot be initialized.
    pub fn new() -> Result<Self, FetchError> {
        Self::with_host_delay(DEFAULT_HOST_DELAY)
    }

    /// [`PoliteClient::new`] with an explicit floor on the per-host delay. Tests use zero;
    /// nothing in production should.
    pub fn with_host_delay(host_delay: Duration) -> Result<Self, FetchError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
            .build()
            .map_err(|error| FetchError::Transport {
                url: "<client construction>".to_string(),
                detail: error.to_string(),
            })?;

        Ok(PoliteClient {
            inner: Arc::new(Inner {
                client,
                robots: Mutex::new(HashMap::new()),
                limiter: RateLimiter::default(),
                validators: Mutex::new(HashMap::new()),
                host_delay,
            }),
        })
    }

    /// GET `url` as text, after checking `robots.txt` and waiting for this host's slot.
    pub async fn get(&self, url: &str) -> Result<PoliteResponse, FetchError> {
        match self.get_conditional_with(url, &Validators::default()).await? {
            Conditional::Fetched(response) => Ok(*response),
            // Unreachable in practice: we sent no validators, so the server has nothing to
            // compare against. Reported rather than `unreachable!()` — a panic in a fetch path
            // would take down the source that called it.
            Conditional::NotModified { url } => Err(FetchError::Decode {
                url,
                detail: "server answered 304 to an unconditional request".to_string(),
            }),
        }
    }

    /// GET `url` as JSON.
    pub async fn get_json(&self, url: &str) -> Result<serde_json::Value, FetchError> {
        self.get(url).await?.json()
    }

    /// GET `url`, replaying whatever `ETag`/`Last-Modified` this client last saw for it.
    ///
    /// The validators are remembered automatically, so an adapter that calls this on every run
    /// gets 304s for free once the process is warm. Verified upstream behaviour
    /// (`docs/INTERNSHIP_SCRAPING.md` § A.2): `raw.githubusercontent.com` answers
    /// `If-None-Match` with 304 and 0 bytes.
    pub async fn get_conditional(&self, url: &str) -> Result<Conditional, FetchError> {
        let stored = {
            let validators = self.inner.validators.lock().await;
            validators.get(url).cloned().unwrap_or_default()
        };
        self.get_conditional_with(url, &stored).await
    }

    /// [`PoliteClient::get_conditional`] with caller-supplied validators, for a caller that
    /// persists them itself.
    pub async fn get_conditional_with(
        &self,
        url: &str,
        validators: &Validators,
    ) -> Result<Conditional, FetchError> {
        let parsed = reqwest::Url::parse(url).map_err(|error| FetchError::Transport {
            url: url.to_string(),
            detail: format!("unparseable url: {error}"),
        })?;
        let host = parsed.host_str().unwrap_or_default().to_string();

        // 1. robots.txt, before anything is sent to the path itself.
        let robots = self.robots_for(&parsed).await?;
        let mut path = parsed.path().to_string();
        if let Some(query) = parsed.query() {
            path.push('?');
            path.push_str(query);
        }
        if let Err(rule) = robots.permits(&path) {
            return Err(FetchError::RobotsDisallowed {
                url: url.to_string(),
                rule,
            });
        }

        // 2. This host's slot. A published `Crawl-delay` raises our delay, never lowers it.
        let delay = robots
            .crawl_delay()
            .map(|published| published.max(self.inner.host_delay))
            .unwrap_or(self.inner.host_delay);
        self.inner.limiter.acquire(&host, delay).await;

        // 3. One attempt. No retry, no backoff — see the module doc.
        let mut request = self.inner.client.get(parsed.clone());
        if let Some(etag) = &validators.etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = &validators.last_modified {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, last_modified);
        }

        let response = request.send().await.map_err(|error| FetchError::Transport {
            url: url.to_string(),
            detail: error.to_string(),
        })?;

        let status = response.status();
        let final_url = response.url().to_string();

        if status == StatusCode::NOT_MODIFIED {
            return Ok(Conditional::NotModified {
                url: url.to_string(),
            });
        }

        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = header_string(&response, reqwest::header::RETRY_AFTER);
            return Err(FetchError::Blocked {
                url: url.to_string(),
                status: status.as_u16(),
                retry_after,
            });
        }

        if !status.is_success() {
            return Err(FetchError::Status {
                url: url.to_string(),
                status: status.as_u16(),
            });
        }

        let etag = header_string(&response, reqwest::header::ETAG);
        let last_modified = header_string(&response, reqwest::header::LAST_MODIFIED);

        let body = response.text().await.map_err(|error| FetchError::Decode {
            url: url.to_string(),
            detail: error.to_string(),
        })?;

        if etag.is_some() || last_modified.is_some() {
            let mut stored = self.inner.validators.lock().await;
            stored.insert(
                url.to_string(),
                Validators {
                    etag: etag.clone(),
                    last_modified: last_modified.clone(),
                },
            );
        }

        Ok(Conditional::Fetched(Box::new(PoliteResponse {
            status: status.as_u16(),
            requested_url: url.to_string(),
            final_url,
            body,
            etag,
            last_modified,
        })))
    }

    /// The cached `robots.txt` for this URL's host, fetching it once if we have not yet.
    ///
    /// The result is cached **including the failure**, so a host whose `robots.txt` is
    /// unreachable is not re-asked once per board for 485 boards.
    async fn robots_for(&self, url: &reqwest::Url) -> Result<Robots, FetchError> {
        let host = url.host_str().unwrap_or_default().to_string();
        let origin = format!(
            "{}://{}{}",
            url.scheme(),
            host,
            url.port().map(|p| format!(":{p}")).unwrap_or_default()
        );

        if let Some(cached) = self.inner.robots.lock().await.get(&origin) {
            return cached.clone().map_err(|detail| FetchError::RobotsUnreachable {
                host: host.clone(),
                detail,
            });
        }

        let robots_url = format!("{origin}/robots.txt");
        self.inner.limiter.acquire(&host, self.inner.host_delay).await;

        let outcome = match self.inner.client.get(&robots_url).send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    match response.text().await {
                        Ok(text) if text.len() <= MAX_ROBOTS_BYTES => Ok(Robots::parse(&text)),
                        // An oversized or unreadable body is not evidence of permission.
                        Ok(text) => Err(format!(
                            "robots.txt is {} bytes, over the {MAX_ROBOTS_BYTES}-byte ceiling",
                            text.len()
                        )),
                        Err(error) => Err(error.to_string()),
                    }
                } else if status.is_client_error() {
                    // RFC 9309 § 2.3.1.3: an "unavailable" (4xx) robots.txt permits access.
                    // This is the branch `api.ashbyhq.com` takes — it answers 401.
                    Ok(Robots::allow_all())
                } else {
                    // § 2.3.1.4: "unreachable" (5xx) means complete disallow.
                    Err(format!("HTTP {}", status.as_u16()))
                }
            }
            Err(error) => Err(error.to_string()),
        };

        self.inner
            .robots
            .lock()
            .await
            .insert(origin, outcome.clone());

        outcome.map_err(|detail| FetchError::RobotsUnreachable { host, detail })
    }

    /// Seed the robots cache without a network round trip. Tests only — production code must
    /// let the client fetch the real file.
    #[cfg(test)]
    pub async fn seed_robots(&self, origin: &str, robots: Robots) {
        self.inner
            .robots
            .lock()
            .await
            .insert(origin.to_string(), Ok(robots));
    }
}

fn header_string(response: &reqwest::Response, name: reqwest::header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
}

// ------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // The `robots.txt` bodies below were fetched on 2026-08-20 and are reproduced verbatim
    // (LinkedIn's from `docs/INTERNSHIP_SCRAPING.md` § A.4, which is why it is the short form
    // — we do not fetch LinkedIn).

    const GREENHOUSE_ROBOTS: &str = "\
# See http://www.robotstxt.org/robotstxt.html for documentation on how to use the robots.txt file

User-agent: *
Disallow: /embed/
";

    const LEVER_ROBOTS: &str = "\
User-agent: *
Allow: /
Crawl-delay: 1
";

    const WWR_ROBOTS: &str = "\
# robots.txt for https://weworkremotely.com/

User-agent: *
Allow: /
Disallow: /admin/
Disallow: /account/
Sitemap: https://weworkremotely.com/sitemap.xml
";

    const LINKEDIN_ROBOTS: &str = "\
User-agent: *
Disallow: /
";

    const HANDSHAKE_ROBOTS: &str = "\
User-agent: *
Disallow: /
Allow: /public
";

    // ---- robots parsing ----

    #[test]
    fn greenhouses_only_rule_blocks_embed_and_nothing_else() {
        let robots = Robots::parse(GREENHOUSE_ROBOTS);
        assert!(robots.permits("/v1/boards/airtable/jobs").is_ok());
        assert!(robots.permits("/embed/job_board").is_err());
    }

    #[test]
    fn a_published_crawl_delay_is_read() {
        // `docs/INTERNSHIP_SCRAPING.md` says no read rate limit is published for Lever. Its
        // robots.txt publishes `Crawl-delay: 1`, which is a rate limit, so we honour it.
        let robots = Robots::parse(LEVER_ROBOTS);
        assert_eq!(robots.crawl_delay(), Some(Duration::from_secs(1)));
        assert!(robots.permits("/v0/postings/acceldata").is_ok());
    }

    #[test]
    fn a_more_specific_disallow_beats_a_blanket_allow() {
        let robots = Robots::parse(WWR_ROBOTS);
        assert!(robots.permits("/categories/remote-programming-jobs.rss").is_ok());
        assert!(robots.permits("/admin/panel").is_err());
    }

    #[test]
    fn disallow_slash_blocks_every_path() {
        // LinkedIn. There is no polite configuration of this source, and the parser must say
        // so for every path rather than only for the ones we happen to test.
        let robots = Robots::parse(LINKEDIN_ROBOTS);
        for path in ["/", "/jobs", "/jobs/search?keywords=intern", "/anything/at/all"] {
            assert!(
                robots.permits(path).is_err(),
                "{path} must be disallowed under `Disallow: /`"
            );
        }
    }

    #[test]
    fn a_longer_allow_beats_a_blanket_disallow() {
        // Handshake: `Disallow: /` with `Allow: /public`. Read first-match-wins instead of
        // longest-match, this refuses a source the research established is permitted.
        let robots = Robots::parse(HANDSHAKE_ROBOTS);
        assert!(robots.permits("/public/jobs/12345").is_ok());
        assert!(robots.permits("/jobs/search").is_err());
    }

    #[test]
    fn rules_from_another_bots_group_do_not_apply_to_us() {
        // Named-bot allowances are exactly what we must not inherit: claiming Googlebot's
        // group would be the evasion the project rules forbid.
        let robots = Robots::parse(
            "User-agent: Googlebot\nAllow: /\n\nUser-agent: *\nDisallow: /\n",
        );
        assert!(robots.permits("/anything").is_err());
    }

    #[test]
    fn consecutive_user_agent_lines_share_one_group() {
        let robots = Robots::parse("User-agent: Bingbot\nUser-agent: *\nDisallow: /private\n");
        assert!(robots.permits("/private/x").is_err());
        assert!(robots.permits("/public/x").is_ok());
    }

    #[test]
    fn an_empty_disallow_value_forbids_nothing() {
        // `Disallow:` with no path is the documented way to say "allow everything", and
        // reading it as `Disallow: ""` would block the entire host.
        let robots = Robots::parse("User-agent: *\nDisallow:\n");
        assert!(robots.permits("/anything").is_ok());
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let robots = Robots::parse("# hello\n\nUser-agent: *   # us\nDisallow: /x  # nope\n");
        assert!(robots.permits("/x/y").is_err());
        assert!(robots.permits("/y").is_ok());
    }

    #[test]
    fn a_missing_robots_file_permits_everything() {
        let robots = Robots::allow_all();
        assert!(robots.permits("/").is_ok());
        assert!(robots.permits("/v1/boards/whatever/jobs").is_ok());
    }

    #[test]
    fn wildcards_and_end_anchors_are_honoured() {
        // Indeed's file uses `Disallow: /*?rss`, so the pattern forms are not hypothetical.
        let robots = Robots::parse("User-agent: *\nDisallow: /*?rss\nDisallow: /tmp$\n");
        assert!(robots.permits("/jobs?rss=1").is_err());
        assert!(robots.permits("/jobs?q=intern").is_ok());
        assert!(robots.permits("/tmp").is_err());
        assert!(robots.permits("/tmpfile").is_ok());
    }

    // ---- rate limiting ----

    #[tokio::test]
    async fn two_requests_to_one_host_are_spaced_out() {
        let limiter = RateLimiter::default();
        let delay = Duration::from_millis(40);
        let started = std::time::Instant::now();
        limiter.acquire("example.com", delay).await;
        limiter.acquire("example.com", delay).await;
        assert!(
            started.elapsed() >= delay,
            "the second request to a host must wait for its slot"
        );
    }

    #[tokio::test]
    async fn two_hosts_do_not_wait_on_each_other() {
        // The whole reason the limiter is per-host: 485 Greenhouse boards are one host, and a
        // global limiter would make the entire run serial for no politeness benefit.
        let limiter = RateLimiter::default();
        let delay = Duration::from_millis(200);
        limiter.acquire("a.example", delay).await;
        let started = std::time::Instant::now();
        limiter.acquire("b.example", delay).await;
        assert!(
            started.elapsed() < delay,
            "a different host must not queue behind the first"
        );
    }

    // ---- errors ----

    #[test]
    fn only_a_robots_refusal_reads_as_a_skip() {
        // Everything else is a failure. Painting a robots-respecting skip red in the health
        // panel trains its reader to ignore red.
        assert!(
            FetchError::RobotsDisallowed {
                url: "https://x/y".to_string(),
                rule: "Disallow: /".to_string()
            }
            .is_refusal()
        );
        assert!(
            !FetchError::Blocked {
                url: "https://x/y".to_string(),
                status: 403,
                retry_after: None
            }
            .is_refusal()
        );
        assert!(
            !FetchError::Status {
                url: "https://x/y".to_string(),
                status: 500
            }
            .is_refusal()
        );
    }

    #[test]
    fn a_404_is_distinguishable_from_every_other_failure() {
        // Lever: 404 means retire the slug; 200-with-an-empty-array means keep polling. The
        // collector must not conflate them.
        assert!(
            FetchError::Status {
                url: "https://api.lever.co/v0/postings/gone".to_string(),
                status: 404
            }
            .is_not_found()
        );
        assert!(
            !FetchError::Status {
                url: "https://api.lever.co/v0/postings/slow".to_string(),
                status: 503
            }
            .is_not_found()
        );
    }

    #[test]
    fn a_response_that_moved_is_visible_as_having_moved() {
        // The § D.2 trap: a dead Greenhouse job answers 200 from the board root.
        let response = PoliteResponse {
            status: 200,
            requested_url: "https://job-boards.greenhouse.io/mcghealth/jobs/8350486002".to_string(),
            final_url: "https://job-boards.greenhouse.io/mcghealth?error=true".to_string(),
            body: String::new(),
            etag: None,
            last_modified: None,
        };
        assert_eq!(response.status, 200, "status alone says the posting is alive");
        assert!(
            response.redirected_away_from(&response.requested_url.clone()),
            "and the redirect is what says it is not"
        );
    }

    #[test]
    fn the_user_agent_identifies_the_project_honestly() {
        assert!(USER_AGENT.contains("personal-website"));
        assert!(USER_AGENT.contains("github.com/terrificjesse"));
        // Nothing here may imitate a browser or a named crawler.
        let lower = USER_AGENT.to_lowercase();
        for impersonation in ["mozilla", "chrome", "safari", "googlebot", "bingbot"] {
            assert!(
                !lower.contains(impersonation),
                "the user agent must not impersonate {impersonation}"
            );
        }
    }
}
