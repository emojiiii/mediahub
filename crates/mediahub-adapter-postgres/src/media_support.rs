// Media repository conversion and invariant helpers.

fn invariant(error: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::Invariant(error.to_string())
}
