#![recursion_limit="256"]
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

use async_std::{
    io::{BufReader},
    net::{TcpStream},
    task,
};

#[derive(Debug)]
enum AppErr {
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
    task::block_on(async {

        //email
        let _mail_password = env!("MAIL_PASSWORD"); 
        let _mail_login = env!("MAIL_LOGIN");
        let _imap_server = env!("IMAP_SERVER");
        let _mail_count = env!("MAIL_COUNT");
        let _from_email = env!("FROM_EMAIL");

        //irc
        let _irc_node = env!("IRC_NODE");
        let _irc_user = env!("IRC_USER");
        let _irc_nick = env!("IRC_NICK");

        try_main().await?;
        Ok(())
    })
}

pub fn send_email(chan: String, msg: String) -> Option<u32> {
    let mail_cred = env!("MAIL_PASSWORD");
    let mail_login = env!("MAIL_LOGIN");
    let from_email = env!("FROM_EMAIL");

    let subject = format!("FromIRC {}", chan);
    let from = format!("WeRust <{}>", from_email);
    let reply_to = format!("WeRust <{}>", mail_login);
    let to = mail_login.to_string();

    let email = lettre::Message::builder()
        .from(from.parse().unwrap())
        .reply_to(reply_to.parse().unwrap())
        .to(to.parse().unwrap())
        .subject(subject)
        .body(msg.to_string())
        .unwrap();

    let creds = Credentials::new(mail_login.to_string(), mail_cred.to_string());

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
    let tls = async_native_tls::TlsConnector::new();

    let imap_addr = (env!("IMAP_SERVER"), 993);
    let client = async_imap::connect(imap_addr, env!("IMAP_SERVER"), tls).await?;

    let mut imap_session = client.login(
        env!("MAIL_LOGIN"), env!("MAIL_PASSWORD")).await.map_err(|e| e.0)?;

    imap_session.select("INBOX").await?;

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

async fn try_main() -> Result<(), AppErr> {
    let irc_node = env!("IRC_NODE");
    let irc_stream = TcpStream::connect(irc_node).await?;
    let (reader, mut writer) = (&irc_stream, &irc_stream);
    let reader = BufReader::new(reader);
    let mut lines_from_server = futures::StreamExt::fuse(reader.lines());

    let irc_user = format!("USER {} 0 * :Ronnie Reagan\n", env!("IRC_USER"));
    let irc_nick = format!("NICK {}\n", env!("IRC_NICK"));
    writer.write_all(irc_user.as_bytes()).await?;
    writer.write_all(irc_nick.as_bytes()).await?;

    let mut email_number = env!("MAIL_COUNT").to_string();

    loop {
        let number = email_number.clone();
        let email = retrieve_email(number).await?;

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
                writer.write_all(irc_message.as_bytes()).await?;
            } else {
                println!("Debug: not marked for IRC forwarding");
            }
            let next_email_number = (email_number.parse::<i32>().unwrap() + 1).to_string();
            email_number = next_email_number;
        } else {
            println!("Debug: no new email {}", email_number);
        }

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
