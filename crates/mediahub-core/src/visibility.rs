use serde::{Deserialize, Serialize};

/// Access policy inherited by media unless it has an explicit override.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    #[default]
    Private,
}
