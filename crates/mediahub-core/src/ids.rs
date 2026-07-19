use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a time-ordered, non-guessable UUIDv7 identity.
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

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self::from_uuid(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.as_uuid()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self::from_uuid)
            }
        }
    };
}

uuid_id!(UserId);
uuid_id!(ApplicationId);
uuid_id!(BucketId);
uuid_id!(MediaId);
uuid_id!(UploadSessionId);
uuid_id!(AccessKeyId);
uuid_id!(VariantId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_round_trip_as_uuid_text() {
        let id = MediaId::new();
        let parsed: MediaId = id.to_string().parse().expect("ID must parse");

        assert_eq!(parsed, id);
        assert_ne!(MediaId::new(), id);
    }

    #[test]
    fn distinct_identity_types_cannot_be_accidentally_compared() {
        let user_id = UserId::new();
        let application_id = ApplicationId::from_uuid(user_id.as_uuid());

        assert_eq!(user_id.as_uuid(), application_id.as_uuid());
    }
}
