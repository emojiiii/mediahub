// Async-job identifiers, states, and actions.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AsyncJobId(Uuid);

impl AsyncJobId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for AsyncJobId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AsyncJobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for AsyncJobId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self::from_uuid)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsyncJobState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl AsyncJobState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AsyncJobAction {
    UpdateTtlSeconds { ttl_seconds: Option<u64> },
    UpdateVisibility { visibility: Visibility },
    Delete,
}

impl AsyncJobAction {
    fn validate(&self) -> AsyncJobResult<()> {
        if matches!(
            self,
            Self::UpdateTtlSeconds {
                ttl_seconds: Some(0)
            }
        ) {
            return Err(AsyncJobError::InvalidTtlSeconds);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsyncJobItemState {
    Pending,
    Succeeded,
    Failed,
    Cancelled,
}

impl AsyncJobItemState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

