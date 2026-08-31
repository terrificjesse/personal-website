//! Writing Gmail labels. **The only module in this crate that modifies a mailbox.**
//!
//! Kept apart from [`super::gmail`] deliberately: that module is read-only and has a test which
//! fails the build if a write call appears in it, so "when did this gain write access" is
//! answerable from a diff. This is that arrival, and it is one file.
//!
//! # What it will and will not do
//!
//! The granted scope is `gmail.modify`, which withholds the two irreversible powers — permanent
//! delete and send-as. Within what it *does* allow, this module further limits itself:
//!
//! - It **adds** labels. It never removes one, including ones it added: a message that was a
//!   confirmation stays a confirmation, and stripping a label a human added by hand would be
//!   a silent loss of their work.
//! - It **never archives.** Removing `INBOX` is permitted by the scope and is not done here.
//!   A mislabelled email is a nuisance you can see; an archived one is gone from where you
//!   look for it.
//! - It **never touches a disregarded message.** Rule 7: disregarded means unlabelled, not
//!   unrecorded. The verdict is stored, the inbox is left alone. This is the highest-volume
//!   path, so it is also the one where a wrong label would do the most damage.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::collections::HashMap;

use super::classify::Category;

const API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// The label a category projects onto a message, or `None` for the ones that get no label.
///
/// These are a **projection of application status**, which is the structural idea of the whole
/// phase: the folders already exist as `internship_applications.status`. Built the other way
/// round — labels as the source of truth — you get two taxonomies that drift, and a tracker
/// still reading `applied` for a job you already interviewed at.
pub fn label_for(category: Category) -> Option<&'static str> {
    match category {
        Category::Confirmation => Some("Hunt/Confirmed"),
        Category::Oa => Some("Hunt/OA"),
        Category::Interview => Some("Hunt/Interview"),
        Category::Offer => Some("Hunt/Offer"),
        Category::Rejection => Some("Hunt/Rejected"),
        Category::Outreach => Some("Hunt/Outreach"),
        // Rule 7. The inbox stays untouched.
        Category::Disregarded => None,
    }
}

#[derive(Debug, Deserialize)]
struct LabelListResponse {
    #[serde(default)]
    labels: Vec<Label>,
}

#[derive(Debug, Deserialize, Clone)]
struct Label {
    id: String,
    name: String,
}

/// Existing label names to ids.
pub async fn existing(client: &reqwest::Client, token: &str) -> Result<HashMap<String, String>> {
    let response = client
        .get(format!("{API}/labels"))
        .bearer_auth(token)
        .send()
        .await
        .context("listing Gmail labels")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Gmail refused the label list ({status}): {body}"));
    }

    Ok(response
        .json::<LabelListResponse>()
        .await
        .context("parsing the label list")?
        .labels
        .into_iter()
        .map(|label| (label.name, label.id))
        .collect())
}

/// Create a label, returning its id. Creating one that exists is not an error.
///
/// Gmail nests on `/`, so creating `Hunt/OA` produces the `Hunt` parent in the UI on its own —
/// no need to create it separately, and trying to would be the 409 handled below.
pub async fn create(
    client: &reqwest::Client,
    token: &str,
    name: &str,
) -> Result<String> {
    let response = client
        .post(format!("{API}/labels"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": name,
            "labelListVisibility": "labelShow",
            "messageListVisibility": "show",
        }))
        .send()
        .await
        .with_context(|| format!("creating the label {name}"))?;

    if response.status().is_success() {
        return Ok(response
            .json::<Label>()
            .await
            .context("parsing the created label")?
            .id);
    }

    // 409 means somebody — a previous run, or you — already made it. Look it up rather than
    // failing: a label existing is the state we wanted.
    if response.status() == reqwest::StatusCode::CONFLICT {
        return existing(client, token)
            .await?
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("Gmail says {name} exists but does not list it"));
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(anyhow!("Gmail refused to create {name} ({status}): {body}"))
}

/// Every label this agent uses, creating any that are missing.
///
/// Resolved once per pass rather than per message: it is two API calls at most, against a
/// hundred messages.
pub async fn ensure_all(client: &reqwest::Client, token: &str) -> Result<HashMap<String, String>> {
    let mut known = existing(client, token).await?;

    for category in [
        Category::Confirmation,
        Category::Oa,
        Category::Interview,
        Category::Offer,
        Category::Rejection,
        Category::Outreach,
    ] {
        let Some(name) = label_for(category) else {
            continue;
        };
        if known.contains_key(name) {
            continue;
        }
        // Created on demand — the "sorted into a newly labelled folder if need be" case. A
        // first run against a fresh mailbox creates all six.
        let id = create(client, token, name).await?;
        println!("inbox: created Gmail label {name}");
        known.insert(name.to_string(), id);
    }

    Ok(known)
}

/// Add one label to one message.
///
/// `addLabelIds` only. `removeLabelIds` is deliberately absent — see the module doc.
pub async fn apply(
    client: &reqwest::Client,
    token: &str,
    message_id: &str,
    label_id: &str,
) -> Result<()> {
    let response = client
        .post(format!("{API}/messages/{message_id}/modify"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "addLabelIds": [label_id] }))
        .send()
        .await
        .context("labelling a message")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Gmail refused to label {message_id} ({status}): {body}"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;


    /// The module's code with comment lines removed.
///
/// Scanning raw source made a doc comment explaining what this module does *not* do fail the
/// check that it does not do it. Prose about a forbidden call is not the call.
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

    #[test]
    fn a_disregarded_message_gets_no_label() {
        // Rule 7, and the highest-volume path: the verdict is recorded, the inbox untouched.
        assert_eq!(label_for(Category::Disregarded), None);
    }

    #[test]
    fn every_other_category_projects_onto_one_label() {
        for category in [
            Category::Confirmation,
            Category::Oa,
            Category::Interview,
            Category::Offer,
            Category::Rejection,
            Category::Outreach,
        ] {
            let label = label_for(category).unwrap_or_else(|| panic!("{category:?}"));
            assert!(label.starts_with("Hunt/"), "{label} should live under Hunt/");
        }
    }

    #[test]
    fn labels_are_namespaced_so_nothing_of_yours_collides() {
        // Everything nests under one parent, so the agent's labels are removable as a group
        // and cannot collide with a label you already had.
        let mut seen = std::collections::HashSet::new();
        for category in [
            Category::Confirmation, Category::Oa, Category::Interview,
            Category::Offer, Category::Rejection, Category::Outreach,
        ] {
            let label = label_for(category).unwrap();
            assert!(seen.insert(label), "{label} is used by two categories");
        }
        assert_eq!(seen.len(), 6);
    }

    /// The counterpart to `gmail.rs`'s read-only assertion: writes live HERE and nowhere else.
    #[test]
    fn this_module_never_removes_a_label_or_archives() {
        let source = include_str!("labels.rs");
        let body = code_only(source.split("mod tests").next().expect("the module above its tests"));

        for forbidden in ["removeLabelIds", "\"INBOX\"", "/trash", "/delete", "batchDelete"] {
            assert!(
                !body.contains(forbidden),
                "labels.rs contains {forbidden:?} — this module adds labels and does nothing \
                 else; removing or archiving needs its own deliberate change"
            );
        }
    }
}
