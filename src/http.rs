use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use tokio::sync::mpsc::{Sender, error::TrySendError};
use tracing::warn;

use crate::model::LogEntry;

const MAX_BODY_SIZE: usize = 16 * 1024 * 1024;

pub fn router(queue: Sender<LogEntry>) -> Router {
    Router::new()
        .route("/log", post(push_log))
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .with_state(queue)
}

/// Queues the event and answers before it is written: the applications send
/// their events fire-and-forget.
async fn push_log(State(queue): State<Sender<LogEntry>>, body: Bytes) -> Response {
    // The first JSON value of the body, whatever follows it and whatever the
    // Content-Type header says.
    let entry = match serde_json::Deserializer::from_slice(&body)
        .into_iter::<LogEntry>()
        .next()
    {
        Some(Ok(entry)) => entry,
        Some(Err(err)) => {
            warn!("Unable to decode JSON: {err}");
            return text_error(StatusCode::BAD_REQUEST, "Invalid JSON");
        }
        None => {
            warn!("Unable to decode JSON: empty body");
            return text_error(StatusCode::BAD_REQUEST, "Invalid JSON");
        }
    };

    match queue.try_send(entry) {
        Ok(()) => ([(header::CONTENT_TYPE, "application/json")], "true").into_response(),
        Err(TrySendError::Full(_)) => text_error(StatusCode::SERVICE_UNAVAILABLE, "Log queue full"),
        Err(TrySendError::Closed(_)) => {
            text_error(StatusCode::SERVICE_UNAVAILABLE, "Shutting down")
        }
    }
}

fn text_error(status: StatusCode, message: &str) -> Response {
    let mut response = (status, format!("{message}\n")).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use serde_json::json;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    async fn post_log(router: Router, body: impl Into<Body>) -> (StatusCode, String, String) {
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/log")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body.into())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            content_type,
            String::from_utf8(body.to_vec()).unwrap(),
        )
    }

    #[tokio::test]
    async fn queues_the_entry() {
        let (tx, mut rx) = mpsc::channel(10);

        let (status, content_type, body) = post_log(
            router(tx),
            r#"{"appName":"databox","action":"test","eventDate":1750000000,"payload":{"a":1}}"#,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "application/json");
        assert_eq!(body, "true");
        assert_eq!(
            rx.try_recv().unwrap(),
            LogEntry {
                app_name: "databox".into(),
                action: "test".into(),
                event_date: 1750000000,
                payload: Some(json!({"a": 1})),
                ..LogEntry::default()
            }
        );
    }

    #[tokio::test]
    async fn ignores_what_follows_the_first_json_value() {
        let (tx, mut rx) = mpsc::channel(10);

        let (status, _, _) = post_log(router(tx), "{\"action\":\"test\"}\ntrailing").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(rx.try_recv().unwrap().action, "test");
    }

    #[tokio::test]
    async fn rejects_invalid_json() {
        for body in ["{\"action\":", "", "[1]", "{\"eventDate\":\"x\"}"] {
            let (tx, mut rx) = mpsc::channel(10);

            let (status, content_type, response) = post_log(router(tx), body).await;

            assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
            assert_eq!(content_type, "text/plain; charset=utf-8");
            assert_eq!(response, "Invalid JSON\n");
            assert!(rx.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn answers_503_when_the_queue_is_full() {
        let (tx, _rx) = mpsc::channel(1);
        let router = router(tx);

        let (first, _, _) = post_log(router.clone(), "{}").await;
        let (second, _, body) = post_log(router, "{}").await;

        assert_eq!(first, StatusCode::OK);
        assert_eq!(second, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, "Log queue full\n");
    }

    #[tokio::test]
    async fn only_accepts_post_on_log() {
        let (tx, _rx) = mpsc::channel(1);

        let get = router(tx.clone())
            .oneshot(Request::get("/log").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let other = router(tx)
            .oneshot(Request::post("/other").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(get.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(other.status(), StatusCode::NOT_FOUND);
    }
}
