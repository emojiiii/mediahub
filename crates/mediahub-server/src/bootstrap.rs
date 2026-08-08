// Application startup and dependency wiring.

pub(super) async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            env::var("RUST_LOG").unwrap_or_else(|_| "mediahub_server=info,tower_http=info".into()),
        )
        .json()
        .init();

    let config = ServerConfig::from_env().map_err(anyhow::Error::msg)?;
    let mut access_key_keyring = config.access_key_master_keyring;
    access_key_keyring.push((
        config.access_key_master_key_version,
        config.access_key_master_key,
    ));
    let access_key_cipher = AccessKeyCipher::from_keyring(
        config.access_key_master_key_version,
        access_key_keyring
            .iter()
            .map(|(version, key)| (*version, key.as_str())),
    )
    .map_err(|error| anyhow::Error::msg(error.to_string()))?;
    let object_store = match config.storage_backend {
        StorageBackend::Local => RuntimeObjectStore::local(
            LocalObjectStore::new(&config.storage_root)
                .map_err(|error| anyhow::Error::msg(error.to_string()))?,
        ),
        StorageBackend::S3 => RuntimeObjectStore::s3(
            config
                .s3_config
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("S3 configuration is missing"))?
                .build()
                .map_err(|error| anyhow::Error::msg(error.to_string()))?,
        ),
    };
    let postgres = PostgresRepository::connect(&config.database_url)
        .await
        .map_err(|error| anyhow::Error::msg(error.to_string()))?;
    postgres
        .migrate()
        .await
        .map_err(|error| anyhow::Error::msg(error.to_string()))?;
    let repository = postgres;
    info!(
        database_backend = "postgres",
        storage_backend = object_store.backend_name(),
        "database repository initialized"
    );
    super::workers::validate_referenced_key_versions(&repository, &access_key_cipher).await?;
    super::workers::validate_storage_database_consistency(&repository, &object_store)
        .await
        .map_err(anyhow::Error::msg)?;
    if let Some(email) = &config.bootstrap_admin_email {
        match repository
            .bootstrap_admin(email, OffsetDateTime::now_utc())
            .await
        {
            Ok(AdminBootstrapOutcome::Completed(user_id)) => {
                info!(%user_id, "initial system administrator bootstrapped; remove MEDIAHUB_BOOTSTRAP_ADMIN_EMAIL before restarting");
            }
            Ok(AdminBootstrapOutcome::AlreadyCompleted) => {
                return Err(anyhow::anyhow!(
                    "admin bootstrap already completed; remove MEDIAHUB_BOOTSTRAP_ADMIN_EMAIL"
                ));
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "admin bootstrap failed closed for {email}: {error}"
                ));
            }
        }
    }
    let access_key_cipher = Arc::new(access_key_cipher);
    let email_provider = config.resend.map(ResendEmailProvider::new).map(Arc::new);
    let system_update = SystemUpdateService::new(config.system_update)
        .map_err(|error| anyhow::anyhow!("failed to initialize system update service: {error}"))?;
    let webdav = webdav::WebDavService::new(
        repository.clone(),
        object_store.clone(),
        Arc::clone(&access_key_cipher),
    );
    let control_bind_addr = config.bind_addr;
    let s3_bind_addr = config.s3_bind_addr;
    let state = Arc::new(AppState {
        repository: repository.clone(),
        object_store: object_store.clone(),
        s3_gc_grace: time::Duration::hours(config.s3_gc_grace_hours),
        webdav,
        access_key_cipher: Arc::clone(&access_key_cipher),
        media_url_signer: Arc::new(MediaUrlSigner::new(config.media_url_signing_key)),
        cookie_config: config.cookie_config,
        cors_allowed_origins: config.cors_allowed_origins,
        registration_enabled: config.registration_enabled,
        expose_auth_tokens: config.expose_auth_tokens,
        email_provider,
        auth_rate_limiter: AuthRateLimiter::default(),
        variant_slots: Arc::new(tokio::sync::Semaphore::new(4)),
        http_metrics: HttpMetrics::default(),
        metrics_bearer_token: config.metrics_bearer_token.map(Arc::from),
        system_update,
    });
    let app = control_plane_router(Arc::clone(&state), config.web_root);
    let s3_app = s3_router::router(Arc::clone(&state));

    // Bind both sockets before starting background workers. Startup is atomic from
    // the process perspective: either both listeners are available or run fails.
    let control_listener = TcpListener::bind(control_bind_addr)
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to bind control-plane listener at {control_bind_addr}: {error}"
            )
        })?;
    let s3_listener = TcpListener::bind(s3_bind_addr).await.map_err(|error| {
        anyhow::anyhow!("failed to bind S3 listener at {s3_bind_addr}: {error}")
    })?;
    info!(address = %control_bind_addr, "control-plane listener ready");
    info!(address = %s3_bind_addr, "S3 listener ready");

    let mut lifecycle_worker = tokio::spawn(super::workers::run_lifecycle_worker(
        repository.clone(),
        object_store.clone(),
    ));
    let mut s3_storage_cleanup_worker = tokio::spawn(
        super::workers::run_s3_storage_cleanup_worker(repository.clone(), object_store),
    );
    let mut outbox_worker = tokio::spawn(super::workers::run_outbox_worker(
        repository.clone(),
        access_key_cipher,
    ));
    let mut async_job_worker = tokio::spawn(super::workers::run_async_job_worker(repository));

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let control_server = std::future::IntoFuture::into_future(
        axum::serve(
            control_listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(wait_for_shutdown(shutdown_rx.clone())),
    );
    let s3_server = std::future::IntoFuture::into_future(
        axum::serve(
            s3_listener,
            s3_app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(wait_for_shutdown(shutdown_rx)),
    );
    tokio::pin!(control_server);
    tokio::pin!(s3_server);

    enum RuntimeExit {
        Shutdown(anyhow::Result<()>),
        Control(std::io::Result<()>),
        S3(std::io::Result<()>),
        Worker(anyhow::Error),
    }

    let exit = tokio::select! {
        result = shutdown_signal() => RuntimeExit::Shutdown(result),
        result = &mut control_server => RuntimeExit::Control(result),
        result = &mut s3_server => RuntimeExit::S3(result),
        result = &mut lifecycle_worker => RuntimeExit::Worker(anyhow::anyhow!(
            "lifecycle worker exited unexpectedly: {result:?}"
        )),
        result = &mut s3_storage_cleanup_worker => RuntimeExit::Worker(anyhow::anyhow!(
            "S3 storage cleanup worker exited unexpectedly: {result:?}"
        )),
        result = &mut outbox_worker => RuntimeExit::Worker(anyhow::anyhow!(
            "outbox worker exited unexpectedly: {result:?}"
        )),
        result = &mut async_job_worker => RuntimeExit::Worker(anyhow::anyhow!(
            "async job worker exited unexpectedly: {result:?}"
        )),
    };

    let _ = shutdown_tx.send(true);
    lifecycle_worker.abort();
    s3_storage_cleanup_worker.abort();
    outbox_worker.abort();
    async_job_worker.abort();

    match exit {
        RuntimeExit::Shutdown(trigger) => {
            let (control_result, s3_result) =
                tokio::join!(&mut control_server, &mut s3_server);
            shutdown_outcome(trigger, control_result, s3_result)
        }
        RuntimeExit::Control(control_result) => {
            let s3_result = (&mut s3_server).await;
            unexpected_listener_outcome("control-plane", control_result, "S3", s3_result)
        }
        RuntimeExit::S3(s3_result) => {
            let control_result = (&mut control_server).await;
            unexpected_listener_outcome("S3", s3_result, "control-plane", control_result)
        }
        RuntimeExit::Worker(error) => {
            let (control_result, s3_result) =
                tokio::join!(&mut control_server, &mut s3_server);
            worker_failure_outcome(error, control_result, s3_result)
        }
    }
}

async fn wait_for_shutdown(mut shutdown: tokio::sync::watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    while shutdown.changed().await.is_ok() {
        if *shutdown.borrow() {
            return;
        }
    }
}

async fn shutdown_signal() -> anyhow::Result<()> {
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| anyhow::anyhow!("failed to listen for shutdown signal: {error}"))?;
    info!("shutdown signal received");
    Ok(())
}

fn shutdown_outcome(
    trigger: anyhow::Result<()>,
    control_result: std::io::Result<()>,
    s3_result: std::io::Result<()>,
) -> anyhow::Result<()> {
    trigger?;
    control_result.map_err(|error| {
        anyhow::anyhow!("control-plane listener failed during shutdown: {error}")
    })?;
    s3_result
        .map_err(|error| anyhow::anyhow!("S3 listener failed during shutdown: {error}"))?;
    Ok(())
}

fn unexpected_listener_outcome(
    exited_name: &str,
    exited_result: std::io::Result<()>,
    peer_name: &str,
    peer_result: std::io::Result<()>,
) -> anyhow::Result<()> {
    let exited = match exited_result {
        Ok(()) => format!("{exited_name} listener exited unexpectedly"),
        Err(error) => format!("{exited_name} listener failed: {error}"),
    };
    match peer_result {
        Ok(()) => Err(anyhow::anyhow!(exited)),
        Err(error) => Err(anyhow::anyhow!(
            "{exited}; {peer_name} listener failed during coordinated shutdown: {error}"
        )),
    }
}

fn worker_failure_outcome(
    worker_error: anyhow::Error,
    control_result: std::io::Result<()>,
    s3_result: std::io::Result<()>,
) -> anyhow::Result<()> {
    match (control_result, s3_result) {
        (Ok(()), Ok(())) => Err(worker_error),
        (Err(control_error), Ok(())) => Err(anyhow::anyhow!(
            "{worker_error}; control-plane listener failed during shutdown: {control_error}"
        )),
        (Ok(()), Err(s3_error)) => Err(anyhow::anyhow!(
            "{worker_error}; S3 listener failed during shutdown: {s3_error}"
        )),
        (Err(control_error), Err(s3_error)) => Err(anyhow::anyhow!(
            "{worker_error}; control-plane listener failed during shutdown: {control_error}; S3 listener failed during shutdown: {s3_error}"
        )),
    }
}
