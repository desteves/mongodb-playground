use mongodb::error::ErrorKind;

#[derive(Debug, thiserror::Error)]
pub enum PlaygroundError {
    /// The request is malformed or exceeds a limit.
    #[error("{0}")]
    Invalid(String),
    /// The request uses an operator or stage that is not allowed.
    #[error("{0}")]
    Forbidden(String),
    /// The run exceeded the wall-clock limit.
    #[error("the run took longer than {0} ms")]
    Timeout(u64),
    /// MongoDB rejected the command (bad operator, bad syntax, maxTimeMS, ...). Safe to show.
    #[error("{0}")]
    Database(String),
    /// Could not reach MongoDB. Details are logged, not returned.
    #[error("the database is temporarily unavailable")]
    Unavailable,
    /// Too many search-index runs at once.
    #[error("too many search runs are in progress")]
    Busy,
    #[error("{0}")]
    NotFound(String),
    /// The service cannot take more of this kind of content right now (for example, share storage is full).
    #[error("{0}")]
    Full(String),
    /// Too many requests from one client; the number is seconds until a retry can succeed.
    #[error("rate limit reached; retry in {0} s")]
    RateLimited(u64),
}

impl PlaygroundError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_request",
            Self::Forbidden(_) => "forbidden",
            Self::Timeout(_) => "timeout",
            Self::Database(_) => "database_error",
            Self::Unavailable => "unavailable",
            Self::Busy => "busy",
            Self::NotFound(_) => "not_found",
            Self::Full(_) => "capacity",
            Self::RateLimited(_) => "rate_limited",
        }
    }
}

impl PlaygroundError {
    /// A short, actionable next step for the caller. Written for agents as much as humans.
    pub fn hint(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "Check the request shape against GET /openapi.json, or copy one from GET /api/v1/examples.",
            Self::Forbidden(_) => "Server-side JavaScript, $out/$merge and cross-database references are blocked. Rewrite with standard operators (for example $expr instead of $where).",
            Self::Timeout(_) => "Use fewer documents or a simpler pipeline. Limits are at GET /api/v1/limits.",
            Self::Database(_) => "MongoDB rejected the command. Fix the operator or syntax named in the message.",
            Self::Unavailable => "Retry in a few seconds.",
            Self::Busy => "Runs that build a search index are limited; retry in a few seconds.",
            Self::RateLimited(_) => "Wait for the Retry-After header's number of seconds. People using the web app get a higher limit after the browser verification; agents should back off and retry.",
            Self::Full(_) => "Try again later, or share the playground another way: Export gives a script you can keep.",
            Self::NotFound(_) => "Check the id. Shared playgrounds expire after 90 days without a view.",
        }
    }
}

impl From<mongodb::error::Error> for PlaygroundError {
    fn from(err: mongodb::error::Error) -> Self {
        match &*err.kind {
            ErrorKind::Command(ce) => Self::Database(format!(
                "{} (code {}: {})",
                ce.message, ce.code, ce.code_name
            )),
            _ => {
                tracing::error!(error = %err, "mongodb error");
                Self::Unavailable
            }
        }
    }
}
