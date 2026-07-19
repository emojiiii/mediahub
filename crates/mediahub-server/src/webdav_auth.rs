// WebDAV request authentication.

impl WebDavService {
    pub(crate) fn new(
        repository: PostgresRepository,
        object_store: RuntimeObjectStore,
        access_key_cipher: Arc<mediahub_server::access_key::AccessKeyCipher>,
    ) -> Self {
        let filesystem = MediaHubDavFs {
            repository: repository.clone(),
            object_store,
        };
        let handler = DavHandler::builder()
            .filesystem(Box::new(filesystem))
            .locksystem(FakeLs::new())
            .strip_prefix("/dav")
            .autoindex(false)
            .build_handler();
        Self {
            handler,
            repository,
            access_key_cipher,
        }
    }

    async fn handle(&self, request: Request) -> Response {
        let auth_input = match DavAuthInput::from_request(&request) {
            Ok(input) => input,
            Err(()) => return unauthorized_response(),
        };
        let credentials = match self.authenticate(auth_input).await {
            Ok(credentials) => credentials,
            Err(()) => return unauthorized_response(),
        };
        let principal = format!("/principals/access-keys/{}", credentials.access_key_id);
        self.handler
            .handle_guarded(request, principal, credentials)
            .await
            .map(Body::new)
    }

    async fn authenticate(&self, input: DavAuthInput) -> Result<DavCredentials, ()> {
        let now = OffsetDateTime::now_utc();
        let access_key = self
            .repository
            .find_active_access_key(&input.access_key_id, now)
            .await
            .map_err(|_| ())?
            .ok_or(())?;
        let expected_secret = self
            .access_key_cipher
            .decrypt(&access_key.secret_ciphertext, access_key.secret_key_version)
            .map_err(|_| ())?;
        if !bool::from(expected_secret.ct_eq(input.secret.as_bytes())) {
            return Err(());
        }
        let application = self
            .repository
            .find_application_by_id(access_key.application_id)
            .await
            .map_err(|_| ())?
            .ok_or(())?;
        Ok(DavCredentials {
            application,
            access_key_id: access_key.access_key_id,
            permissions: Arc::from(access_key.permissions),
            method: input.method,
            content_type: input.content_type,
            request_id: input.request_id,
        })
    }
}

struct DavAuthInput {
    access_key_id: String,
    secret: String,
    method: axum::http::Method,
    content_type: Option<String>,
    request_id: String,
}

impl DavAuthInput {
    fn from_request(request: &Request) -> Result<Self, ()> {
        let encoded = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Basic "))
            .ok_or(())?;
        let decoded = STANDARD.decode(encoded).map_err(|_| ())?;
        let decoded = std::str::from_utf8(&decoded).map_err(|_| ())?;
        let (access_key_id, secret) = decoded.split_once(':').ok_or(())?;
        if access_key_id.is_empty() || secret.is_empty() {
            return Err(());
        }
        let content_type = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        Ok(Self {
            access_key_id: access_key_id.to_owned(),
            secret: secret.to_owned(),
            method: request.method().clone(),
            content_type,
            request_id: request.extensions().get::<RequestId>().map_or_else(
                || format!("dav_{}", uuid::Uuid::now_v7().simple()),
                |request_id| request_id.0.clone(),
            ),
        })
    }
}

#[axum::debug_handler]
pub(crate) async fn handle_webdav(
    State(state): State<Arc<AppState>>,
    request: Request,
) -> Response {
    state.webdav.handle(request).await
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, AUTH_CHALLENGE)],
    )
        .into_response()
}

#[derive(Clone, Debug)]
struct DavCredentials {
    application: ApplicationSummary,
    access_key_id: String,
    permissions: Arc<[String]>,
    method: axum::http::Method,
    content_type: Option<String>,
    request_id: String,
}

impl DavCredentials {
    fn allows(&self, permission: &str) -> bool {
        self.permissions.iter().any(|value| value == permission)
    }

    fn require(&self, permission: &str) -> FsResult<()> {
        self.allows(permission)
            .then_some(())
            .ok_or(FsError::Forbidden)
    }

    fn require_any(&self, permissions: &[&str]) -> FsResult<()> {
        permissions
            .iter()
            .any(|permission| self.allows(permission))
            .then_some(())
            .ok_or(FsError::Forbidden)
    }
}

