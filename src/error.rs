use std::error::Error as StdErr;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub struct Error(Option<Box<dyn StdErr + Send + Sync + 'static>>);

impl Display for Error {
    fn fmt(&self, fmt: &mut Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        fmt.write_str("Audioserve error")?;
        if let Some(e) = self.source() {
            write!(fmt, "\nCause: {}", e)
        } else {
            Ok(())
        }
    }
}

impl StdErr for Error {
    fn cause(&self) -> Option<&dyn StdErr> {
        self.0.as_ref().map(|e| e.as_ref() as &dyn StdErr)
    }
}

impl Error {
    pub fn new_with_cause<E: Into<Box<dyn StdErr + Send + Sync + 'static>>>(cause: E) -> Self {
        Error(Some(cause.into()))
    }
}
