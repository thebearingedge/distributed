//! CORS layer composition for browser clients.

use std::time::Duration;

use axum::{
    Router,
    http::{HeaderName, HeaderValue, Method, header},
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::request_id::REQUEST_ID_HEADER;

/// `idempotency-key` request header, for callers whose API accepts idempotent
/// operations.
pub const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");

/// `traceparent` request header, as read by [`crate::middleware::with_http_tracing`].
pub const TRACEPARENT: HeaderName = HeaderName::from_static("traceparent");

/// `tracestate` request header, as read by [`crate::middleware::with_http_tracing`].
pub const TRACESTATE: HeaderName = HeaderName::from_static("tracestate");

const CORS_MAX_AGE: Duration = Duration::from_secs(600);

/// CORS policy for [`with_cors`]: exact origins plus the allowed methods and
/// headers advertised on preflight responses.
#[derive(Debug, Clone)]
pub struct CorsConfig {
    /// Exact origins allowed to make cross-origin requests; empty means CORS
    /// is off. Never a wildcard.
    pub allowed_origins: Vec<HeaderValue>,
    /// Methods advertised as `Access-Control-Allow-Methods`.
    pub allowed_methods: Vec<Method>,
    /// Headers advertised as `Access-Control-Allow-Headers`.
    pub allowed_headers: Vec<HeaderName>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: Vec::new(),
            allowed_methods: vec![Method::GET, Method::POST, Method::PUT, Method::DELETE],
            allowed_headers: vec![
                header::CONTENT_TYPE,
                IDEMPOTENCY_KEY,
                TRACEPARENT,
                TRACESTATE,
            ],
        }
    }
}

/// Errors from [`parse_allowed_origins`].
#[derive(Debug, thiserror::Error)]
pub enum ParseOriginsError {
    /// An origin ended with a trailing slash.
    #[error("CORS origin must not end with a slash: {origin}")]
    TrailingSlash {
        /// The rejected origin.
        origin: String,
    },
    /// An origin was not a valid header value.
    #[error("invalid CORS origin: {origin}")]
    Invalid {
        /// The rejected origin.
        origin: String,
        /// The underlying header-value parse failure.
        #[source]
        source: axum::http::header::InvalidHeaderValue,
    },
}

/// Parses a comma-separated origins value into exact-origin header values.
/// Entries are trimmed, empty entries are dropped, and empty input yields an
/// empty list (CORS off).
///
/// # Errors
///
/// Returns [`ParseOriginsError::TrailingSlash`] when an entry ends with a
/// slash and [`ParseOriginsError::Invalid`] when an entry is not a valid
/// header value.
pub fn parse_allowed_origins(raw: &str) -> Result<Vec<HeaderValue>, ParseOriginsError> {
    raw.split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(|origin| {
            if origin.ends_with('/') {
                return Err(ParseOriginsError::TrailingSlash {
                    origin: origin.to_owned(),
                });
            }
            HeaderValue::from_str(origin).map_err(|source| ParseOriginsError::Invalid {
                origin: origin.to_owned(),
                source,
            })
        })
        .collect()
}

/// Wraps `router` with a [`CorsLayer`] according to `config`: requests whose
/// `Origin` is not in `config.allowed_origins` receive no CORS headers, and
/// the layer never emits a wildcard `Access-Control-Allow-Origin` or reflects
/// an arbitrary request origin.
///
/// Method and header policy is not baked in here: defaults live in
/// [`CorsConfig::default`], and callers construct or override the policy via
/// struct literal.
///
/// Preflight `OPTIONS` requests are answered directly by the layer without
/// reaching `router`; responses to actual requests from allowed origins carry
/// the response headers named by
/// [`Access-Control-Expose-Headers`](https://fetch.spec.whatwg.org/#http-access-control-expose-headers)
/// (`location` and [`crate::request_id::REQUEST_ID_HEADER`]).
///
/// Empty `config.allowed_origins` leaves the router unchanged (CORS off), so
/// callers without configured origins get no CORS layer at all.
///
/// # Panics
///
/// Panics when `config.allowed_origins` contains a wildcard (`*`) value, via
/// [`AllowOrigin::list`].
pub fn with_cors(router: Router, config: CorsConfig) -> Router {
    if config.allowed_origins.is_empty() {
        return router;
    }
    router.layer(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(config.allowed_origins))
            .allow_methods(config.allowed_methods)
            .allow_headers(config.allowed_headers)
            .expose_headers([header::LOCATION, REQUEST_ID_HEADER])
            .max_age(CORS_MAX_AGE),
    )
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{self, HeaderValue, Method, Request, StatusCode},
        response::Response,
        routing::get,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::{CorsConfig, parse_allowed_origins, with_cors};
    use crate::request_id::REQUEST_ID_HEADER;

    const ALLOWED_ORIGIN: &str = "http://localhost:5173";
    const UNLISTED_ORIGIN: &str = "http://localhost:9999";
    const ALLOWED_METHODS: [Method; 4] = [Method::GET, Method::POST, Method::PUT, Method::DELETE];
    const ADVERTISED_METHODS: &str = "GET,POST,PUT,DELETE";
    const PREFLIGHT_REQUEST_HEADERS: &str = "content-type, idempotency-key";

    fn router() -> Router {
        Router::new().route("/todos", get(|| async { "ok" }))
    }

    fn allowed_origins() -> Vec<HeaderValue> {
        vec![HeaderValue::from_static(ALLOWED_ORIGIN)]
    }

    fn with_test_cors(router: Router) -> Router {
        with_cors(
            router,
            CorsConfig {
                allowed_origins: allowed_origins(),
                ..CorsConfig::default()
            },
        )
    }

    fn preflight(origin: &str, request_method: &Method, request_headers: &str) -> Request<Body> {
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/todos")
            .header(http::header::ORIGIN, origin)
            .header(
                http::header::ACCESS_CONTROL_REQUEST_METHOD,
                HeaderValue::from_str(request_method.as_str()).unwrap(),
            )
            .header(
                http::header::ACCESS_CONTROL_REQUEST_HEADERS,
                request_headers,
            )
            .body(Body::empty())
            .unwrap()
    }

    fn request(method: Method, uri: &str, origin: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(http::header::ORIGIN, origin)
            .body(Body::empty())
            .unwrap()
    }

    async fn body(res: Response) -> Vec<u8> {
        res.into_body().collect().await.unwrap().to_bytes().to_vec()
    }

    fn cors_headers(res: &Response) -> Vec<&str> {
        res.headers()
            .keys()
            .filter_map(|name| {
                name.as_str()
                    .starts_with("access-control-")
                    .then_some(name.as_str())
            })
            .collect()
    }

    #[tokio::test]
    async fn empty_origin_list_leaves_the_router_without_cors() {
        let res = with_cors(router(), CorsConfig::default())
            .oneshot(preflight(
                ALLOWED_ORIGIN,
                &Method::POST,
                PREFLIGHT_REQUEST_HEADERS,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(cors_headers(&res), [] as [&str; 0]);
    }

    #[tokio::test]
    async fn allowed_origin_preflight_is_answered_with_the_exact_origin() {
        let res = with_test_cors(router())
            .oneshot(preflight(
                ALLOWED_ORIGIN,
                &Method::POST,
                PREFLIGHT_REQUEST_HEADERS,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get("access-control-allow-origin").unwrap(),
            ALLOWED_ORIGIN
        );
        assert_eq!(
            res.headers().get("access-control-allow-methods").unwrap(),
            ADVERTISED_METHODS
        );
        assert!(
            res.headers()
                .get("access-control-allow-headers")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("idempotency-key")
        );
        assert_eq!(res.headers().get("access-control-max-age").unwrap(), "600");
    }

    #[tokio::test]
    async fn preflight_is_answered_for_every_allowed_method() {
        for method in ALLOWED_METHODS {
            let res = with_test_cors(router())
                .oneshot(preflight(
                    ALLOWED_ORIGIN,
                    &method,
                    PREFLIGHT_REQUEST_HEADERS,
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn preflight_method_outside_the_allowed_list_is_not_advertised() {
        let res = with_test_cors(router())
            .oneshot(preflight(
                ALLOWED_ORIGIN,
                &Method::PATCH,
                PREFLIGHT_REQUEST_HEADERS,
            ))
            .await
            .unwrap();
        let methods = res
            .headers()
            .get("access-control-allow-methods")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(methods, ADVERTISED_METHODS);
        assert!(!methods.contains(Method::PATCH.as_str()));
    }

    #[tokio::test]
    async fn preflight_header_outside_the_allowed_list_is_not_advertised() {
        let res = with_test_cors(router())
            .oneshot(preflight(
                ALLOWED_ORIGIN,
                &Method::POST,
                "content-type, x-not-allowed",
            ))
            .await
            .unwrap();
        let headers = res
            .headers()
            .get("access-control-allow-headers")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(headers.contains("content-type"));
        assert!(!headers.contains("x-not-allowed"));
    }

    #[tokio::test]
    async fn unlisted_origin_preflight_has_no_allow_origin() {
        let res = with_test_cors(router())
            .oneshot(preflight(
                UNLISTED_ORIGIN,
                &Method::POST,
                PREFLIGHT_REQUEST_HEADERS,
            ))
            .await
            .unwrap();
        assert!(res.headers().get("access-control-allow-origin").is_none());
    }

    #[tokio::test]
    async fn actual_request_from_allowed_origin_carries_expose_headers() {
        let res = with_test_cors(router())
            .oneshot(request(Method::GET, "/todos", ALLOWED_ORIGIN))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get("access-control-allow-origin").unwrap(),
            ALLOWED_ORIGIN
        );
        let expose = res
            .headers()
            .get("access-control-expose-headers")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(expose.contains("location"));
        assert!(expose.contains(REQUEST_ID_HEADER.as_str()));
        assert_eq!(body(res).await, b"ok".to_vec());
    }

    #[test]
    fn empty_raw_yields_no_origins() {
        assert_eq!(
            parse_allowed_origins("").unwrap(),
            Vec::<HeaderValue>::new()
        );
    }

    #[test]
    fn entries_are_trimmed() {
        let origins =
            parse_allowed_origins(" http://localhost:5173 , http://localhost:3000 ").unwrap();
        assert_eq!(
            origins,
            vec![
                HeaderValue::from_static("http://localhost:5173"),
                HeaderValue::from_static("http://localhost:3000"),
            ]
        );
    }

    #[test]
    fn trailing_slash_is_rejected_with_the_origin_named() {
        let err = parse_allowed_origins("http://localhost:5173/").unwrap_err();
        assert!(err.to_string().contains("http://localhost:5173"));
    }

    #[test]
    fn invalid_header_value_is_rejected() {
        let err = parse_allowed_origins("http://bad\u{1}origin.com").unwrap_err();
        assert!(err.to_string().contains("invalid CORS origin"));
    }

    #[test]
    fn empty_entries_between_commas_are_dropped() {
        let origins =
            parse_allowed_origins("http://localhost:5173,,http://localhost:3000").unwrap();
        assert_eq!(
            origins,
            vec![
                HeaderValue::from_static("http://localhost:5173"),
                HeaderValue::from_static("http://localhost:3000"),
            ]
        );
    }
}
