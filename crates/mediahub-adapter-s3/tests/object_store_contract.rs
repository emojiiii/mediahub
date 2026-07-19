use std::sync::Arc;

use mediahub_adapter_s3::{S3Config, S3ObjectStore};
use mediahub_app::{UploadSessionStorage, object_store_contract::verify_object_store_contract};
use mediahub_core::{
    ApplicationId, BucketId, ClientMetadata, MediaId, NewUploadSession, OffsetDateTime,
    UploadSession, UploadSessionId,
};
use object_store::memory::InMemory;
use uuid::Uuid;

#[tokio::test]
async fn adapter_wrapper_satisfies_shared_object_store_contract() {
    let store = S3ObjectStore::from_backend(Arc::new(InMemory::new()), Some("mediahub-tests"))
        .expect("test store");
    let namespace = format!("contract/{}", Uuid::new_v4());

    verify_object_store_contract(&store, &namespace)
        .await
        .expect("S3 adapter wrapper satisfies the shared contract");
}

#[tokio::test]
#[ignore = "requires an isolated S3-compatible bucket configured by MEDIAHUB_TEST_S3_* variables"]
async fn real_s3_satisfies_shared_object_store_contract() {
    let bucket = required("MEDIAHUB_TEST_S3_BUCKET");
    let region = std::env::var("MEDIAHUB_TEST_S3_REGION").unwrap_or_else(|_| "us-east-1".into());
    let endpoint = std::env::var("MEDIAHUB_TEST_S3_ENDPOINT").ok();
    let allow_http = endpoint
        .as_deref()
        .is_some_and(|value| value.starts_with("http://"));
    let prefix = format!("mediahub-contract/{}", Uuid::new_v4());
    let store = S3Config {
        bucket,
        region,
        endpoint,
        access_key_id: std::env::var("MEDIAHUB_TEST_S3_ACCESS_KEY_ID").ok(),
        secret_access_key: std::env::var("MEDIAHUB_TEST_S3_SECRET_ACCESS_KEY").ok(),
        session_token: std::env::var("MEDIAHUB_TEST_S3_SESSION_TOKEN").ok(),
        allow_http,
        virtual_hosted_style: false,
        prefix: Some(prefix),
    }
    .build()
    .expect("S3 test configuration");

    verify_object_store_contract(&store, "objects")
        .await
        .expect("real S3-compatible backend satisfies the shared contract");

    let body = b"real S3 direct upload contract".to_vec();
    let upload_session_id = UploadSessionId::new();
    let media_id = MediaId::new();
    let now = OffsetDateTime::now_utc();
    let expires_at = now + std::time::Duration::from_secs(600);
    let prepared = store
        .prepare_upload(
            upload_session_id,
            media_id,
            body.len() as u64,
            "text/plain",
            expires_at,
        )
        .await
        .expect("real presigned PUT target");
    let client = reqwest::Client::new();
    let mut request = client.put(&prepared.target.url).body(body.clone());
    for (name, value) in &prepared.target.headers {
        request = request.header(name, value);
    }
    request
        .send()
        .await
        .expect("send real presigned PUT")
        .error_for_status()
        .expect("real presigned PUT accepted");
    let session = UploadSession::new(
        NewUploadSession {
            id: upload_session_id,
            media_id,
            application_id: ApplicationId::new(),
            bucket_id: BucketId::new(),
            object_key: "contract/direct-upload.txt".to_owned(),
            original_name: Some("direct-upload.txt".to_owned()),
            display_name: "direct-upload.txt".to_owned(),
            extension: Some("txt".to_owned()),
            expected_size: body.len() as u64,
            expected_mime: "text/plain".to_owned(),
            storage_backend: "s3".to_owned(),
            storage_key: prepared.storage_key,
            visibility_override: None,
            media_expires_at: None,
            client_metadata: ClientMetadata::default(),
            session_expires_at: expires_at,
        },
        now,
    )
    .expect("real upload session");
    let inspected = store
        .inspect_upload(&session)
        .await
        .expect("inspect real direct upload");
    assert_eq!(inspected.size, body.len() as u64);
    assert_eq!(inspected.mime, "text/plain");
    assert_eq!(
        inspected.sha256,
        "4d70ce74a797134dd325d6b00d5a44f45da44915ec1d93fcc0d7b2376cad693b"
    );
    store
        .abort_upload(&session)
        .await
        .expect("abort real upload");
    store
        .abort_upload(&session)
        .await
        .expect("repeat real upload abort");
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required for the ignored S3 test"))
}
