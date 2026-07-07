use std::{error, fmt, sync::Arc};

pub type Result<T> = std::result::Result<T, Error>;

type ArcError = Arc<dyn error::Error + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub enum Error {
    MissingResource { type_name: &'static str },
    ResourceTypeMismatch { type_name: &'static str },
    Source { source: ArcError },
    Report { message: String },
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

    pub fn report<E>(error: E) -> Self
    where
        E: fmt::Display + fmt::Debug + Send + Sync + 'static,
    {
        Self::Report {
            message: error.to_string(),
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
            Self::Report { message } => f.write_str(message),
        }
    }
}

pub trait IntoTaskResult {
    type Output;

    fn into_task_result(self) -> Result<Self::Output>;
}

impl IntoTaskResult for () {
    type Output = ();

    fn into_task_result(self) -> Result<Self::Output> {
        Ok(())
    }
}

impl<T, E> IntoTaskResult for std::result::Result<T, E>
where
    E: fmt::Display + fmt::Debug + Send + Sync + 'static,
{
    type Output = T;

    fn into_task_result(self) -> Result<Self::Output> {
        self.map_err(Error::report)
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
