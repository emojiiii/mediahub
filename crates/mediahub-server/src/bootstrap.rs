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
    let email_provider = config
        .email_provider
        .map(HttpEmailProvider::new)
        .map(Arc::new);
    let webdav = webdav::WebDavService::new(
        repository.clone(),
        object_store.clone(),
        Arc::clone(&access_key_cipher),
    );
    let app = router(AppState {
        repository: repository.clone(),
        object_store: object_store.clone(),
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
    });
    tokio::spawn(super::workers::run_lifecycle_worker(repository.clone(), object_store));
    tokio::spawn(super::workers::run_outbox_worker(repository.clone(), access_key_cipher));
    tokio::spawn(super::workers::run_async_job_worker(repository));

    let listener = TcpListener::bind(config.bind_addr).await?;
    info!(address = %config.bind_addr, "MediaHub server listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

