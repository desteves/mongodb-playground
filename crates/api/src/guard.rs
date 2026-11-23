//! Abuse controls inside the app, behind Cloudflare's own rate limits and Cloud Armor.
//!
//! - A **token bucket per client** (keyed by the address Cloudflare reports) for the endpoints that
//!   do real work. Responses say how many requests are left, and a refusal is a structured
//!   `429 rate_limited` with `Retry-After`.
//! - **Two tiers.** Everyone starts on the *basic* tier. A browser that completes Cloudflare
//!   Turnstile gets a signed, short-lived *session* and the higher *verified* tier. Turnstile
//!   therefore raises a limit instead of gating the API, so agents (which cannot solve it) keep
//!   working. Without Turnstile configured (local runs) everyone is on the verified tier.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use hmac::{Hmac, KeyInit, Mac};
use playground_core::PlaygroundError;
use serde_json::{json, Value};
use sha2::Sha256;

use crate::{error_json, ApiError, Shared};

pub const SESSION_HEADER: &str = "x-playground-session";
const SESSION_TTL_SECS: u64 = 3600;
const CLOUDFLARE_VERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tier {
    Basic,
    Verified,
}

/// `(burst, refill per second)` for each tier.
#[derive(Clone, Debug)]
pub struct RateConfig {
    pub basic: (f64, f64),
    pub verified: (f64, f64),
}

impl RateConfig {
    /// Defaults: basic 10 requests then 0.5/s; verified 40 then 4/s. Override with
    /// `RATE_BASIC_BURST`, `RATE_BASIC_PER_SEC`, `RATE_VERIFIED_BURST`, `RATE_VERIFIED_PER_SEC`.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let num = |k: &str, d: f64| {
            get(k)
                .and_then(|v| v.parse().ok())
                .filter(|v: &f64| *v > 0.0)
                .unwrap_or(d)
        };
        Self {
            basic: (
                num("RATE_BASIC_BURST", 10.0),
                num("RATE_BASIC_PER_SEC", 0.5),
            ),
            verified: (
                num("RATE_VERIFIED_BURST", 40.0),
                num("RATE_VERIFIED_PER_SEC", 4.0),
            ),
        }
    }
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

pub struct RateLimiter {
    cfg: RateConfig,
    buckets: Mutex<HashMap<(String, Tier), Bucket>>,
}

#[derive(Debug)]
pub struct Allowed {
    pub remaining: f64,
    pub limit: f64,
}

impl RateLimiter {
    pub fn new(cfg: RateConfig) -> Self {
        Self {
            cfg,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn config(&self) -> &RateConfig {
        &self.cfg
    }

    /// Take `cost` tokens. `Err(seconds)` is how long until the request would fit.
    pub fn check(&self, key: &str, tier: Tier, cost: f64, now: Instant) -> Result<Allowed, u64> {
        let (burst, rate) = match tier {
            Tier::Basic => self.cfg.basic,
            Tier::Verified => self.cfg.verified,
        };
        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 50_000 {
            map.retain(|_, b| now.duration_since(b.at) < Duration::from_secs(600));
        }
        let b = map.entry((key.to_string(), tier)).or_insert(Bucket {
            tokens: burst,
            at: now,
        });
        let elapsed = now.saturating_duration_since(b.at).as_secs_f64();
        b.tokens = (b.tokens + elapsed * rate).min(burst);
        b.at = now;
        if b.tokens >= cost {
            b.tokens -= cost;
            Ok(Allowed {
                remaining: b.tokens,
                limit: burst,
            })
        } else {
            Err((((cost - b.tokens) / rate).ceil() as u64).max(1))
        }
    }
}

// ----- sessions: proof that a browser passed Turnstile -------------------------------------------

fn sign(secret: &[u8], expiry: u64) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    m.update(format!("pg-session|{expiry}").as_bytes());
    m.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// `"<expiry unix seconds>.<hex hmac>"` and its expiry.
pub fn issue_session(secret: &[u8], now_unix: u64) -> (String, u64) {
    let expiry = now_unix + SESSION_TTL_SECS;
    (format!("{expiry}.{}", hex(&sign(secret, expiry))), expiry)
}

pub fn session_valid(secret: &[u8], token: &str, now_unix: u64) -> bool {
    let Some((expiry, sig)) = token.split_once('.') else {
        return false;
    };
    let (Ok(expiry), Some(sig)) = (expiry.parse::<u64>(), unhex(sig)) else {
        return false;
    };
    if expiry <= now_unix {
        return false;
    }
    let mut m = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    m.update(format!("pg-session|{expiry}").as_bytes());
    m.verify_slice(&sig).is_ok()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ----- Turnstile verification -----------------------------------------------------------------------

/// `(token, client address) -> accepted?` Injectable so tests need no network.
pub type Verifier = Arc<
    dyn Fn(String, Option<String>) -> Pin<Box<dyn Future<Output = Result<bool, String>> + Send>>
        + Send
        + Sync,
>;

/// Verify tokens with Cloudflare's siteverify (or `TURNSTILE_VERIFY_URL`, for a local fake).
pub fn cloudflare_verifier(secret: String, url: Option<String>) -> Verifier {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("an HTTP client");
    let url = url.unwrap_or_else(|| CLOUDFLARE_VERIFY_URL.to_string());
    Arc::new(move |token, ip| {
        let (client, secret, url) = (client.clone(), secret.clone(), url.clone());
        Box::pin(async move {
            let mut form = vec![("secret", secret), ("response", token)];
            if let Some(ip) = ip {
                form.push(("remoteip", ip));
            }
            let res = client
                .post(&url)
                .form(&form)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let body: Value = res.json().await.map_err(|e| e.to_string())?;
            Ok(body["success"].as_bool().unwrap_or(false))
        })
    })
}

pub struct Guard {
    pub limiter: RateLimiter,
    /// Its own small bucket, so a client whose main bucket is empty can still get verified,
    /// while the siteverify call cannot be used to hammer Cloudflare.
    pub verify_limiter: RateLimiter,
    pub site_key: Option<String>,
    pub session_secret: Vec<u8>,
    /// `None` when Turnstile is not configured.
    pub verifier: Option<Verifier>,
}

impl Guard {
    pub fn turnstile_enabled(&self) -> bool {
        self.verifier.is_some()
    }

    pub fn tier(&self, headers: &HeaderMap) -> Tier {
        if !self.turnstile_enabled() {
            return Tier::Verified;
        }
        let ok = headers
            .get(SESSION_HEADER)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| session_valid(&self.session_secret, t, now_unix()));
        if ok {
            Tier::Verified
        } else {
            Tier::Basic
        }
    }
}

/// The address to rate limit on: what Cloudflare saw, else the first `X-Forwarded-For`.
pub fn client_key(headers: &HeaderMap) -> String {
    let get = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
    get("cf-connecting-ip")
        .or_else(|| get("x-forwarded-for").and_then(|v| v.split(',').next()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "local".to_string())
}

/// What a request costs. Only POSTs that do work are limited; reads and discovery are free.
pub fn cost(method: &Method, path: &str) -> f64 {
    if method != Method::POST {
        return 0.0;
    }
    match path {
        "/api/v1/run" | "/api/v1/explain" | "/api/v1/share" | "/api/v1/mcp" | "/mcp" => 1.0,
        "/api/v1/validate" => 0.5,
        "/api/v1/events" | "/api/v1/export" => 0.2,
        _ => 0.0,
    }
}

/// Middleware: charge the request to its client's bucket or refuse it with a 429.
pub async fn limit(State(s): State<Shared>, req: Request, next: Next) -> Response {
    let cost = cost(req.method(), req.uri().path());
    if cost == 0.0 {
        return next.run(req).await;
    }
    let key = client_key(req.headers());
    let tier = s.guard.tier(req.headers());
    match s.guard.limiter.check(&key, tier, cost, Instant::now()) {
        Ok(a) => {
            let mut res = next.run(req).await;
            let h = res.headers_mut();
            if let Ok(v) = HeaderValue::from_str(&format!("{}", a.limit as u64)) {
                h.insert("x-ratelimit-limit", v);
            }
            if let Ok(v) = HeaderValue::from_str(&format!("{}", a.remaining.floor() as u64)) {
                h.insert("x-ratelimit-remaining", v);
            }
            res
        }
        Err(retry) => {
            let e = PlaygroundError::RateLimited(retry);
            (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, retry.to_string())],
                Json(json!({ "error": error_json(&e), "tier": if tier == Tier::Verified { "verified" } else { "basic" } })),
            )
                .into_response()
        }
    }
}

/// `POST /api/v1/verify {token}`: exchange a Turnstile token for a one-hour session that unlocks
/// the verified tier. A no-op when Turnstile is not configured.
pub async fn verify(
    State(s): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let Some(verifier) = &s.guard.verifier else {
        return Ok(Json(
            json!({ "verificationRequired": false, "session": null }),
        ));
    };
    if let Err(retry) =
        s.guard
            .verify_limiter
            .check(&client_key(&headers), Tier::Basic, 1.0, Instant::now())
    {
        return Err(ApiError(PlaygroundError::RateLimited(retry)));
    }
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty() && t.len() < 4096)
        .ok_or_else(|| {
            ApiError(PlaygroundError::Invalid(
                "send {\"token\": \"<Turnstile token>\"}".into(),
            ))
        })?;
    let ip = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    match verifier(token.to_string(), ip).await {
        Ok(true) => {
            let (session, expiry) = issue_session(&s.guard.session_secret, now_unix());
            Ok(Json(json!({
                "verificationRequired": true,
                "session": session,
                "expiresAt": expiry,
                "expiresInSeconds": SESSION_TTL_SECS,
            })))
        }
        Ok(false) => Err(ApiError(PlaygroundError::Forbidden(
            "the verification was not accepted; reload the page and try again".into(),
        ))),
        Err(e) => {
            tracing::warn!(error = %e, "turnstile verification could not be completed");
            Err(ApiError(PlaygroundError::Unavailable))
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, middleware::from_fn_with_state, routing::post, Router};
    use playground_core::{Limits, Runner, Shares, Stats};
    use tower::ServiceExt;

    use super::*;
    use crate::AppState;

    fn cfg() -> RateConfig {
        RateConfig {
            basic: (3.0, 1.0),
            verified: (10.0, 5.0),
        }
    }

    #[test]
    fn bucket_allows_a_burst_then_refuses_then_refills() {
        let rl = RateLimiter::new(cfg());
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(rl.check("a", Tier::Basic, 1.0, t0).is_ok());
        }
        let retry = rl.check("a", Tier::Basic, 1.0, t0).unwrap_err();
        assert_eq!(retry, 1, "one token per second");
        assert!(rl
            .check("a", Tier::Basic, 1.0, t0 + Duration::from_millis(1100))
            .is_ok());
        assert!(rl
            .check("a", Tier::Basic, 1.0, t0 + Duration::from_millis(1100))
            .is_err());
        // a long pause refills to the burst, not beyond it
        let later = t0 + Duration::from_secs(60);
        for _ in 0..3 {
            assert!(rl.check("a", Tier::Basic, 1.0, later).is_ok());
        }
        assert!(rl.check("a", Tier::Basic, 1.0, later).is_err());
    }

    #[test]
    fn clients_and_tiers_have_separate_buckets() {
        let rl = RateLimiter::new(cfg());
        let t0 = Instant::now();
        for _ in 0..3 {
            rl.check("a", Tier::Basic, 1.0, t0).unwrap();
        }
        assert!(rl.check("a", Tier::Basic, 1.0, t0).is_err());
        assert!(
            rl.check("b", Tier::Basic, 1.0, t0).is_ok(),
            "another client is unaffected"
        );
        assert!(
            rl.check("a", Tier::Verified, 1.0, t0).is_ok(),
            "the verified tier has its own, larger bucket"
        );
    }

    #[test]
    fn retry_after_scales_with_the_cost_and_rate() {
        let rl = RateLimiter::new(RateConfig {
            basic: (1.0, 0.5),
            verified: (1.0, 0.5),
        });
        let t0 = Instant::now();
        rl.check("a", Tier::Basic, 1.0, t0).unwrap();
        assert_eq!(
            rl.check("a", Tier::Basic, 1.0, t0).unwrap_err(),
            2,
            "1 token at 0.5/s is 2 s"
        );
    }

    #[test]
    fn config_reads_env_and_ignores_junk() {
        let env = |k: &str| match k {
            "RATE_BASIC_BURST" => Some("7".to_string()),
            "RATE_VERIFIED_PER_SEC" => Some("nonsense".to_string()),
            "RATE_BASIC_PER_SEC" => Some("-1".to_string()),
            _ => None,
        };
        let c = RateConfig::from_env(env);
        assert_eq!(c.basic, (7.0, 0.5));
        assert_eq!(c.verified, (40.0, 4.0));
    }

    #[test]
    fn sessions_round_trip_and_reject_tampering_and_expiry() {
        let secret = b"test-secret";
        let (tok, expiry) = issue_session(secret, 1_000);
        assert_eq!(expiry, 1_000 + SESSION_TTL_SECS);
        assert!(session_valid(secret, &tok, 1_001));
        assert!(!session_valid(secret, &tok, expiry), "expired");
        assert!(!session_valid(b"other-secret", &tok, 1_001), "wrong key");
        let (exp, sig) = tok.split_once('.').unwrap();
        let later = format!("{}.{sig}", exp.parse::<u64>().unwrap() + 1000);
        assert!(
            !session_valid(secret, &later, 1_001),
            "a longer expiry cannot be forged"
        );
        for bad in ["", "x", "1.2.3", "99999999999.zz", "99999999999."] {
            assert!(!session_valid(secret, bad, 1_001), "{bad}");
        }
    }

    #[test]
    fn only_working_posts_are_charged() {
        assert_eq!(cost(&Method::GET, "/api/v1/run"), 0.0);
        assert_eq!(cost(&Method::GET, "/api/v1/examples"), 0.0);
        assert_eq!(cost(&Method::POST, "/api/v1/run"), 1.0);
        assert_eq!(cost(&Method::POST, "/mcp"), 1.0);
        assert_eq!(cost(&Method::POST, "/api/v1/validate"), 0.5);
        assert_eq!(
            cost(&Method::POST, "/api/v1/verify"),
            0.0,
            "verify has its own bucket"
        );
        assert_eq!(cost(&Method::POST, "/internal/jobs/sweep"), 0.0);
    }

    #[test]
    fn client_key_prefers_cloudflare() {
        let mut h = HeaderMap::new();
        assert_eq!(client_key(&h), "local");
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.5, 10.0.0.1"),
        );
        assert_eq!(client_key(&h), "203.0.113.5");
        h.insert("cf-connecting-ip", HeaderValue::from_static("198.51.100.7"));
        assert_eq!(client_key(&h), "198.51.100.7");
    }

    async fn state(verifier: Option<Verifier>) -> Shared {
        let runner = Runner::connect(
            "mongodb://localhost:1/?serverSelectionTimeoutMS=200",
            Limits::default(),
        )
        .await
        .unwrap();
        let client = runner.client().clone();
        Arc::new(AppState {
            shares: Shares::new(client.clone(), "x"),
            runner,
            stats: Stats::disabled(),
            examples: Value::Null,
            internal_token: None,
            stats_salt: "t".into(),
            stats_db: "t".into(),
            stats_client: client,
            public_url: "https://t".into(),
            features: crate::Features { sample_data: false },
            guard: Guard {
                limiter: RateLimiter::new(cfg()),
                verify_limiter: RateLimiter::new(RateConfig {
                    basic: (5.0, 0.1),
                    verified: (5.0, 0.1),
                }),
                site_key: verifier.as_ref().map(|_| "site-key".to_string()),
                session_secret: b"secret".to_vec(),
                verifier,
            },
        })
    }

    fn app(s: Shared) -> Router {
        Router::new()
            .route("/api/v1/validate", post(|| async { "ok" }))
            .route("/api/v1/run", post(|| async { "ok" }))
            .route("/api/v1/verify", post(verify))
            .layer(from_fn_with_state(s.clone(), limit))
            .with_state(s)
    }

    fn post_to(path: &str, ip: &str, session: Option<&str>, body: &str) -> Request {
        let mut b = Request::builder()
            .method("POST")
            .uri(path)
            .header("cf-connecting-ip", ip)
            .header("content-type", "application/json");
        if let Some(s) = session {
            b = b.header(SESSION_HEADER, s);
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    fn fake_verifier() -> Verifier {
        Arc::new(|token, _ip| Box::pin(async move { Ok(token == "good") }))
    }

    #[tokio::test]
    async fn the_middleware_answers_429_with_retry_after_and_a_structured_error() {
        // Turnstile off: everyone is on the verified tier (burst 10 at 5/s).
        let app = app(state(None).await);
        let mut last = None;
        for _ in 0..11 {
            last = Some(
                app.clone()
                    .oneshot(post_to("/api/v1/run", "1.1.1.1", None, "{}"))
                    .await
                    .unwrap(),
            );
        }
        let res = last.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(res.headers().get(header::RETRY_AFTER).is_some());
        let body = axum::body::to_bytes(res.into_body(), 1 << 16)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"]["code"], "rate_limited");
        assert!(v["error"]["hint"].as_str().unwrap().contains("Retry-After"));
        // another client is fine, and successful responses say how much is left
        let ok = app
            .clone()
            .oneshot(post_to("/api/v1/run", "2.2.2.2", None, "{}"))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(ok.headers()["x-ratelimit-limit"], "10");
        assert_eq!(ok.headers()["x-ratelimit-remaining"], "9");
    }

    #[tokio::test]
    async fn turnstile_raises_the_limit_after_verification() {
        let app = app(state(Some(fake_verifier())).await);
        // basic tier: burst 3
        for _ in 0..3 {
            assert_eq!(
                app.clone()
                    .oneshot(post_to("/api/v1/run", "9.9.9.9", None, "{}"))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
        }
        assert_eq!(
            app.clone()
                .oneshot(post_to("/api/v1/run", "9.9.9.9", None, "{}"))
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );

        // a bad token is refused
        let bad = app
            .clone()
            .oneshot(post_to(
                "/api/v1/verify",
                "8.8.8.8",
                None,
                r#"{"token":"bad"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::FORBIDDEN);
        // a good one returns a session
        let good = app
            .clone()
            .oneshot(post_to(
                "/api/v1/verify",
                "9.9.9.9",
                None,
                r#"{"token":"good"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(good.status(), StatusCode::OK);
        let v: Value = serde_json::from_slice(
            &axum::body::to_bytes(good.into_body(), 1 << 16)
                .await
                .unwrap(),
        )
        .unwrap();
        let session = v["session"].as_str().unwrap().to_string();
        assert_eq!(v["expiresInSeconds"], 3600);

        // the same address, now with the session, is on the verified tier and may continue
        let res = app
            .clone()
            .oneshot(post_to("/api/v1/run", "9.9.9.9", Some(&session), "{}"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()["x-ratelimit-limit"], "10");
        // a forged session does not help
        let forged = app
            .clone()
            .oneshot(post_to(
                "/api/v1/run",
                "9.9.9.9",
                Some("99999999999.deadbeef"),
                "{}",
            ))
            .await
            .unwrap();
        assert_eq!(forged.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn verify_is_a_noop_without_turnstile_and_validates_its_input() {
        let off = app(state(None).await);
        let res = off
            .oneshot(post_to("/api/v1/verify", "1.1.1.1", None, "{}"))
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(
            &axum::body::to_bytes(res.into_body(), 1 << 16)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(v, json!({"verificationRequired": false, "session": null}));

        let on = app(state(Some(fake_verifier())).await);
        let res = on
            .oneshot(post_to("/api/v1/verify", "1.1.1.1", None, "{}"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
