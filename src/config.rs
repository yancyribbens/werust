use toml;

use crate::error::AppErr;
use serde_derive::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Config {
    pub(crate) from_email: String,
    pub(crate) imap_login: String,
    pub(crate) imap_server: String,
    pub(crate) imap_session: String,
    pub(crate) imap_starting_at: String,
    pub(crate) irc_server: String,
    pub(crate) irc_user: String,
    pub(crate) irc_nick: String,
    pub(crate) irc_first_name: String,
    pub(crate) irc_last_name: String
}

impl Config {
    pub(crate) fn load_toml() -> Result<Config, AppErr> {
        const DATA: &str = include_str!("config.toml");
        let config: Config = toml::from_str(DATA).unwrap();
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config() {
        let config = Config::load_toml();

        assert_eq!("radical_ed@beebop.org", config.as_ref().unwrap().from_email);
        assert_eq!("bogusdata", config.as_ref().unwrap().imap_login);
        assert_eq!("beebop", config.as_ref().unwrap().imap_server);
        assert_eq!("utopiaplanitia.net:6667", config.as_ref().unwrap().irc_server);
        assert_eq!("radical_ed", config.as_ref().unwrap().irc_user);
        assert_eq!("radical_ed", config.as_ref().unwrap().irc_nick);
        assert_eq!("radical", config.as_ref().unwrap().irc_first_name);
        assert_eq!("ed", config.as_ref().unwrap().irc_last_name);
        assert_eq!("inbox", config.as_ref().unwrap().imap_session);
    }
}
