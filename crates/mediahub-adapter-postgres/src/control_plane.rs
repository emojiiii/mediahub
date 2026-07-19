// PostgreSQL control-plane repository wiring.

use async_trait::async_trait;
use mediahub_app::{
    AccessKeyRecord, AccessKeyRepository, ApplicationRepository, ApplicationSummary,
    AuthRepository, NewAccessKey, OneTimeTokenPurpose, QuotaSnapshot, RepositoryError,
    SessionRecord, UserAccount,
};
use mediahub_core::{ApplicationId, OffsetDateTime, UserId};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow, types::Json};
use uuid::Uuid;

use crate::{
    PostgresRepository,
    codec::{as_i64, as_u32, database_error, postgres_time},
};

include!("control_auth.rs");
include!("control_applications.rs");
include!("control_access_keys.rs");
include!("control_helpers.rs");
