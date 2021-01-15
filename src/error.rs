#[derive(Debug)]
pub(crate) enum AppErr {
    IoError(std::io::Error),
    ImapError(async_imap::error::Error)
}

impl From<async_imap::error::Error> for AppErr {
    fn from(error: async_imap::error::Error) -> Self {
        AppErr::ImapError(error)
    }
}

impl From<std::io::Error> for AppErr {
    fn from(error: std::io::Error) -> Self {
        AppErr::IoError(error)
    }
}
