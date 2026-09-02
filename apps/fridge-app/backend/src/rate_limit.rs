//! In-process write throttles for the two cheap denial-of-service paths (Phase 10j).
//!
//! # Contract
//!
//! `POST /auth/login` permits ten attempts per fifteen-minute sliding window. Two independent
//! buckets must both have room: the direct socket peer IP (stops one client spraying account
//! names) and the normalized email address (stops many clients concentrating on one account).
//! Every attempt counts, including a successful one, and the check runs before the database
//! lookup and Argon2 verification.
//!
//! `POST /reviews` permits thirty submissions per hour under the same two-bucket rule, using
//! the authenticated user id as the account key. Login and review buckets are independent.
//!
//! The state is intentionally in memory: this deployment is one process, a restart may forgive
//! old attempts, and the user explicitly accepted an in-memory limiter for this hardening pass.
//! It is not a distributed rate limiter. IP means the TCP peer supplied by Axum; proxy headers
//! are not trusted, because accepting a caller-controlled `X-Forwarded-For` would make the IP
//! limit optional. A reverse proxy therefore shares one IP bucket unless a later deployment
//! change introduces an explicitly trusted proxy boundary.

use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    hash::Hash,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{ConnectInfo, FromRequestParts},
    http::{HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use serde::Serialize;

const LOGIN_MAX_ATTEMPTS: usize = 10;
const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
const REVIEW_MAX_SUBMISSIONS: usize = 30;
const REVIEW_WINDOW: Duration = Duration::from_secs(60 * 60);
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// The direct TCP peer. Missing connection metadata collapses into one shared bucket rather
/// than failing open; `main` supplies it with `into_make_service_with_connect_info`.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(address)| address.ip())
            .unwrap_or(IpAddr::from([0, 0, 0, 0]));
        Ok(Self(ip))
    }
}

#[derive(Debug, Clone, Default)]
pub struct RateLimits {
    inner: Arc<Mutex<AllLimits>>,
}

impl RateLimits {
    pub fn check_login(&self, ip: IpAddr, account: &str) -> Result<(), RetryAfter> {
        self.check(Endpoint::Login, ip, account, Instant::now())
    }

    pub fn check_review(&self, ip: IpAddr, account: &str) -> Result<(), RetryAfter> {
        self.check(Endpoint::Review, ip, account, Instant::now())
    }

    fn check(
        &self,
        endpoint: Endpoint,
        ip: IpAddr,
        account: &str,
        now: Instant,
    ) -> Result<(), RetryAfter> {
        let mut limits = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        match endpoint {
            Endpoint::Login => {
                limits
                    .login
                    .check(ip, account, now, LOGIN_MAX_ATTEMPTS, LOGIN_WINDOW)
            }
            Endpoint::Review => {
                limits
                    .reviews
                    .check(ip, account, now, REVIEW_MAX_SUBMISSIONS, REVIEW_WINDOW)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Endpoint {
    Login,
    Review,
}

#[derive(Debug, Default)]
struct AllLimits {
    login: EndpointLimit,
    reviews: EndpointLimit,
}

#[derive(Debug, Default)]
struct EndpointLimit {
    by_ip: HashMap<IpAddr, VecDeque<Instant>>,
    by_account: HashMap<String, VecDeque<Instant>>,
    last_sweep: Option<Instant>,
}

impl EndpointLimit {
    fn check(
        &mut self,
        ip: IpAddr,
        account: &str,
        now: Instant,
        maximum: usize,
        window: Duration,
    ) -> Result<(), RetryAfter> {
        self.sweep_if_due(now, window);

        let ip_wait = retry_after(self.by_ip.get_mut(&ip), now, maximum, window);
        let account_key = account.to_owned();
        let account_wait = retry_after(self.by_account.get_mut(&account_key), now, maximum, window);

        if let Some(wait) = ip_wait.into_iter().chain(account_wait).max() {
            return Err(RetryAfter(wait));
        }

        self.by_ip.entry(ip).or_default().push_back(now);
        self.by_account
            .entry(account_key)
            .or_default()
            .push_back(now);
        Ok(())
    }

    fn sweep_if_due(&mut self, now: Instant, window: Duration) {
        if self
            .last_sweep
            .is_some_and(|last| now.saturating_duration_since(last) < SWEEP_INTERVAL)
        {
            return;
        }

        prune_map(&mut self.by_ip, now, window);
        prune_map(&mut self.by_account, now, window);
        self.last_sweep = Some(now);
    }
}

fn retry_after(
    attempts: Option<&mut VecDeque<Instant>>,
    now: Instant,
    maximum: usize,
    window: Duration,
) -> Option<Duration> {
    let attempts = attempts?;
    prune(attempts, now, window);
    if attempts.len() < maximum {
        return None;
    }

    attempts
        .front()
        .map(|oldest| window.saturating_sub(now.saturating_duration_since(*oldest)))
}

fn prune_map<K: Eq + Hash>(
    attempts_by_key: &mut HashMap<K, VecDeque<Instant>>,
    now: Instant,
    window: Duration,
) {
    attempts_by_key.retain(|_, attempts| {
        prune(attempts, now, window);
        !attempts.is_empty()
    });
}

fn prune(attempts: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while attempts
        .front()
        .is_some_and(|attempt| now.saturating_duration_since(*attempt) >= window)
    {
        attempts.pop_front();
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetryAfter(Duration);

impl RetryAfter {
    fn seconds(self) -> u64 {
        let seconds = self.0.as_secs();
        seconds + u64::from(self.0.subsec_nanos() > 0)
    }

    pub fn response(self, endpoint: &'static str, ip: IpAddr, account: &str) -> Response {
        let seconds = self.seconds().max(1);
        eprintln!(
            "rate limit: blocked {endpoint} ip={ip} account={account:?} retry_after={seconds}s"
        );

        let mut response = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(RateLimitBody {
                error: "Too many requests. Try again later.",
            }),
        )
            .into_response();
        response.headers_mut().insert(
            header::RETRY_AFTER,
            HeaderValue::from_str(&seconds.to_string()).expect("integer is a valid header value"),
        );
        response
    }
}

#[derive(Debug, Serialize)]
struct RateLimitBody {
    error: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(last_octet: u8) -> IpAddr {
        IpAddr::from([192, 0, 2, last_octet])
    }

    #[test]
    fn login_refuses_the_eleventh_attempt() {
        let limits = RateLimits::default();
        let now = Instant::now();

        for _ in 0..LOGIN_MAX_ATTEMPTS {
            assert!(
                limits
                    .check(Endpoint::Login, ip(1), "person@example.com", now)
                    .is_ok()
            );
        }

        let retry = limits
            .check(Endpoint::Login, ip(1), "person@example.com", now)
            .expect_err("the eleventh attempt must be refused");
        assert_eq!(retry.seconds(), LOGIN_WINDOW.as_secs());
    }

    #[test]
    fn cloned_handles_share_one_budget() {
        let limits = RateLimits::default();
        let another_request_handle = limits.clone();
        let now = Instant::now();

        for _ in 0..LOGIN_MAX_ATTEMPTS {
            limits
                .check(Endpoint::Login, ip(1), "person@example.com", now)
                .expect("within login limit");
        }

        assert!(
            another_request_handle
                .check(Endpoint::Login, ip(1), "person@example.com", now)
                .is_err()
        );
    }

    #[test]
    fn one_ip_cannot_spray_many_login_accounts() {
        let limits = RateLimits::default();
        let now = Instant::now();

        for attempt in 0..LOGIN_MAX_ATTEMPTS {
            assert!(
                limits
                    .check(
                        Endpoint::Login,
                        ip(1),
                        &format!("person-{attempt}@example.com"),
                        now,
                    )
                    .is_ok()
            );
        }

        assert!(
            limits
                .check(Endpoint::Login, ip(1), "another@example.com", now)
                .is_err()
        );
    }

    #[test]
    fn many_ips_cannot_concentrate_on_one_login_account() {
        let limits = RateLimits::default();
        let now = Instant::now();

        for attempt in 0..LOGIN_MAX_ATTEMPTS {
            assert!(
                limits
                    .check(
                        Endpoint::Login,
                        ip(attempt as u8 + 1),
                        "person@example.com",
                        now,
                    )
                    .is_ok()
            );
        }

        assert!(
            limits
                .check(Endpoint::Login, ip(200), "person@example.com", now)
                .is_err()
        );
    }

    #[test]
    fn a_login_window_reopens_at_its_boundary() {
        let limits = RateLimits::default();
        let now = Instant::now();

        for _ in 0..LOGIN_MAX_ATTEMPTS {
            limits
                .check(Endpoint::Login, ip(1), "person@example.com", now)
                .expect("within limit");
        }

        assert!(
            limits
                .check(
                    Endpoint::Login,
                    ip(1),
                    "person@example.com",
                    now + LOGIN_WINDOW,
                )
                .is_ok()
        );
    }

    #[test]
    fn review_and_login_budgets_are_independent() {
        let limits = RateLimits::default();
        let now = Instant::now();

        for _ in 0..LOGIN_MAX_ATTEMPTS {
            limits
                .check(Endpoint::Login, ip(1), "account-1", now)
                .expect("within login limit");
        }
        assert!(
            limits
                .check(Endpoint::Review, ip(1), "account-1", now)
                .is_ok()
        );
    }

    #[test]
    fn review_refuses_the_thirty_first_submission() {
        let limits = RateLimits::default();
        let now = Instant::now();

        for _ in 0..REVIEW_MAX_SUBMISSIONS {
            limits
                .check(Endpoint::Review, ip(1), "account-1", now)
                .expect("within review limit");
        }

        assert!(
            limits
                .check(Endpoint::Review, ip(1), "account-1", now)
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejection_is_json_and_tells_the_client_when_to_retry() {
        let response = RetryAfter(Duration::from_millis(1)).response(
            "POST /auth/login",
            ip(1),
            "person@example.com",
        );

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "1");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");

        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .expect("response body");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).expect("JSON body"),
            serde_json::json!({ "error": "Too many requests. Try again later." })
        );
    }
}
