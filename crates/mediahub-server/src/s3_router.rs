use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, middleware, routing::get};
use tower_http::trace::TraceLayer;

use crate::{
    AppState, MAX_REQUEST_BYTES, MAX_S3_CONTROL_REQUEST_BYTES, authenticate_hmac_request,
    metrics_middleware, request_id_middleware, s3_http,
};

const S3_ROOT_PATH: &str = "/";
const S3_BUCKET_PATH: &str = "/{bucket}";
const S3_OBJECT_PATH: &str = "/{bucket}/{*object_key}";

pub(super) fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(S3_ROOT_PATH, get(s3_http::s3_list_buckets))
        .route(
            S3_BUCKET_PATH,
            get(s3_http::s3_bucket_get)
                .head(s3_http::s3_head_bucket)
                .put(s3_http::s3_bucket_put)
                .post(s3_http::s3_bucket_post)
                .options(s3_http::s3_options_bucket)
                .delete(s3_http::s3_bucket_delete)
                .layer(DefaultBodyLimit::max(MAX_S3_CONTROL_REQUEST_BYTES)),
        )
        .route(
            S3_OBJECT_PATH,
            get(s3_http::s3_get_object)
                .head(s3_http::s3_get_object)
                .put(s3_http::s3_put_object)
                .post(s3_http::s3_post_object)
                .options(s3_http::s3_options_object)
                .delete(s3_http::s3_delete_object)
                .layer(DefaultBodyLimit::max(MAX_S3_CONTROL_REQUEST_BYTES)),
        )
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            s3_http::s3_cors_response_middleware,
        ))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            authenticate_hmac_request,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            metrics_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(request_id_middleware))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::{S3_BUCKET_PATH, S3_OBJECT_PATH, S3_ROOT_PATH};

    #[test]
    fn route_shape_is_native_path_style_s3_without_legacy_prefix() {
        assert_eq!(S3_ROOT_PATH, "/");
        assert_eq!(S3_BUCKET_PATH, "/{bucket}");
        assert_eq!(S3_OBJECT_PATH, "/{bucket}/{*object_key}");
        for path in [S3_ROOT_PATH, S3_BUCKET_PATH, S3_OBJECT_PATH] {
            assert!(!path.starts_with("/s3"));
        }
    }
}
