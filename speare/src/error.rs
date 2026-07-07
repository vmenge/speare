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

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Source { source } => Some(source.as_ref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum RunError<E> {
    Task(E),
    App(Error),
}

impl<E> fmt::Display for RunError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Task(error) => error.fmt(f),
            Self::App(error) => error.fmt(f),
        }
    }
}

impl<E> error::Error for RunError<E>
where
    E: error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Task(error) => Some(error),
            Self::App(error) => Some(error),
        }
    }
}

impl<E> From<RunError<E>> for Error
where
    E: fmt::Display + fmt::Debug + Send + Sync + 'static,
{
    fn from(error: RunError<E>) -> Self {
        match error {
            RunError::Task(error) => Self::report(error),
            RunError::App(error) => error,
        }
    }
}

pub trait IntoTaskResult {
    type Output;
    type Error;

    fn into_task_result(self) -> std::result::Result<Self::Output, Self::Error>;
}

impl IntoTaskResult for () {
    type Output = ();
    type Error = std::convert::Infallible;

    fn into_task_result(self) -> std::result::Result<Self::Output, Self::Error> {
        Ok(())
    }
}

impl<T, E> IntoTaskResult for std::result::Result<T, E> {
    type Output = T;
    type Error = E;

    fn into_task_result(self) -> std::result::Result<Self::Output, Self::Error> {
        self
    }
}
