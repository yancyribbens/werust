#![recursion_limit="256"]

pub mod error;
pub mod config;
use config::Config;

use futures::select;
use futures::FutureExt;
use irc_proto::command::*;
use regex::Regex;
use irc_proto::message::Message as IrcMessage;

use async_std::prelude::*;
use std::env;

use lettre::{SmtpTransport, Transport};
use lettre::transport::smtp::authentication::Credentials;

use select::document::Document;
use error::AppErr;

use async_std::{
    io::{BufReader},
    net::{TcpStream},
    task,
};

use std::time::Duration;

#[derive(Default, Debug, Clone)]
struct Transformer {
    subject: Option<String>,
    body: Option<String>,
    nick: Option<String>,
    irc_message: Option<String>,
    email: Option<String>,
    irc_recipient: Option<String>,
    send_to_irc: bool
}

// This strict handles the transformation of email communications to IRC
// messages.
impl Transformer {
    fn new(email: Option<String>) -> Transformer {
        let mut t = Transformer::default();
        t.email = email;
        t
    }

    fn send_to_irc(mut self) -> Self {
        if let Some(subject) = self.subject.as_ref() {
            let send_to_irc_r = Regex::new(r"(?m)^SendToIRC.*").unwrap();
            if let Some(_cap) = send_to_irc_r.captures_iter(subject).next() {
                self.send_to_irc = true;
            }
        }

        self
    }

    fn parse_email_body(mut self) -> Self {
        let e = self.email.as_ref().unwrap();
        let regex_body = Regex::new(r"(?ms)^<html.*>.*</html>").unwrap();

        if let Some(html_capture) = regex_body.captures_iter(e).next() {
            let document = Document::from(&html_capture[0]);

            let html = document.nth(0).unwrap()
                .last_child().unwrap()
                .children().next().unwrap()
                .next().unwrap()
                .first_child().unwrap()
                .text();

            self.body = Some(html);
        }
        
        self
    }

    fn parse_email_subject(mut self) -> Self {
        let e = self.email.as_ref().unwrap();
        let mut subject = None;

        let regex_subject = Regex::new(r"(?m)^Subject: [^\r\n]*").unwrap();
        if let Some(email_subject) = regex_subject.find(&e) {
            let subject_split = email_subject.as_str().split(":");
            let subject_vec: Vec<_> = subject_split.collect();
            let subject_body = subject_vec[1];
            subject = Some(subject_body.trim_start().to_string());
        }

        self.subject = subject;

        self
    }

    fn set_recipient(mut self) -> Self {
        let subject: Vec<_> = self.subject.as_ref().unwrap().split(" ").collect();
        self.irc_recipient = Some(subject[1].to_string());

        self
    }

    fn get_recipient(self) -> String {
        self.irc_recipient.unwrap()
    }

    fn get_body(self) -> String {
        self.body.as_ref().unwrap().to_string()
    }
}

pub(crate) fn main() -> Result<(), AppErr> {
    task::spawn(async {
        run_irc().await;
    });

    task::spawn(async {
        run_imap_monitor().await;
    });

    Ok(())
}

pub fn send_email(chan: String, msg: String) -> Option<u32> {
    let config = Config::load_toml();

    let imap_password = env!("IMAP_PASSWORD");
    let imap_login = &config.as_ref().unwrap().imap_login;
    let from_email = &config.as_ref().unwrap().from_email;

    let subject = format!("FromIRC {}", chan);
    let from = format!("WeRust <{}>", from_email);
    let reply_to = format!("WeRust <{}>", imap_login);
    let to = imap_login.to_string();

    let email = lettre::Message::builder()
        .from(from.parse().unwrap())
        .reply_to(reply_to.parse().unwrap())
        .to(to.parse().unwrap())
        .subject(subject)
        .body(msg.to_string())
        .unwrap();

    let creds = Credentials::new(imap_login.to_string(), imap_password.to_string());

    let mailer = SmtpTransport::relay("mail.gandi.net")
        .unwrap()
        .credentials(creds)
        .build();

    match mailer.send(&email) {
        Ok(_) => println!("Email sent successfully!"),
        Err(e) => panic!("Could not send email: {:?}", e),
    }

    None
}

async fn retrieve_email(email_number: String) -> Result<Option<String>, AppErr> {
    let config = Config::load_toml();
    let imap_server = &config.as_ref().unwrap().imap_server;
    let imap_login = &config.as_ref().unwrap().imap_login;
    let imap_inbox = &config.as_ref().unwrap().imap_session;
    let tls = async_native_tls::TlsConnector::new();

    let imap_addr:(&str, u16)  = (&imap_server, 993);
    let client = async_imap::connect(imap_addr, imap_server, tls).await?;

    let mut imap_session = client.login(
        imap_login, env!("IMAP_PASSWORD")).await.map_err(|e| e.0)?;

    imap_session.select(imap_inbox).await?;

    let messages_stream = imap_session.fetch(email_number, "RFC822").await?;
    let messages: Vec<_> = messages_stream.collect::<async_imap::error::Result<_>>().await?;
    let message = if let Some(m) = messages.first() {
        m
    } else {
        return Ok(None);
    };

    let body = message.body().expect("message did not have a body!");
    let body = std::str::from_utf8(body)
        .expect("message was not valid utf-8")
        .to_string();

    Ok(Some(body))
}

async fn run_imap_monitor() -> Result<(), AppErr> {
    let config = Config::load_toml();
    let mut mailbox_count:String = config.as_ref().unwrap().imap_starting_at.clone();

    loop {
        let email = retrieve_email(mailbox_count.to_string()).await?;

        if let Some(email) = email {

            let transformer = Transformer::new(Some(email)).parse_email_subject().send_to_irc();

            if transformer.send_to_irc {
                let transformer = transformer.set_recipient().parse_email_body();

                let body = transformer.clone().get_body();
                let recipient = transformer.clone().get_recipient();

                let irc_command = Command::new(
                    "PRIVMSG", vec![&recipient, &body]
                ).unwrap();

                let irc_message = format!("{}\n", IrcMessage::from(irc_command).to_string());

                //TODO connect threads with ARC
                //writer.write_all(irc_message.as_bytes()).await?;
            } else {
                println!("Debug: not marked for IRC forwarding");
            }
            let increment = mailbox_count.parse::<i32>().unwrap() + 1;
            mailbox_count = increment.clone().to_string();
        } else {
            println!("Debug: no new email {}", mailbox_count);
        }
    }

    Ok(())
}

async fn run_irc() -> Result<(), AppErr> {
    let config = Config::load_toml();
    let irc_server = &config.as_ref().unwrap().irc_server;
    let irc_user = &config.as_ref().unwrap().irc_user;
    let irc_nick = &config.as_ref().unwrap().irc_nick;
    let irc_first_name = &config.as_ref().unwrap().irc_first_name;
    let irc_last_name = &config.as_ref().unwrap().irc_last_name;

    let irc_stream = TcpStream::connect(irc_server).await?;

    let (reader, mut writer) = (&irc_stream, &irc_stream);
    let reader = BufReader::new(reader);
    let mut lines_from_server = futures::StreamExt::fuse(reader.lines());

    let irc_user = format!("USER {} 0 * :{} {}\n", irc_user, irc_first_name, irc_last_name);
    let irc_nick = format!("NICK {}\n", irc_nick);
    writer.write_all(irc_user.as_bytes()).await?;
    writer.write_all(irc_nick.as_bytes()).await?;

    loop {
        select! {
            line = lines_from_server.next().fuse() => match line {
                Some(line) => {
                    let line = line?;
                    let message = line.parse::<IrcMessage>().unwrap();

                    match message.command {
                        Command::PING(ref server, ref _server_two) => {
                            let cmd = Command::new("PONG", vec![server]).unwrap();
                            let irc_message = format!(
                                "{}\n", IrcMessage::from(cmd).to_string());
                            println!("writing pong: {}", irc_message);
                            writer.write_all(irc_message.as_bytes()).await?;
                        },

                        Command::PRIVMSG(ref chan, ref msg) => {
                            send_email(chan.to_string(), msg.to_string());
                        },

                        _ => continue,
                    };

                    println!("{}", line);
                },
                None => break,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    //this email should result in a irc message with the contents: "42"
    fn test_vector_one() -> Option<String> {
        Some(
            "Subject: SendToIRC radical_ed\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alterna\
            tive; \r\n\tboundary=\"----=_Part_6_139340551.1608742910254\"\r\nX-Correlation-ID: <35\
            fcb212-04a8-427f-afca-8ddfc2010606@beebop.lol>\r\n\r\n------=_Part_6_139340551.1608742\
            910254\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 7bit\r\
            \n\r\nHello to beebop\r\n\r\n------=_Part_6_139340551.1608742910254\r\nContent-Type: t\
            ext/html; charset=UTF-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\n<html x-block=\"true\
            \"> \r\n <head x-block=\"true\"></head> \r\n <body x-block=\"true\"> <span style=\"fon\
            t-family:sans-serif\">42</span> \r\n  <br>  \r\n </body>\r\n</html>\r\n------=_Part_6_\
            139340551.1608742910254--\r\n".to_string()
        )
    }

    //contains no <html> tags
    fn test_vector_two() -> Option<String> {
        let email = "Subject: SendToIRC hello\r\nMIME-Version: 1.0\r\nContent-Type: multipart/a\
                     lternative; \r\n\tboundary=\"----=_Part_6_139340551.1608742910254\"\r\nX-C\
                     orrelation-ID: <35fcb212-04a8-427f-afca-8ddfc2010606@beebop.lol>\r\n\r\n--\
                     ----=_Part_6_139340551.1608742910254\r\nContent-Type: text/plain; charset=\
                     UTF-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nHello to beebop\r\n\r\n---\
                     ---=_Part_6_139340551.1608742910254\r\nContent-Type: text/html; charset=UT\
                     F-8\r\n";
        Some(email.to_string())
    }

    //Subject does not contain SendToIRC 
    fn test_vector_three() -> Option<String> {
        let email = "Subject: hello\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative\
                     ; \r\n\tboundary=\"----=_Part_6_139340551.1608742910254\"\r\nX-Correlation\
                     -ID: <35fcb212-04a8-427f-afca-8ddfc2010606@beebop.lol>\r\n\r\n------=_Part\
                     _6_139340551.1608742910254\r\nContent-Type: text/plain; charset=UTF-8\r\nC\
                     ontent-Transfer-Encoding: 7bit\r\n\r\nHello to beebop\r\n\r\n------=_Part_\
                     6_139340551.1608742910254\r\nContent-Type: text/html; charset=UTF-8\r\n";
        Some(email.to_string())
    }

    //Test that colon doesn't break parser
    fn test_vector_four() -> Option<String> {
        let email = "Subject: SendToIRC hello\r\nMIME-Version: 1.0\r\nContent-Type: multipart/a\
                     lternative; \r\n\tboundary=\"----=_Part_6_139340551.1608742910254\"\r\nX-C\
                     orrelation-ID: <35fcb212-04a8-427f-afca-8ddfc2010606@beebop.lol>\r\n\r\n--\
                     ----=_Part_6_139340551.1608742910254\r\nContent-Type: text/plain; charset=\
                     UTF-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nbody :)\r\n\r\n------=_Par\
                     t_6_139340551.1608742910254\r\nContent-Type: text/html; charset=UTF-8\r\nC\
                     ontent-Transfer-Encoding: 7bit\r\n\r\n<html> \r\n <head></head> \r\n <body\
                     > <span style=\"font-family:sans-serif\">body :)</span> \r\n  <br>  \r\n <\
                     /body>\r\n</html>\r\n------=_Part_6_139340551.1608742910254--\r\n";
        Some(email.to_string())
    }

    //html tag variation

    #[test]
    fn test_parse_subject() {
        let expected_subject = Some("SendToIRC radical_ed".to_string());
        let expected_recipient = Some("radical_ed".to_string());

        let transformer = Transformer::new(test_vector_one()).parse_email_subject().set_recipient();
        assert_eq!(expected_subject, transformer.subject);
        assert_eq!(expected_recipient, transformer.irc_recipient);

        let transformer= transformer.send_to_irc();
        assert_eq!(true, transformer.send_to_irc);
    }

    #[test]
    fn test_parse_body() {
        let transformer = Transformer::new(test_vector_one()).parse_email_body();
        let expected_body = Some("42".to_string());
        assert_eq!(expected_body, transformer.body);
    }

    #[test]
    fn test_parse_body_without_html_does_not_halt() {
        let transformer = Transformer::new(test_vector_two()).parse_email_body();
        assert_eq!(None, transformer.body);
    }

    #[test]
    fn test_email_not_destined_for_irc() {
        let transformer = Transformer::new(test_vector_three()).parse_email_body();
        assert_eq!(false, transformer.send_to_irc);
    }

    #[test]
    fn test_can_parse_emoji() {
        let transformer = Transformer::new(test_vector_four()).parse_email_body();
        let expected_body = Some("body :)".to_string());
        assert_eq!(expected_body, transformer.body);
    }
}
