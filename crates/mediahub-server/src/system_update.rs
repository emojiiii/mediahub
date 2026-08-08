use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::http::{StatusCode, header};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info};
use url::Url;
use uuid::Uuid;

use super::{OffsetDateTime, server_config::SystemUpdateConfig};

const GITHUB_REPOSITORY: &str = "emojiiii/mediahub";
const GITHUB_WORKFLOW: &str = "ci.yml";
const GITHUB_API_BASE: &str = "https://api.github.com";
const LATEST_BUILD_CACHE_TTL: Duration = Duration::from_secs(20 * 60);
const UPDATE_TRIGGER_DELAY: Duration = Duration::from_millis(750);
const VERSION_CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct BuildInfo {
    pub(super) version: String,
    pub(super) revision: Option<String>,
    pub(super) channel: String,
}

impl BuildInfo {
    pub(super) fn current() -> Self {
        let version = option_env!("MEDIAHUB_BUILD_VERSION")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(concat!(env!("CARGO_PKG_VERSION"), "-dev"));
        let revision = option_env!("MEDIAHUB_BUILD_REVISION")
            .filter(|value| !value.trim().is_empty() && *value != "unknown")
            .map(str::to_owned);
        let channel = option_env!("MEDIAHUB_UPDATE_CHANNEL")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("dev");
        Self {
            version: version.to_owned(),
            revision,
            channel: channel.to_owned(),
        }
    }

    fn source_url(&self) -> String {
        self.revision.as_ref().map_or_else(
            || format!("https://github.com/{GITHUB_REPOSITORY}"),
            |revision| format!("https://github.com/{GITHUB_REPOSITORY}/commit/{revision}"),
        )
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct LatestBuild {
    pub(super) version: String,
    pub(super) revision: String,
    pub(super) source_url: String,
    pub(super) published_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum UpdatePhase {
    #[default]
    Idle,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub(super) struct UpdateOperation {
    pub(super) phase: UpdatePhase,
    pub(super) operation_id: Option<String>,
    pub(super) from_version: Option<String>,
    pub(super) target_version: Option<String>,
    pub(super) started_at: Option<OffsetDateTime>,
    pub(super) completed_at: Option<OffsetDateTime>,
    pub(super) message: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct SystemVersionInfo {
    pub(super) current_version: String,
    pub(super) current_revision: Option<String>,
    pub(super) channel: String,
    pub(super) current_source_url: String,
    pub(super) latest_build: Option<LatestBuild>,
    pub(super) has_update: Option<bool>,
    pub(super) update_enabled: bool,
    pub(super) warning: Option<String>,
    pub(super) operation: UpdateOperation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpdateTriggerError {
    Disabled,
    InProgress,
}

#[derive(Clone)]
pub(super) struct SystemUpdateService {
    client: Client,
    config: SystemUpdateConfig,
    build: BuildInfo,
    github_api_base: String,
    latest_cache: Arc<RwLock<Option<CachedLatestBuild>>>,
    operation: Arc<Mutex<UpdateOperation>>,
    trigger_delay: Duration,
}

impl std::fmt::Debug for SystemUpdateService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SystemUpdateService")
            .field("build", &self.build)
            .field("update_enabled", &self.config.enabled())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct CachedLatestBuild {
    cached_at: Instant,
    build: LatestBuild,
}

#[derive(Debug, Deserialize)]
struct WorkflowRunsResponse {
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Debug, Deserialize)]
struct WorkflowRun {
    head_sha: String,
    html_url: String,
    updated_at: DateTime<Utc>,
}

impl SystemUpdateConfig {
    fn enabled(&self) -> bool {
        self.updater_url.is_some() && self.updater_token.is_some()
    }
}

impl SystemUpdateService {
    pub(super) fn new(config: SystemUpdateConfig) -> Result<Self, reqwest::Error> {
        Self::with_endpoints(
            config,
            BuildInfo::current(),
            GITHUB_API_BASE.to_owned(),
            UPDATE_TRIGGER_DELAY,
        )
    }

    fn with_endpoints(
        config: SystemUpdateConfig,
        build: BuildInfo,
        github_api_base: String,
        trigger_delay: Duration,
    ) -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(format!("PrismArk/{}", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            config,
            build,
            github_api_base,
            latest_cache: Arc::new(RwLock::new(None)),
            operation: Arc::new(Mutex::new(UpdateOperation::default())),
            trigger_delay,
        })
    }

    pub(super) async fn status(&self, force: bool) -> SystemVersionInfo {
        let (latest_build, warning) = match self.latest_build(force).await {
            Ok(build) => (build, None),
            Err(message) => (None, Some(message)),
        };
        let has_update = self.build.revision.as_ref().and_then(|current| {
            latest_build
                .as_ref()
                .map(|latest| latest.revision != *current)
        });
        SystemVersionInfo {
            current_version: self.build.version.clone(),
            current_revision: self.build.revision.clone(),
            channel: self.build.channel.clone(),
            current_source_url: self.build.source_url(),
            latest_build,
            has_update,
            update_enabled: self.config.enabled(),
            warning,
            operation: self.operation.lock().await.clone(),
        }
    }

    pub(super) async fn trigger_update(&self) -> Result<UpdateOperation, UpdateTriggerError> {
        if !self.config.enabled() {
            return Err(UpdateTriggerError::Disabled);
        }
        let latest = self
            .latest_cache
            .read()
            .await
            .as_ref()
            .map(|cached| cached.build.clone());
        let mut current = self.operation.lock().await;
        if current.phase == UpdatePhase::Running {
            return Err(UpdateTriggerError::InProgress);
        }
        let operation = UpdateOperation {
            phase: UpdatePhase::Running,
            operation_id: Some(format!("update_{}", Uuid::new_v4().simple())),
            from_version: Some(self.build.version.clone()),
            target_version: latest.map(|build| build.version),
            started_at: Some(OffsetDateTime::now_utc()),
            completed_at: None,
            message: Some("已提交镜像更新检查，服务可能短暂重启".to_owned()),
        };
        *current = operation.clone();
        drop(current);
        let service = self.clone();
        let operation_id = operation.operation_id.clone().unwrap_or_default();
        tokio::spawn(async move {
            tokio::time::sleep(service.trigger_delay).await;
            service.run_update(&operation_id).await;
        });
        Ok(operation)
    }

    async fn latest_build(&self, force: bool) -> Result<Option<LatestBuild>, String> {
        let Some(branch) = workflow_branch(&self.build.channel) else {
            return Ok(None);
        };
        if !force
            && let Some(cached) = self.latest_cache.read().await.as_ref()
            && cached.cached_at.elapsed() < LATEST_BUILD_CACHE_TTL
        {
            return Ok(Some(cached.build.clone()));
        }
        let endpoint = format!(
            "{}/repos/{GITHUB_REPOSITORY}/actions/workflows/{GITHUB_WORKFLOW}/runs",
            self.github_api_base.trim_end_matches('/')
        );
        let mut endpoint = Url::parse(&endpoint).map_err(|_| "版本服务地址配置无效".to_owned())?;
        endpoint
            .query_pairs_mut()
            .append_pair("branch", branch)
            .append_pair("status", "success")
            .append_pair("per_page", "1");
        let mut request = self
            .client
            .get(endpoint)
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(VERSION_CHECK_TIMEOUT);
        if let Some(token) = self.config.github_token.as_deref() {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|_| "暂时无法连接版本服务".to_owned())?;
        if !response.status().is_success() {
            return Err(github_status_warning(
                response.status(),
                self.config.github_token.is_some(),
            ));
        }
        let payload = response
            .json::<WorkflowRunsResponse>()
            .await
            .map_err(|_| "版本服务返回了无法识别的数据".to_owned())?;
        let Some(run) = payload.workflow_runs.into_iter().next() else {
            return Err("当前更新通道还没有成功构建".to_owned());
        };
        if run.head_sha.len() < 12 {
            return Err("版本服务返回了无效的构建标识".to_owned());
        }
        let published_at = OffsetDateTime::from_unix_timestamp(run.updated_at.timestamp())
            .map_err(|_| "版本服务返回了无效的构建时间".to_owned())?;
        let latest = LatestBuild {
            version: format!("{}-{}", self.build.channel, &run.head_sha[..12]),
            revision: run.head_sha,
            source_url: run.html_url,
            published_at,
        };
        *self.latest_cache.write().await = Some(CachedLatestBuild {
            cached_at: Instant::now(),
            build: latest.clone(),
        });
        Ok(Some(latest))
    }

    async fn run_update(&self, operation_id: &str) {
        let (Some(updater_url), Some(updater_token)) = (
            self.config.updater_url.as_deref(),
            self.config.updater_token.as_deref(),
        ) else {
            self.finish_operation(operation_id, UpdatePhase::Failed, "自动更新器未配置")
                .await;
            return;
        };
        info!(
            operation_id,
            "requesting image update from configured updater"
        );
        let response = self
            .client
            .get(updater_url)
            .bearer_auth(updater_token)
            .timeout(UPDATE_TIMEOUT)
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                self.finish_operation(
                    operation_id,
                    UpdatePhase::Completed,
                    "镜像检查已完成；若存在新镜像，服务将自动切换",
                )
                .await;
            }
            Ok(response) => {
                error!(operation_id, status = %response.status(), "configured updater rejected image update");
                self.finish_operation(
                    operation_id,
                    UpdatePhase::Failed,
                    "更新器拒绝了镜像更新请求，请检查服务日志",
                )
                .await;
            }
            Err(error) => {
                error!(operation_id, error = %error, "configured updater request failed");
                self.finish_operation(
                    operation_id,
                    UpdatePhase::Failed,
                    "无法连接自动更新器，请检查服务日志",
                )
                .await;
            }
        }
    }

    async fn finish_operation(&self, operation_id: &str, phase: UpdatePhase, message: &str) {
        let mut current = self.operation.lock().await;
        if current.operation_id.as_deref() != Some(operation_id) {
            return;
        }
        current.phase = phase;
        current.completed_at = Some(OffsetDateTime::now_utc());
        current.message = Some(message.to_owned());
    }
}

fn workflow_branch(channel: &str) -> Option<&'static str> {
    match channel {
        "prod" => Some("master"),
        _ => None,
    }
}

fn github_status_warning(status: StatusCode, has_token: bool) -> String {
    if status == StatusCode::NOT_FOUND && !has_token {
        return "无法读取私有仓库版本；配置 MEDIAHUB_GITHUB_TOKEN 后可显示最新构建".to_owned();
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return "GitHub Token 无权读取最新构建".to_owned();
    }
    format!("版本服务暂时不可用（HTTP {}）", status.as_u16())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::{Router, http::HeaderMap, routing::get};
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn production_update_channel_maps_to_master() {
        assert_eq!(workflow_branch("prod"), Some("master"));
        assert_eq!(workflow_branch("dev"), None);
    }

    #[test]
    fn status_warning_does_not_expose_response_content() {
        assert!(
            github_status_warning(StatusCode::NOT_FOUND, false).contains("MEDIAHUB_GITHUB_TOKEN")
        );
        assert!(github_status_warning(StatusCode::FORBIDDEN, true).contains("无权"));
    }

    #[tokio::test]
    async fn disabled_and_concurrent_triggers_are_rejected() {
        let disabled = SystemUpdateService::with_endpoints(
            SystemUpdateConfig {
                updater_url: None,
                updater_token: None,
                github_token: None,
            },
            BuildInfo {
                version: "test".to_owned(),
                revision: None,
                channel: "dev".to_owned(),
            },
            "http://127.0.0.1:9".to_owned(),
            Duration::from_secs(60),
        )
        .expect("service");
        assert_eq!(
            disabled.trigger_update().await,
            Err(UpdateTriggerError::Disabled)
        );

        let enabled = SystemUpdateService::with_endpoints(
            SystemUpdateConfig {
                updater_url: Some("http://127.0.0.1:9/v1/update".to_owned()),
                updater_token: Some("0123456789abcdef0123456789abcdef".to_owned()),
                github_token: None,
            },
            BuildInfo {
                version: "test".to_owned(),
                revision: None,
                channel: "dev".to_owned(),
            },
            "http://127.0.0.1:9".to_owned(),
            Duration::from_secs(60),
        )
        .expect("service");
        enabled.trigger_update().await.expect("first trigger");
        assert_eq!(
            enabled.trigger_update().await,
            Err(UpdateTriggerError::InProgress)
        );
    }

    #[tokio::test]
    async fn updater_receives_a_bearer_authenticated_get_request() {
        let calls = Arc::new(AtomicUsize::new(0));
        let handler_calls = Arc::clone(&calls);
        let app = Router::new().route(
            "/v1/update",
            get(move |headers: HeaderMap| {
                let handler_calls = Arc::clone(&handler_calls);
                async move {
                    assert_eq!(
                        headers
                            .get(header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok()),
                        Some("Bearer 0123456789abcdef0123456789abcdef")
                    );
                    handler_calls.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock updater listener");
        let address = listener.local_addr().expect("mock updater address");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock updater server");
        });

        let service = SystemUpdateService::with_endpoints(
            SystemUpdateConfig {
                updater_url: Some(format!("http://{address}/v1/update")),
                updater_token: Some("0123456789abcdef0123456789abcdef".to_owned()),
                github_token: None,
            },
            BuildInfo {
                version: "test".to_owned(),
                revision: None,
                channel: "dev".to_owned(),
            },
            format!("http://{address}"),
            Duration::from_millis(1),
        )
        .expect("service");
        service.trigger_update().await.expect("trigger update");

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if service.operation.lock().await.phase == UpdatePhase::Completed {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("updater call should complete");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
