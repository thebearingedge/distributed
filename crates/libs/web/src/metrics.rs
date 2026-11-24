//! OpenTelemetry-convention HTTP server metrics middleware:
//! [`crate::metrics::with_http_metrics`].

use std::time::Instant;

use axum::{
    Router,
    extract::{MatchedPath, Request, State},
    http::{Method, Uri, uri::Scheme},
    middleware::{self, Next},
    response::Response,
};
use opentelemetry::{
    InstrumentationScope, Key, KeyValue,
    metrics::{Histogram, MeterProvider, UpDownCounter},
};

use telemetry::metrics::{InFlightGuard, duration_histogram};

const HTTP_SERVER_REQUEST_DURATION: &str = "http.server.request.duration";
const HTTP_SERVER_ACTIVE_REQUESTS: &str = "http.server.active_requests";
const HTTP_REQUEST_METHOD: Key = Key::from_static_str("http.request.method");
const HTTP_ROUTE: Key = Key::from_static_str("http.route");
const HTTP_RESPONSE_STATUS_CODE: Key = Key::from_static_str("http.response.status_code");
const URL_SCHEME: Key = Key::from_static_str("url.scheme");
const ERROR_TYPE: Key = Key::from_static_str("error.type");
const HTTP_REQUEST_METHOD_OTHER: &str = "_OTHER";

#[derive(Clone)]
struct HttpServerMetrics {
    duration: Histogram<f64>,
    active_requests: UpDownCounter<i64>,
}

/// Records `http.server.request.duration` (in seconds) and
/// `http.server.active_requests` for every route and fallback that
/// `router` has at call time; routers merged afterwards (e.g. health
/// probes) are not measured, so apply it last.
///
/// The recorded attributes are `http.request.method`, `url.scheme`,
/// `http.response.status_code`, and — on matched requests — `http.route`,
/// the matched route template; unmatched requests record no `http.route`.
/// 5xx responses also record `error.type`, the status code. Methods
/// outside the standard set are recorded as `_OTHER`. A request that is
/// dropped before responding is removed from active requests and records
/// no duration.
///
/// Instruments are created on `meter_provider` under the `web`
/// instrumentation scope.
pub fn with_http_metrics<P>(router: Router, meter_provider: &P) -> Router
where
    P: MeterProvider + ?Sized,
{
    let meter = meter_provider.meter_with_scope(
        InstrumentationScope::builder(env!("CARGO_PKG_NAME"))
            .with_version(env!("CARGO_PKG_VERSION"))
            .build(),
    );
    let metrics = HttpServerMetrics {
        duration: duration_histogram(
            &meter,
            HTTP_SERVER_REQUEST_DURATION,
            "Duration of HTTP server requests.",
        ),
        active_requests: meter
            .i64_up_down_counter(HTTP_SERVER_ACTIVE_REQUESTS)
            .with_unit("{request}")
            .with_description("Number of active HTTP server requests.")
            .build(),
    };
    router.layer(middleware::from_fn_with_state(metrics, record))
}

fn method_attribute(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::CONNECT => "CONNECT",
        Method::OPTIONS => "OPTIONS",
        Method::TRACE => "TRACE",
        Method::PATCH => "PATCH",
        Method::QUERY => "QUERY",
        _ => HTTP_REQUEST_METHOD_OTHER,
    }
}

fn scheme_attribute(uri: &Uri) -> &'static str {
    match uri.scheme() {
        Some(scheme) if *scheme == Scheme::HTTPS => "https",
        _ => "http",
    }
}

async fn record(State(metrics): State<HttpServerMetrics>, req: Request, next: Next) -> Response {
    let method = KeyValue::new(HTTP_REQUEST_METHOD, method_attribute(req.method()));
    let scheme = KeyValue::new(URL_SCHEME, scheme_attribute(req.uri()));
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|path| KeyValue::new(HTTP_ROUTE, path.as_str().to_owned()));
    let _in_flight = InFlightGuard::start(
        &metrics.active_requests,
        vec![method.clone(), scheme.clone()],
    );
    let started = Instant::now();
    let response = next.run(req).await;
    let status = response.status();
    let mut attributes = vec![
        method,
        scheme,
        KeyValue::new(HTTP_RESPONSE_STATUS_CODE, i64::from(status.as_u16())),
    ];
    attributes.extend(route);
    attributes.extend(
        status
            .is_server_error()
            .then(|| KeyValue::new(ERROR_TYPE, status.as_str().to_owned())),
    );
    metrics
        .duration
        .record(started.elapsed().as_secs_f64(), &attributes);
    response
}

#[cfg(test)]
mod tests {
    use std::{future, sync::Arc};

    use axum::{body::Body, http::StatusCode, response::Response, routing::get};
    use opentelemetry::Value;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use super::*;
    use crate::fallback::with_json_fallbacks;
    use telemetry::test_support::MetricsHarness;

    fn request(method: &str, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    fn router(harness: &MetricsHarness) -> Router {
        with_http_metrics(
            with_json_fallbacks(
                Router::new()
                    .route("/things/{id}", get(|| async { "ok" }))
                    .route("/boom", get(|| async { StatusCode::INTERNAL_SERVER_ERROR })),
            ),
            harness.provider(),
        )
    }

    #[tokio::test]
    async fn matched_requests_record_the_route_template() {
        let harness = MetricsHarness::default();
        router(&harness)
            .oneshot(request("GET", "/things/1"))
            .await
            .unwrap();

        let points = harness.f64_histogram(HTTP_SERVER_REQUEST_DURATION);
        assert_eq!(points.len(), 1);
        let point = &points[0];
        assert_eq!(point.count, 1);
        assert_eq!(
            point.attribute(HTTP_ROUTE.as_str()),
            Some(&Value::from("/things/{id}"))
        );
        assert_eq!(
            point.attribute(HTTP_REQUEST_METHOD.as_str()),
            Some(&Value::from("GET"))
        );
        assert_eq!(
            point.attribute(HTTP_RESPONSE_STATUS_CODE.as_str()),
            Some(&Value::I64(200))
        );
        assert_eq!(
            point.attribute(URL_SCHEME.as_str()),
            Some(&Value::from("http"))
        );
        assert_eq!(point.attribute(ERROR_TYPE.as_str()), None);
        assert_eq!(
            point.bounds,
            telemetry::metrics::DURATION_HISTOGRAM_BOUNDARIES.to_vec()
        );
    }

    #[tokio::test]
    async fn unmatched_requests_record_no_route() {
        let harness = MetricsHarness::default();
        router(&harness)
            .oneshot(request("GET", "/nope"))
            .await
            .unwrap();

        let points = harness.f64_histogram(HTTP_SERVER_REQUEST_DURATION);
        assert_eq!(points.len(), 1);
        assert_eq!(
            points[0].attribute(HTTP_RESPONSE_STATUS_CODE.as_str()),
            Some(&Value::I64(404))
        );
        assert_eq!(points[0].attribute(HTTP_ROUTE.as_str()), None);
    }

    #[tokio::test]
    async fn method_not_allowed_requests_keep_the_route() {
        let harness = MetricsHarness::default();
        router(&harness)
            .oneshot(request("POST", "/things/1"))
            .await
            .unwrap();

        let points = harness.f64_histogram(HTTP_SERVER_REQUEST_DURATION);
        assert_eq!(points.len(), 1);
        assert_eq!(
            points[0].attribute(HTTP_RESPONSE_STATUS_CODE.as_str()),
            Some(&Value::I64(405))
        );
        assert_eq!(
            points[0].attribute(HTTP_ROUTE.as_str()),
            Some(&Value::from("/things/{id}"))
        );
    }

    #[tokio::test]
    async fn server_errors_record_the_error_type() {
        let harness = MetricsHarness::default();
        router(&harness)
            .oneshot(request("GET", "/boom"))
            .await
            .unwrap();

        let points = harness.f64_histogram(HTTP_SERVER_REQUEST_DURATION);
        assert_eq!(points.len(), 1);
        assert_eq!(
            points[0].attribute(ERROR_TYPE.as_str()),
            Some(&Value::from("500"))
        );
    }

    #[tokio::test]
    async fn unknown_methods_are_recorded_as_other() {
        let harness = MetricsHarness::default();
        router(&harness)
            .oneshot(request("FOO", "/things/1"))
            .await
            .unwrap();

        let points = harness.f64_histogram(HTTP_SERVER_REQUEST_DURATION);
        assert_eq!(points.len(), 1);
        assert_eq!(
            points[0].attribute(HTTP_REQUEST_METHOD.as_str()),
            Some(&Value::from(HTTP_REQUEST_METHOD_OTHER))
        );
    }

    #[tokio::test]
    async fn active_requests_return_to_zero_after_completion() {
        let harness = MetricsHarness::default();
        router(&harness)
            .oneshot(request("GET", "/things/1"))
            .await
            .unwrap();

        let points = harness.i64_sum(HTTP_SERVER_ACTIVE_REQUESTS);
        assert!(!points.is_empty());
        assert!(points.iter().all(|point| point.value == 0));
    }

    #[tokio::test]
    async fn dropped_requests_leave_active_requests_and_record_no_duration() {
        let harness = MetricsHarness::default();
        let entered = Arc::new(Notify::new());
        let router = with_http_metrics(
            with_json_fallbacks(Router::new().route(
                "/slow",
                get({
                    let entered = Arc::clone(&entered);
                    move || async move {
                        entered.notify_one();
                        future::pending::<Response>().await
                    }
                }),
            )),
            harness.provider(),
        );
        let handle = tokio::spawn(router.oneshot(request("GET", "/slow")));
        entered.notified().await;

        let points = harness.i64_sum(HTTP_SERVER_ACTIVE_REQUESTS);
        assert!(!points.is_empty());
        assert!(points.iter().all(|point| point.value == 1));

        handle.abort();
        assert!(handle.await.unwrap_err().is_cancelled());

        let points = harness.i64_sum(HTTP_SERVER_ACTIVE_REQUESTS);
        assert!(!points.is_empty());
        assert!(points.iter().all(|point| point.value == 0));
        assert!(
            harness
                .f64_histogram(HTTP_SERVER_REQUEST_DURATION)
                .is_empty()
        );
    }

    #[tokio::test]
    async fn routes_merged_afterwards_are_not_measured() {
        let harness = MetricsHarness::default();
        let router = with_http_metrics(Router::new(), harness.provider())
            .merge(Router::new().route("/livez", get(|| async { "ok" })));
        router.oneshot(request("GET", "/livez")).await.unwrap();

        assert!(
            harness
                .f64_histogram(HTTP_SERVER_REQUEST_DURATION)
                .is_empty()
        );
    }

    #[test]
    fn standard_methods_keep_their_names() {
        assert_eq!(method_attribute(&Method::GET), Method::GET.as_str());
        assert_eq!(method_attribute(&Method::HEAD), Method::HEAD.as_str());
        assert_eq!(method_attribute(&Method::POST), Method::POST.as_str());
        assert_eq!(method_attribute(&Method::PUT), Method::PUT.as_str());
        assert_eq!(method_attribute(&Method::DELETE), Method::DELETE.as_str());
        assert_eq!(method_attribute(&Method::CONNECT), Method::CONNECT.as_str());
        assert_eq!(method_attribute(&Method::OPTIONS), Method::OPTIONS.as_str());
        assert_eq!(method_attribute(&Method::TRACE), Method::TRACE.as_str());
        assert_eq!(method_attribute(&Method::PATCH), Method::PATCH.as_str());
        assert_eq!(method_attribute(&Method::QUERY), Method::QUERY.as_str());
    }

    #[test]
    fn extension_methods_map_to_other() {
        assert_eq!(
            method_attribute(&Method::from_bytes(b"FOO").unwrap()),
            HTTP_REQUEST_METHOD_OTHER
        );
    }

    #[test]
    fn https_uris_report_https() {
        let uri = "https://example.com/x".parse::<Uri>().unwrap();
        assert_eq!(scheme_attribute(&uri), "https");
    }

    #[test]
    fn other_uris_report_http() {
        let relative = "/x".parse::<Uri>().unwrap();
        assert_eq!(scheme_attribute(&relative), "http");
        let explicit = "http://example.com/x".parse::<Uri>().unwrap();
        assert_eq!(scheme_attribute(&explicit), "http");
    }
}
