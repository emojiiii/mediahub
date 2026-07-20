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
pub const MAX_ASYNC_JOB_ITEMS: u32 = 1_000;
pub const MAX_ASYNC_JOB_ATTEMPTS: u32 = 100;
pub const MAX_ASYNC_JOB_LEASE_SECONDS: i64 = 3_600;
pub const MAX_ASYNC_JOB_REQUEST_ID_BYTES: usize = 255;

include!("async_job_identity.rs");
include!("async_job_item.rs");
include!("async_job_model.rs");
include!("async_job_error.rs");

#[cfg(test)]
mod tests {
    include!("async_job_tests.rs");
}
