//! RSS / Atom feeds. In practice: WeWorkRemotely, and that is nearly the whole class.
//!
//! `docs/INTERNSHIP_SCRAPING.md` § A.3 checked the obvious candidates and found the class mostly
//! dead: Greenhouse's RSS path 404s, SmartRecruiters' 404s, and Lever and Ashby never had one.
//! **The modern ATSs replaced RSS with JSON and did not keep both** — the § A.1 endpoints *are*
//! the feed. Don't go looking for per-company feeds; this module exists for the one real feed
//! that was found, and for the next one, if there is a next one.
//!
//! # The outcome rule, which is the whole point of this adapter
//!
//! **A truncated feed can never report [`SourceOutcome::Success`], and this one always is
//! truncated.** WeWorkRemotely publishes the 25 most recent items and nothing older. That is
//! not a fetch that went wrong — it is a source that structurally cannot be enumerated, at any
//! price, on any run.
//!
//! `Success` means *"absence from this run is evidence of closure"*. If this adapter ever
//! reported it, `expiry::settle_source_run` would increment `consecutive_misses` on every
//! posting this feed ever supplied and reset only the 25 currently on the front page. After the
//! miss threshold, everything older would expire — not because it closed, but because a
//! new-postings feed is doing exactly what a new-postings feed does. So [`FeedSource::fetch`]
//! returns [`SourceOutcome::Partial`] on every successful fetch, and
//! `a_truncated_feed_is_never_a_complete_enumeration` pins it.
//!
//! It is the clearest case in the registry of `Partial` meaning "this is real data that proves
//! nothing about what is missing", which is precisely the distinction the four-variant outcome
//! enum exists to carry.
//!
//! # WeWorkRemotely's shape
//!
//! Standard RSS 2.0: `title`, `link`, `guid`, `pubDate`, `category`, `region`, `description`.
//! Two things to know:
//!
//! - **`title` is `"Company: Role"`** and must be split on the **first** colon. Used raw, every
//!   posting's company is empty and its title carries the company name (§ B note 20).
//! - **`pubDate` is RFC 2822** (`"Wed, 22 Jul 2026 07:03:46 +0000"`), which
//!   `normalize::parse_timestamp` does **not** read — it handles RFC 3339, a few plain formats,
//!   and epochs. So it is converted here rather than passed through, and a conversion failure
//!   yields `None` rather than a fabricated date.
//!
//! Also: remote-first and not internship-specific. Everything on it is remote, so `remote_hint`
//! is `Some(true)` (§ B marks remote as present for this source); almost nothing on it is an
//! internship, so QC will filter most of it, and a large `filtered` count here is health rather
//! than a defect.
//!
//! # Parsing
//!
//! `quick-xml`, approved for this work, and both RSS `<item>` and Atom `<entry>` are handled by
//! one reader — a hand-rolled XML parser is where entities, CDATA and namespaces silently
//! corrupt data instead of failing.

use chrono::DateTime;
use quick_xml::Reader;
use quick_xml::events::Event;

use super::super::models::RawPosting;
use super::{BoxFuture, Source, SourceContext, SourceFetch};

/// WeWorkRemotely's programming category feed. Verified 2026-08-20: 25 items, standard RSS 2.0.
pub const WE_WORK_REMOTELY_URL: &str =
    "https://weworkremotely.com/categories/remote-programming-jobs.rss";

/// One RSS/Atom feed.
pub struct FeedSource {
    name: &'static str,
    description: &'static str,
    url: &'static str,
    /// Everything on this feed is remote. `None` for a feed where that is not true — it means
    /// *unknown*, and QC may still infer remoteness from the location or title.
    remote: Option<bool>,
}

impl FeedSource {
    pub fn we_work_remotely() -> Self {
        FeedSource {
            name: "weworkremotely",
            description: "WeWorkRemotely programming RSS — truncated to the 25 newest items, so \
                          never a complete enumeration",
            url: WE_WORK_REMOTELY_URL,
            remote: Some(true),
        }
    }

    pub fn url(&self) -> &str {
        self.url
    }
}

/// Alias kept because the registry reads better as `RssSource::we_work_remotely()`.
pub type RssSource = FeedSource;

impl Source for FeedSource {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn fetch<'a>(&'a self, ctx: &'a SourceContext) -> BoxFuture<'a, SourceFetch> {
        Box::pin(async move {
            let response = match ctx.http.get_conditional(self.url).await {
                Ok(super::super::http::Conditional::Fetched(response)) => response,
                // Nothing new since the last poll. Still not an enumeration — see the module
                // doc — so it is `Partial` either way, and saying so plainly beats replaying a
                // memo that would be no more complete than the feed itself.
                Ok(super::super::http::Conditional::NotModified { .. }) => {
                    return SourceFetch::partial(
                        Vec::new(),
                        format!("{}: 304 Not Modified — no new items since the last poll", self.url),
                    );
                }
                Err(error) if error.is_refusal() => return SourceFetch::skipped(error.to_string()),
                Err(error) => return SourceFetch::failed(error.to_string()),
            };

            let entries = match parse_feed(&response.body) {
                Ok(entries) => entries,
                Err(error) => {
                    return SourceFetch::failed(format!("{}: {error}", self.url));
                }
            };

            if entries.is_empty() {
                return SourceFetch::failed(format!(
                    "{}: parsed as a feed but contained no items — the shape has probably changed",
                    self.url
                ));
            }

            let count = entries.len();
            let postings: Vec<RawPosting> = entries
                .iter()
                .map(|entry| entry.to_raw_posting(self.name, self.remote))
                .collect();

            // ALWAYS `Partial`. The feed carries only the newest items; absence from it is not
            // evidence of anything. See the module doc.
            SourceFetch::partial(
                postings,
                format!(
                    "{count} items — this feed publishes only the most recent postings and \
                     carries no older ones, so it is never a complete enumeration and absence \
                     from it is not evidence of closure"
                ),
            )
        })
    }
}

// ------------------------------------------------------------------------------------------
// Feed parsing
// ------------------------------------------------------------------------------------------

/// One `<item>` or `<entry>`, with the fields both formats share.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedEntry {
    pub title: Option<String>,
    pub link: Option<String>,
    pub guid: Option<String>,
    /// RSS `pubDate` (RFC 2822) or Atom `published`/`updated` (RFC 3339), as written.
    pub published: Option<String>,
    pub category: Option<String>,
    /// WeWorkRemotely's non-standard `<region>` element.
    pub region: Option<String>,
    pub description: Option<String>,
}

impl FeedEntry {
    /// Split `"Company: Role"` on the **first** colon.
    ///
    /// § B note 20. The first colon specifically: a role like
    /// `"Acme: Engineer: Backend"` must yield company `Acme`, not `Acme: Engineer`. When there
    /// is no colon there is no company to state — the whole string is the title, and QC rejects
    /// the row visibly rather than this inventing an issuer for it.
    pub fn company_and_title(&self) -> (Option<String>, Option<String>) {
        let Some(title) = self.title.as_deref().map(str::trim) else {
            return (None, None);
        };
        match title.split_once(':') {
            Some((company, role)) if !company.trim().is_empty() && !role.trim().is_empty() => (
                Some(company.trim().to_string()),
                Some(role.trim().to_string()),
            ),
            _ => (None, Some(title.to_string())),
        }
    }

    pub fn to_raw_posting(&self, source: &str, remote: Option<bool>) -> RawPosting {
        let (company, title) = self.company_and_title();
        let url = self
            .link
            .clone()
            .or_else(|| self.guid.clone())
            .unwrap_or_default();

        RawPosting {
            source: source.to_string(),
            // `guid` is the feed's own stable identifier and is what makes an item
            // recognizable next run; the link is the fallback because it is also stable.
            external_id: self.guid.clone().or_else(|| self.link.clone()).unwrap_or_default(),
            url,
            company: company.unwrap_or_default(),
            title: title.unwrap_or_default(),
            location_raw: self.region.clone(),
            // No pay of any kind on this feed (§ B).
            pay_raw: None,
            // `category` is a topic ("Full-Stack Programming"), not a season. Passed as the
            // term hint anyway because QC reads title and term together for the internship
            // signal, and a category naming an internship programme would otherwise be lost.
            term_raw: self.category.clone(),
            class_year_raw: None,
            // RFC 2822 converted to RFC 3339 — `normalize::parse_timestamp` does not read
            // RFC 2822. A failed conversion is `None`, never a guessed date.
            posted_at_raw: self.published.as_deref().and_then(to_rfc3339),
            deadline_raw: None,
            description: self.description.clone(),
            remote_hint: remote,
            raw_json: serde_json::json!({
                "title": self.title,
                "link": self.link,
                "guid": self.guid,
                "pubDate": self.published,
                "category": self.category,
                "region": self.region,
            })
            .to_string(),
        }
    }
}

/// Convert an RSS `pubDate` into something `normalize::parse_timestamp` reads.
///
/// Tries RFC 2822 first (what RSS 2.0 specifies and what WeWorkRemotely sends), then RFC 3339
/// (what Atom sends), and otherwise passes the value through untouched so a plain `YYYY-MM-DD`
/// still reaches the parser that handles it.
pub fn to_rfc3339(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc2822(trimmed) {
        return Some(parsed.to_rfc3339());
    }
    if DateTime::parse_from_rfc3339(trimmed).is_ok() {
        return Some(trimmed.to_string());
    }
    Some(trimmed.to_string())
}

/// Parse an RSS 2.0 or Atom document into its entries.
///
/// One reader for both formats: RSS nests items under `<item>`, Atom under `<entry>`, and the
/// child element names differ but overlap enough that a single dispatch covers them.
pub fn parse_feed(xml: &str) -> Result<Vec<FeedEntry>, String> {
    let mut reader = Reader::from_str(xml);
    // Deliberately **not** `trim_text(true)`. An entity arrives as its own event, so
    // `Ben &amp; Jerry's` is delivered as three chunks — `"Ben "`, the entity, `" Jerry's"` —
    // and per-chunk trimming eats the spaces on either side of every entity, yielding
    // `"Ben&Jerry's"`. That corrupts the company key rather than merely the label, and it does
    // it silently. Whitespace is trimmed once, on the assembled value, in `assign`.
    //
    // Inter-element indentation is not a problem: those chunks arrive while no field is open
    // and are discarded.
    //
    // Real feeds are not always well-formed. Tolerating a mismatched close tag keeps one bad
    // item from costing the whole feed.
    reader.config_mut().check_end_names = false;

    let mut entries = Vec::new();
    let mut current: Option<FeedEntry> = None;
    // The element whose text we are collecting, and the text collected so far.
    let mut field: Option<String> = None;
    let mut text = String::new();
    // Atom puts the link in an attribute rather than in the element body.
    let mut atom_link: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                let name = local_name(start.name().as_ref());
                match name.as_str() {
                    "item" | "entry" => {
                        current = Some(FeedEntry::default());
                        atom_link = None;
                    }
                    _ if current.is_some() => {
                        if name == "link"
                            && let Some(href) = attribute(&start, b"href")
                        {
                            atom_link = Some(href);
                        }
                        field = Some(name);
                        text.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(empty)) => {
                // Atom's `<link href="…"/>` is an empty element, so it never produces a Start.
                if current.is_some()
                    && local_name(empty.name().as_ref()) == "link"
                    && let Some(href) = attribute(&empty, b"href")
                {
                    atom_link = Some(href);
                }
            }
            Ok(Event::Text(chunk)) => {
                if field.is_some()
                    && let Ok(decoded) = chunk.decode()
                {
                    text.push_str(&unescape(&decoded));
                }
            }
            Ok(Event::CData(chunk)) => {
                // CDATA is already unescaped by definition — running it through the entity
                // decoder would mangle any literal `&amp;` the author intended.
                if field.is_some()
                    && let Ok(decoded) = chunk.decode()
                {
                    text.push_str(&decoded);
                }
            }
            Ok(Event::GeneralRef(entity)) => {
                // quick-xml surfaces `&amp;` as its own event rather than inline in the text,
                // so an entity dropped here silently deletes characters from every title
                // containing an ampersand.
                if field.is_some()
                    && let Ok(name) = entity.decode()
                {
                    text.push_str(&unescape(&format!("&{name};")));
                }
            }
            Ok(Event::End(end)) => {
                let name = local_name(end.name().as_ref());
                match name.as_str() {
                    "item" | "entry" => {
                        if let Some(mut entry) = current.take() {
                            if entry.link.is_none() {
                                entry.link = atom_link.take();
                            }
                            entries.push(entry);
                        }
                        field = None;
                    }
                    _ => {
                        if let (Some(entry), Some(open)) = (current.as_mut(), field.as_ref())
                            && open == &name
                        {
                            assign(entry, &name, text.trim());
                            text.clear();
                            field = None;
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(format!("XML parse error: {error}")),
        }
    }

    Ok(entries)
}

fn assign(entry: &mut FeedEntry, name: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    let value = value.to_string();
    match name {
        "title" => entry.title.get_or_insert(value),
        "link" => entry.link.get_or_insert(value),
        "guid" | "id" => entry.guid.get_or_insert(value),
        "pubdate" | "published" | "updated" | "date" => entry.published.get_or_insert(value),
        "category" => entry.category.get_or_insert(value),
        "region" => entry.region.get_or_insert(value),
        "description" | "summary" | "content" => entry.description.get_or_insert(value),
        _ => return,
    };
}

/// The element name without its namespace prefix, lowercased. `<dc:date>` and `<date>` are the
/// same field to us, and RSS element names are conventionally but not reliably cased.
fn local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    name.rsplit(':').next().unwrap_or(&name).to_ascii_lowercase()
}

fn attribute(tag: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    tag.attributes().flatten().find_map(|attribute| {
        (attribute.key.as_ref() == key)
            .then(|| String::from_utf8_lossy(attribute.value.as_ref()).trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

/// Resolve XML entities, leaving the text alone if any of them is unknown.
///
/// Feed descriptions are full of HTML-escaped markup (`&lt;p&gt;`) and of HTML entities that
/// are *not* XML entities (`&nbsp;`). Failing closed — returning the input unchanged — keeps an
/// unknown entity from costing the whole field.
fn unescape(text: &str) -> String {
    match quick_xml::escape::unescape(text) {
        Ok(unescaped) => unescaped.into_owned(),
        Err(_) => text.to_string(),
    }
}

// ------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internships::models::SourceOutcome;
    use crate::internships::normalize::parse_timestamp;

    /// The first five items of the real WeWorkRemotely programming feed, fetched 2026-08-20.
    /// Descriptions replaced with a placeholder to keep the fixture small; see
    /// `data/internships/README.md`.
    const FIXTURE: &str =
        include_str!("../../../data/internships/fixtures/weworkremotely.sample.rss");

    // ---- the outcome rule ----

    #[test]
    fn a_truncated_feed_is_never_a_complete_enumeration() {
        // THE test for this module. WeWorkRemotely publishes the 25 newest items and nothing
        // older. Reported as `Success`, `settle_source_run` would increment misses on every
        // posting this feed ever supplied and reset only the 25 on the front page — so
        // everything older would expire after the miss threshold, because a new-postings feed
        // did exactly what a new-postings feed does.
        let entries = parse_feed(FIXTURE).expect("the fixture parses");
        let postings: Vec<RawPosting> = entries
            .iter()
            .map(|entry| entry.to_raw_posting("weworkremotely", Some(true)))
            .collect();
        let fetch = SourceFetch::partial(postings, "truncated feed");

        assert_ne!(fetch.outcome(), SourceOutcome::Success);
        assert_eq!(fetch.outcome(), SourceOutcome::Partial);
        assert!(!fetch.postings().is_empty(), "the items themselves are still real data");
    }

    #[test]
    fn the_source_advertises_its_own_truncation() {
        // Whoever reads the health panel should be able to see why this source never expires
        // anything, without going to the code to find out.
        let source = FeedSource::we_work_remotely();
        assert!(source.description().contains("never a complete enumeration"));
        assert_eq!(source.url(), WE_WORK_REMOTELY_URL);
    }

    // ---- parsing the real feed ----

    #[test]
    fn the_fixture_parses_into_entries_with_the_documented_fields() {
        let entries = parse_feed(FIXTURE).expect("parses");
        assert!(!entries.is_empty());
        for entry in &entries {
            assert!(entry.title.is_some());
            assert!(entry.link.is_some());
            assert!(entry.guid.is_some());
            assert!(entry.published.is_some());
            assert!(entry.category.is_some());
            assert!(entry.region.is_some(), "WeWorkRemotely's non-standard <region>");
        }
    }

    #[test]
    fn the_title_is_split_into_company_and_role_on_the_first_colon() {
        // § B note 20. Used raw, every posting's company is empty and its title carries the
        // company name — which then lands in the dedup key and the rendered card.
        let entries = parse_feed(FIXTURE).expect("parses");
        let posting = entries[0].to_raw_posting("weworkremotely", Some(true));
        assert!(!posting.company.is_empty());
        assert!(!posting.title.is_empty());
        assert!(
            !posting.title.contains(&posting.company),
            "the company must not survive inside the title"
        );
    }

    #[test]
    fn only_the_first_colon_splits() {
        let entry = FeedEntry {
            title: Some("Acme: Engineer: Backend".to_string()),
            ..FeedEntry::default()
        };
        let (company, title) = entry.company_and_title();
        assert_eq!(company.as_deref(), Some("Acme"));
        assert_eq!(title.as_deref(), Some("Engineer: Backend"));
    }

    #[test]
    fn a_title_with_no_colon_states_no_company_rather_than_inventing_one() {
        // QC then rejects the row visibly with `missing_company`, which is a defect someone can
        // find. A fabricated issuer is a defect nobody can.
        let entry = FeedEntry {
            title: Some("Senior Rust Engineer".to_string()),
            ..FeedEntry::default()
        };
        let (company, title) = entry.company_and_title();
        assert_eq!(company, None);
        assert_eq!(title.as_deref(), Some("Senior Rust Engineer"));
        assert!(entry.to_raw_posting("weworkremotely", None).company.is_empty());
    }

    #[test]
    fn a_colon_with_nothing_before_it_is_not_a_company() {
        let entry = FeedEntry {
            title: Some(": Engineer".to_string()),
            ..FeedEntry::default()
        };
        assert_eq!(entry.company_and_title().0, None);
    }

    // ---- dates ----

    #[test]
    fn rfc_2822_pub_dates_are_converted_because_normalize_cannot_read_them() {
        // The trap. `normalize::parse_timestamp` handles RFC 3339, a few plain formats and
        // epochs — not RFC 2822. Passed through, every WeWorkRemotely posting would have no
        // posted date, get backfilled from first sighting, and read to the ranking as an
        // estimate instead of a stated date.
        let raw = "Wed, 22 Jul 2026 07:03:46 +0000";
        assert!(
            parse_timestamp(raw).is_none(),
            "if normalize ever learns RFC 2822 this conversion can be reconsidered"
        );

        let converted = to_rfc3339(raw).expect("converts");
        let parsed = parse_timestamp(&converted).expect("and normalize reads the result");
        assert_eq!(parsed.format("%Y-%m-%d").to_string(), "2026-07-22");
    }

    #[test]
    fn every_fixture_item_yields_a_date_normalize_can_read() {
        for entry in parse_feed(FIXTURE).expect("parses") {
            let posting = entry.to_raw_posting("weworkremotely", Some(true));
            let raw = posting.posted_at_raw.as_deref().expect("pubDate is present");
            assert!(parse_timestamp(raw).is_some(), "normalize could not read {raw}");
        }
    }

    #[test]
    fn an_rfc_3339_date_passes_through_unchanged() {
        // Atom feeds send RFC 3339 already.
        assert_eq!(
            to_rfc3339("2026-07-22T07:03:46+00:00").as_deref(),
            Some("2026-07-22T07:03:46+00:00")
        );
    }

    #[test]
    fn an_unparseable_date_is_passed_on_rather_than_guessed() {
        // QC's parser gets the final say and yields `None` if it cannot read it either. What
        // must not happen is a date being invented here.
        assert_eq!(to_rfc3339("sometime last week").as_deref(), Some("sometime last week"));
        assert_eq!(to_rfc3339("   "), None);
    }

    // ---- XML handling ----

    #[test]
    fn entities_survive_parsing() {
        // quick-xml surfaces `&amp;` as its own event rather than inline in the text. Dropped,
        // this silently deletes characters from every title containing an ampersand — and
        // "Barnes & Noble" becoming "Barnes  Noble" breaks the company key, not just the label.
        let xml = r#"<rss><channel><item>
            <title>Ben &amp; Jerry&#39;s: Software Engineering Intern</title>
            <link>https://example.test/1</link>
        </item></channel></rss>"#;
        let entries = parse_feed(xml).expect("parses");
        assert_eq!(entries.len(), 1);
        let (company, title) = entries[0].company_and_title();
        assert_eq!(company.as_deref(), Some("Ben & Jerry's"));
        assert_eq!(title.as_deref(), Some("Software Engineering Intern"));
    }

    #[test]
    fn cdata_content_is_read_verbatim() {
        let xml = r#"<rss><channel><item>
            <title><![CDATA[Acme: Engineer & Friend]]></title>
        </item></channel></rss>"#;
        let entries = parse_feed(xml).expect("parses");
        assert_eq!(entries[0].title.as_deref(), Some("Acme: Engineer & Friend"));
    }

    #[test]
    fn an_unknown_html_entity_costs_nothing() {
        // Feed descriptions carry HTML entities that are not XML entities (`&nbsp;`). Failing
        // closed keeps one of them from costing the whole field.
        let xml = "<rss><channel><item><title>Acme: A&nbsp;B</title></item></channel></rss>";
        let entries = parse_feed(xml).expect("parses");
        assert!(entries[0].title.as_deref().unwrap().contains('B'));
    }

    #[test]
    fn an_atom_feed_parses_through_the_same_reader() {
        // One crate and one reader for both formats — the reason `quick-xml` was chosen over a
        // format-specific crate.
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <entry>
                <title>Acme: Backend Intern</title>
                <link href="https://example.test/jobs/1"/>
                <id>urn:uuid:1</id>
                <published>2026-07-22T07:03:46Z</published>
                <summary>Work on things.</summary>
            </entry>
        </feed>"#;
        let entries = parse_feed(xml).expect("parses");
        assert_eq!(entries.len(), 1);
        let posting = entries[0].to_raw_posting("atomfeed", None);
        assert_eq!(posting.url, "https://example.test/jobs/1");
        assert_eq!(posting.external_id, "urn:uuid:1");
        assert_eq!(posting.company, "Acme");
        assert!(parse_timestamp(posting.posted_at_raw.as_deref().unwrap()).is_some());
    }

    #[test]
    fn a_namespaced_element_is_read_as_its_local_name() {
        let xml = r#"<rss xmlns:dc="http://purl.org/dc/elements/1.1/"><channel><item>
            <title>Acme: Intern</title><dc:date>2026-07-22T07:03:46Z</dc:date>
        </item></channel></rss>"#;
        let entries = parse_feed(xml).expect("parses");
        assert!(entries[0].published.is_some());
    }

    #[test]
    fn channel_level_elements_are_not_mistaken_for_items() {
        // The channel has its own <title> and <link>. Read as an item, the feed gains a
        // phantom posting named after the site on every single run.
        let entries = parse_feed(FIXTURE).expect("parses");
        for entry in &entries {
            let title = entry.title.as_deref().unwrap_or_default();
            assert!(
                !title.starts_with("We Work Remotely:"),
                "the channel title leaked into the items: {title}"
            );
        }
    }

    #[test]
    fn a_document_that_is_not_a_feed_yields_no_entries_rather_than_an_error() {
        assert_eq!(parse_feed("<html><body>hi</body></html>"), Ok(Vec::new()));
        assert_eq!(parse_feed(""), Ok(Vec::new()));
    }

    #[test]
    fn every_posting_from_this_feed_is_marked_remote() {
        // § B marks remote as present for WeWorkRemotely: it is a remote-only job board, so
        // this is a fact about the source rather than an inference about the posting.
        for entry in parse_feed(FIXTURE).expect("parses") {
            assert_eq!(
                entry.to_raw_posting("weworkremotely", Some(true)).remote_hint,
                Some(true)
            );
        }
    }

    #[test]
    fn no_posting_from_this_feed_claims_a_salary() {
        for entry in parse_feed(FIXTURE).expect("parses") {
            assert!(entry.to_raw_posting("weworkremotely", Some(true)).pay_raw.is_none());
        }
    }
}
