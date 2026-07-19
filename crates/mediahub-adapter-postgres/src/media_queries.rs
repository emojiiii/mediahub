// Media and lifecycle query operations.

impl PostgresRepository {
    pub async fn find_media_by_id(
        &self,
        media_id: MediaId,
    ) -> Result<Option<Media>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM media WHERE id = $1")
            .bind(media_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_media).transpose()
    }

    pub async fn list_media(
        &self,
        application_id: ApplicationId,
        limit: usize,
    ) -> Result<Vec<Media>, RepositoryError> {
        let limit = i64::try_from(limit)
            .map_err(|_| RepositoryError::Invariant("media list limit is too large".into()))?;
        let rows = sqlx::query(
            "SELECT * FROM media WHERE application_id = $1 \
             ORDER BY created_at DESC, id DESC LIMIT $2",
        )
        .bind(application_id.as_uuid())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_media).collect()
    }

    pub async fn list_media_page(
        &self,
        application_id: ApplicationId,
        query: &MediaListQuery,
    ) -> Result<MediaPage, RepositoryError> {
        if query.limit == 0 || query.limit > 100 {
            return Err(RepositoryError::Invariant(
                "media page limit must be between 1 and 100".into(),
            ));
        }
        let mut sql = QueryBuilder::<Postgres>::new("SELECT * FROM media WHERE application_id = ");
        sql.push_bind(application_id.as_uuid());
        if let Some(bucket_id) = query.bucket_id {
            sql.push(" AND bucket_id = ").push_bind(bucket_id.as_uuid());
        }
        if let Some(state) = query.state {
            sql.push(" AND state = ").push_bind(media_state_name(state));
        }
        if let Some(mime) = &query.mime {
            sql.push(" AND content_type = ").push_bind(mime);
        }
        if let Some(created_from) = query.created_from {
            sql.push(" AND created_at >= ")
                .push_bind(postgres_time(created_from));
        }
        if let Some(created_before) = query.created_before {
            sql.push(" AND created_at < ")
                .push_bind(postgres_time(created_before));
        }
        if let Some(prefix) = &query.object_key_prefix {
            sql.push(" AND LEFT(object_key, char_length(")
                .push_bind(prefix)
                .push(")) = ")
                .push_bind(prefix);
        }
        if let Some(cursor) = query.cursor {
            let created_at = postgres_time(cursor.created_at);
            sql.push(" AND (created_at < ")
                .push_bind(created_at)
                .push(" OR (created_at = ")
                .push_bind(created_at)
                .push(" AND id < ")
                .push_bind(cursor.id.as_uuid())
                .push("))");
        }
        let fetch_limit = i64::try_from(query.limit + 1)
            .map_err(|_| RepositoryError::Invariant("media page limit is too large".into()))?;
        sql.push(" ORDER BY created_at DESC, id DESC LIMIT ")
            .push_bind(fetch_limit);
        let mut rows = sql
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(database_error)?;
        let has_more = rows.len() > query.limit;
        rows.truncate(query.limit);
        Ok(MediaPage {
            items: rows
                .into_iter()
                .map(row_to_media)
                .collect::<Result<Vec<_>, _>>()?,
            has_more,
        })
    }

    pub async fn list_media_directory_page(
        &self,
        application_id: ApplicationId,
        query: &MediaDirectoryListQuery,
    ) -> Result<MediaDirectoryPage, RepositoryError> {
        if query.limit == 0 || query.limit > 100 {
            return Err(RepositoryError::Invariant(
                "media directory page limit must be between 1 and 100".into(),
            ));
        }

        let prefix = &query.object_key_prefix;
        let mut sql = QueryBuilder::<Postgres>::new(
            "WITH filtered AS (SELECT media.*, substring(object_key FROM char_length(",
        );
        sql.push_bind(prefix)
            .push(") + 1) AS relative_key FROM media WHERE application_id = ")
            .push_bind(application_id.as_uuid())
            .push(" AND bucket_id = ")
            .push_bind(query.bucket_id.as_uuid());
        if let Some(state) = query.state {
            sql.push(" AND state = ").push_bind(media_state_name(state));
        }
        if let Some(mime) = &query.mime {
            sql.push(" AND content_type = ").push_bind(mime);
        }
        if let Some(created_from) = query.created_from {
            sql.push(" AND created_at >= ")
                .push_bind(postgres_time(created_from));
        }
        if let Some(created_before) = query.created_before {
            sql.push(" AND created_at < ")
                .push_bind(postgres_time(created_before));
        }
        sql.push(" AND LEFT(object_key, char_length(")
            .push_bind(prefix)
            .push(")) = ")
            .push_bind(prefix)
            .push(
                "), entries AS (\
                 SELECT object_key AS entry_key, FALSE AS is_prefix, id AS media_id \
                 FROM filtered WHERE relative_key <> '' AND strpos(relative_key, '/') = 0 \
                 UNION ALL SELECT ",
            )
            .push_bind(prefix)
            .push(
                " || split_part(relative_key, '/', 1) || '/' AS entry_key, \
                 TRUE AS is_prefix, NULL::uuid AS media_id FROM filtered \
                 WHERE relative_key <> '' AND strpos(relative_key, '/') > 0 \
                 GROUP BY split_part(relative_key, '/', 1)\
                 ), paged AS (SELECT entry_key, is_prefix, media_id FROM entries",
            );
        if let Some(cursor) = &query.cursor {
            let cursor_rank = i32::from(!cursor.is_prefix);
            sql.push(" WHERE (CASE WHEN is_prefix THEN 0 ELSE 1 END, entry_key) > (")
                .push_bind(cursor_rank)
                .push(", ")
                .push_bind(&cursor.entry_key)
                .push(")");
        }
        let fetch_limit = i64::try_from(query.limit + 1).map_err(|_| {
            RepositoryError::Invariant("media directory page limit is too large".into())
        })?;
        sql.push(" ORDER BY is_prefix DESC, entry_key ASC LIMIT ")
            .push_bind(fetch_limit)
            .push(
                ") SELECT paged.entry_key, paged.is_prefix, media.* FROM paged \
                 LEFT JOIN media ON media.id = paged.media_id \
                 ORDER BY paged.is_prefix DESC, paged.entry_key ASC",
            );

        let mut rows = sql
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(database_error)?;
        let has_more = rows.len() > query.limit;
        rows.truncate(query.limit);
        let next_cursor = if has_more {
            rows.last()
                .map(|row| {
                    let entry_key = row
                        .try_get::<String, _>("entry_key")
                        .map_err(database_error)?;
                    let is_prefix = row
                        .try_get::<bool, _>("is_prefix")
                        .map_err(database_error)?;
                    Ok(MediaDirectoryListCursor {
                        entry_key,
                        is_prefix,
                    })
                })
                .transpose()?
        } else {
            None
        };
        let mut items = Vec::new();
        let mut common_prefixes = Vec::new();
        for row in rows {
            if row
                .try_get::<bool, _>("is_prefix")
                .map_err(database_error)?
            {
                common_prefixes.push(
                    row.try_get::<String, _>("entry_key")
                        .map_err(database_error)?,
                );
            } else {
                items.push(row_to_media(row)?);
            }
        }
        Ok(MediaDirectoryPage {
            items,
            common_prefixes,
            next_cursor,
        })
    }

    pub async fn list_s3_media_page(
        &self,
        application_id: ApplicationId,
        query: &S3MediaListQuery,
    ) -> Result<S3MediaPage, RepositoryError> {
        if query.limit == 0 || query.limit > 1_000 {
            return Err(RepositoryError::Invariant(
                "S3 media page limit must be between 1 and 1000".into(),
            ));
        }
        let fetch_limit = i64::try_from(query.limit + 1)
            .map_err(|_| RepositoryError::Invariant("S3 media page limit is too large".into()))?;
        let prefix = &query.object_key_prefix;
        if !query.delimiter {
            let mut sql =
                QueryBuilder::<Postgres>::new("SELECT media.* FROM media WHERE application_id = ");
            sql.push_bind(application_id.as_uuid())
                .push(" AND bucket_id = ")
                .push_bind(query.bucket_id.as_uuid())
                .push(" AND state = 'active' AND LEFT(object_key, char_length(")
                .push_bind(prefix)
                .push(")) = ")
                .push_bind(prefix);
            if let Some(start_after) = &query.start_after {
                sql.push(" AND object_key COLLATE \"C\" > ")
                    .push_bind(start_after)
                    .push(" COLLATE \"C\"");
            }
            sql.push(" ORDER BY object_key COLLATE \"C\" ASC LIMIT ")
                .push_bind(fetch_limit);
            let mut rows = sql
                .build()
                .fetch_all(&self.pool)
                .await
                .map_err(database_error)?;
            let has_more = rows.len() > query.limit;
            rows.truncate(query.limit);
            let items = rows
                .into_iter()
                .map(row_to_media)
                .collect::<Result<Vec<_>, _>>()?;
            let next_cursor = has_more
                .then(|| items.last().map(|media| media.object_key().to_owned()))
                .flatten();
            return Ok(S3MediaPage {
                items,
                common_prefixes: Vec::new(),
                next_cursor,
            });
        }

        let mut sql = QueryBuilder::<Postgres>::new(
            "WITH filtered AS (SELECT media.*, substring(object_key FROM char_length(",
        );
        sql.push_bind(prefix)
            .push(") + 1) AS relative_key FROM media WHERE application_id = ")
            .push_bind(application_id.as_uuid())
            .push(" AND bucket_id = ")
            .push_bind(query.bucket_id.as_uuid())
            .push(" AND state = 'active' AND LEFT(object_key, char_length(")
            .push_bind(prefix)
            .push(")) = ")
            .push_bind(prefix)
            .push(
                "), entries AS (\
                 SELECT object_key AS entry_key, FALSE AS is_prefix, id AS media_id \
                 FROM filtered WHERE strpos(relative_key, '/') = 0 \
                 UNION ALL SELECT ",
            )
            .push_bind(prefix)
            .push(
                " || split_part(relative_key, '/', 1) || '/' AS entry_key, \
                 TRUE AS is_prefix, NULL::uuid AS media_id FROM filtered \
                 WHERE strpos(relative_key, '/') > 0 GROUP BY split_part(relative_key, '/', 1)\
                 ), paged AS (SELECT entry_key, is_prefix, media_id FROM entries",
            );
        if let Some(start_after) = &query.start_after {
            sql.push(" WHERE entry_key COLLATE \"C\" > ")
                .push_bind(start_after)
                .push(" COLLATE \"C\"");
        }
        sql.push(" ORDER BY entry_key COLLATE \"C\" ASC LIMIT ")
            .push_bind(fetch_limit)
            .push(
                ") SELECT paged.entry_key, paged.is_prefix, media.* FROM paged \
                 LEFT JOIN media ON media.id = paged.media_id \
                 ORDER BY paged.entry_key COLLATE \"C\" ASC",
            );
        let mut rows = sql
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(database_error)?;
        let has_more = rows.len() > query.limit;
        rows.truncate(query.limit);
        let next_cursor = if has_more {
            rows.last()
                .map(|row| row.try_get::<String, _>("entry_key"))
                .transpose()
                .map_err(database_error)?
        } else {
            None
        };
        let mut items = Vec::new();
        let mut common_prefixes = Vec::new();
        for row in rows {
            if row
                .try_get::<bool, _>("is_prefix")
                .map_err(database_error)?
            {
                common_prefixes.push(
                    row.try_get::<String, _>("entry_key")
                        .map_err(database_error)?,
                );
            } else {
                items.push(row_to_media(row)?);
            }
        }
        Ok(S3MediaPage {
            items,
            common_prefixes,
            next_cursor,
        })
    }

    pub async fn list_expired_media(
        &self,
        now: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<Media>, RepositoryError> {
        let limit = positive_limit(limit, "expired media scan limit must be positive")?;
        let rows = sqlx::query(
            "SELECT * FROM media WHERE state = 'active' AND expires_at IS NOT NULL \
             AND expires_at <= $1 ORDER BY expires_at, id LIMIT $2",
        )
        .bind(postgres_time(now))
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_media).collect()
    }

    pub async fn list_lifecycle_buckets(&self) -> Result<Vec<Bucket>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT * FROM buckets WHERE lifecycle_policy IS NOT NULL \
             AND jsonb_array_length(lifecycle_policy) > 0 ORDER BY application_id, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_bucket).collect()
    }

    pub async fn list_keep_latest_surplus(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
        count: u32,
        limit: usize,
    ) -> Result<Vec<Media>, RepositoryError> {
        if count == 0 {
            return Err(RepositoryError::Invariant(
                "keep_latest count and limit must be positive".into(),
            ));
        }
        let limit = positive_limit(limit, "keep_latest count and limit must be positive")?;
        let rows = sqlx::query(
            "WITH ranked AS ( \
                SELECT media.*, ROW_NUMBER() OVER (ORDER BY created_at DESC, id DESC) AS lifecycle_rank \
                FROM media WHERE application_id = $1 AND bucket_id = $2 AND state = 'active' \
                  AND LEFT(object_key, char_length($3)) = $3 \
             ) SELECT * FROM ranked WHERE lifecycle_rank > $4 \
             ORDER BY created_at, id LIMIT $5",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_id.as_uuid())
        .bind(prefix)
        .bind(i64::from(count))
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_media).collect()
    }

    pub async fn list_expire_after_due(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
        created_before: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<Media>, RepositoryError> {
        let limit = positive_limit(limit, "expire_after scan limit must be positive")?;
        let rows = sqlx::query(
            "SELECT * FROM media WHERE application_id = $1 AND bucket_id = $2 \
             AND state = 'active' AND created_at <= $3 \
             AND LEFT(object_key, char_length($4)) = $4 \
             ORDER BY created_at, id LIMIT $5",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_id.as_uuid())
        .bind(postgres_time(created_before))
        .bind(prefix)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_media).collect()
    }

    pub async fn list_variant_storage_keys(
        &self,
        media_id: MediaId,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT storage_key FROM variants WHERE media_id = $1 ORDER BY created_at, id",
        )
        .bind(media_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter()
            .map(|row| row.try_get("storage_key").map_err(database_error))
            .collect()
    }
}
