// Async-job item result model.

/// Serializable result for one explicit media target in a batch job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsyncJobItemResult {
    pub job_id: AsyncJobId,
    pub application_id: ApplicationId,
    pub media_id: MediaId,
    pub ordinal: u32,
    pub state: AsyncJobItemState,
    pub attempt_count: u32,
    pub result: Option<Value>,
    pub error_code: Option<String>,
    pub error_summary: Option<String>,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
    pub updated_at: OffsetDateTime,
}

impl AsyncJobItemResult {
    #[must_use]
    pub fn pending(
        job_id: AsyncJobId,
        application_id: ApplicationId,
        media_id: MediaId,
        ordinal: u32,
        now: OffsetDateTime,
    ) -> Self {
        Self {
            job_id,
            application_id,
            media_id,
            ordinal,
            state: AsyncJobItemState::Pending,
            attempt_count: 0,
            result: None,
            error_code: None,
            error_summary: None,
            started_at: None,
            completed_at: None,
            updated_at: now,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn succeeded(
        job_id: AsyncJobId,
        application_id: ApplicationId,
        media_id: MediaId,
        ordinal: u32,
        attempt_count: u32,
        result: Option<Value>,
        started_at: OffsetDateTime,
        completed_at: OffsetDateTime,
    ) -> AsyncJobResult<Self> {
        validate_item_attempt(attempt_count)?;
        validate_item_times(started_at, completed_at)?;
        Ok(Self {
            job_id,
            application_id,
            media_id,
            ordinal,
            state: AsyncJobItemState::Succeeded,
            attempt_count,
            result,
            error_code: None,
            error_summary: None,
            started_at: Some(started_at),
            completed_at: Some(completed_at),
            updated_at: completed_at,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn failed(
        job_id: AsyncJobId,
        application_id: ApplicationId,
        media_id: MediaId,
        ordinal: u32,
        attempt_count: u32,
        error_code: impl Into<String>,
        error_summary: impl Into<String>,
        started_at: OffsetDateTime,
        completed_at: OffsetDateTime,
    ) -> AsyncJobResult<Self> {
        validate_item_attempt(attempt_count)?;
        validate_item_times(started_at, completed_at)?;
        let error_code = error_code.into();
        let error_summary = error_summary.into();
        validate_error_code(&error_code)?;
        validate_error_summary(&error_summary)?;
        Ok(Self {
            job_id,
            application_id,
            media_id,
            ordinal,
            state: AsyncJobItemState::Failed,
            attempt_count,
            result: None,
            error_code: Some(error_code),
            error_summary: Some(error_summary),
            started_at: Some(started_at),
            completed_at: Some(completed_at),
            updated_at: completed_at,
        })
    }
}

