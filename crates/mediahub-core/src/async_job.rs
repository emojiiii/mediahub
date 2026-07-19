use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{ApplicationId, MediaId, Visibility};

pub const MAX_ASYNC_JOB_ERROR_BYTES: usize = 2_048;
pub const MAX_ASYNC_JOB_IDEMPOTENCY_KEY_BYTES: usize = 255;
pub const MAX_ASYNC_JOB_OPERATION_SCOPE_BYTES: usize = 128;

include!("async_job_identity.rs");
include!("async_job_item.rs");
include!("async_job_model.rs");
include!("async_job_error.rs");

#[cfg(test)]
mod tests {
    include!("async_job_tests.rs");
}
