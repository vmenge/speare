use std::{error, fmt, sync::Arc};

pub type Result<T> = std::result::Result<T, Error>;

type ArcError = Arc<dyn error::Error + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub enum Error {
    MissingResource { type_name: &'static str },
    ResourceTypeMismatch { type_name: &'static str },
    Source { source: ArcError },
}

impl Error {
    pub fn source<E>(error: E) -> Self
    where
        E: error::Error + Send + Sync + 'static,
    {
        Self::Source {
            source: Arc::new(error),
        }
    }

    pub fn downcast_ref<E>(&self) -> Option<&E>
    where
        E: error::Error + Send + Sync + 'static,
    {
        match self {
            Self::Source { source } => source.downcast_ref::<E>(),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingResource { type_name } => write!(f, "{type_name} is not registered"),
            Self::ResourceTypeMismatch { type_name } => {
                write!(f, "{type_name} resource had an unexpected type")
            }
            Self::Source { source } => source.fmt(f),
        }
    }
}

pub trait IntoSystemResult {
    fn into_system_result(self) -> Result<()>;
}

impl IntoSystemResult for () {
    fn into_system_result(self) -> Result<()> {
        Ok(())
    }
}

impl IntoSystemResult for Result<()> {
    fn into_system_result(self) -> Result<()> {
        self
    }
}

impl<E> IntoSystemResult for std::result::Result<(), E>
where
    E: error::Error + Send + Sync + 'static,
{
    fn into_system_result(self) -> Result<()> {
        self.map_err(Error::source)
    }
}

impl<E> From<E> for Error
where
    E: error::Error + Send + Sync + 'static,
{
    fn from(error: E) -> Self {
        Self::source(error)
    }
}
