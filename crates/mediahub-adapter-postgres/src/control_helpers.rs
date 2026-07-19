// Control-plane row conversion and validation helpers.

async fn consume_one_time_token(
    transaction: &mut Transaction<'_, Postgres>,
    token_hash: &str,
    purpose: OneTimeTokenPurpose,
    now: OffsetDateTime,
) -> Result<Option<Uuid>, RepositoryError> {
    let row = sqlx::query(
        "UPDATE one_time_tokens SET consumed_at = $1 \
         WHERE token_hash = $2 AND purpose = $3 AND consumed_at IS NULL AND expires_at > $1 \
         RETURNING user_id",
    )
    .bind(now)
    .bind(token_hash)
    .bind(purpose.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?;
    row.map(|row| row.try_get("user_id").map_err(database_error))
        .transpose()
}

async fn find_application(
    pool: &sqlx::PgPool,
    query: &str,
    user_id: Uuid,
) -> Result<Option<ApplicationSummary>, RepositoryError> {
    let row = sqlx::query(query)
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(database_error)?;
    row.map(row_to_application).transpose()
}

fn row_to_user(row: PgRow) -> Result<UserAccount, RepositoryError> {
    Ok(UserAccount {
        id: UserId::from_uuid(row.try_get("id").map_err(database_error)?),
        email_normalized: row.try_get("email_normalized").map_err(database_error)?,
        password_hash: row.try_get("password_hash").map_err(database_error)?,
        email_verified_at: row.try_get("email_verified_at").map_err(database_error)?,
        status: row.try_get("status").map_err(database_error)?,
        system_role: row.try_get("system_role").map_err(database_error)?,
        last_login_at: row.try_get("last_login_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_session(row: PgRow, current_token_hash: &str) -> Result<SessionRecord, RepositoryError> {
    let token_hash = row
        .try_get::<String, _>("token_hash")
        .map_err(database_error)?;
    Ok(SessionRecord {
        id: row
            .try_get::<Uuid, _>("id")
            .map_err(database_error)?
            .to_string(),
        expires_at: row.try_get("expires_at").map_err(database_error)?,
        last_seen_at: row.try_get("last_seen_at").map_err(database_error)?,
        created_ip: row.try_get("created_ip").map_err(database_error)?,
        last_seen_ip: row.try_get("last_seen_ip").map_err(database_error)?,
        user_agent_summary: row.try_get("user_agent_summary").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        is_current: token_hash == current_token_hash,
    })
}

fn row_to_application(row: PgRow) -> Result<ApplicationSummary, RepositoryError> {
    Ok(ApplicationSummary {
        id: ApplicationId::from_uuid(row.try_get("id").map_err(database_error)?),
        name: row.try_get("name").map_err(database_error)?,
        app_id: row.try_get("app_id").map_err(database_error)?,
        quota: QuotaSnapshot {
            quota_bytes: crate::codec::as_u64(row.try_get("quota_bytes").map_err(database_error)?)?,
            used_bytes: crate::codec::as_u64(row.try_get("used_bytes").map_err(database_error)?)?,
            reserved_bytes: crate::codec::as_u64(
                row.try_get("reserved_bytes").map_err(database_error)?,
            )?,
        },
    })
}

fn row_to_access_key(row: PgRow) -> Result<AccessKeyRecord, RepositoryError> {
    Ok(AccessKeyRecord {
        id: row
            .try_get::<Uuid, _>("id")
            .map_err(database_error)?
            .to_string(),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        access_key_id: row.try_get("access_key_id").map_err(database_error)?,
        secret_ciphertext: row.try_get("secret_ciphertext").map_err(database_error)?,
        secret_key_version: as_u32(row.try_get("secret_key_version").map_err(database_error)?)?,
        secret_last_four: row.try_get("secret_last_four").map_err(database_error)?,
        name: row.try_get("name").map_err(database_error)?,
        permissions: row
            .try_get::<Json<Vec<String>>, _>("permissions")
            .map_err(database_error)?
            .0,
        expires_at: row.try_get("expires_at").map_err(database_error)?,
        revoked_at: row.try_get("revoked_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
    })
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, RepositoryError> {
    Uuid::parse_str(value).map_err(|_| RepositoryError::Invariant(format!("{field} is not a UUID")))
}

fn as_i32(value: u32, field: &str) -> Result<i32, RepositoryError> {
    i32::try_from(value)
        .map_err(|_| RepositoryError::Invariant(format!("{field} exceeds PostgreSQL INTEGER")))
}

fn affected_one(rows: u64) -> Result<(), RepositoryError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(RepositoryError::NotFound)
    }
}

fn control_write_error(error: sqlx::Error) -> RepositoryError {
    if error
        .as_database_error()
        .is_some_and(|database| matches!(database.code().as_deref(), Some("23503" | "23505")))
    {
        RepositoryError::Conflict
    } else {
        database_error(error)
    }
}
