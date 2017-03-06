extern crate futures;
extern crate tokio_core;
extern crate tokio_proto;
extern crate tokio_service;
extern crate encoding;
extern crate uuid;


use std::io;
use std::fs::File;
use std::fs;
use std::io::{Write, Seek, SeekFrom};
use tokio_core::io::{Codec, EasyBuf};
use encoding::{Encoding, DecoderTrap, EncoderTrap};
use tokio_core::io::{Framed, Io};
use tokio_proto::pipeline::ServerProto;
use encoding::all::ASCII;
use futures::{IntoFuture, Future, Sink, Stream};
use time;
use uuid::Uuid;

use config;

pub struct ASCIILineBased;

impl Codec for ASCIILineBased {
  type In = String;
  type Out = String;

  // Read a line from the wire
  //
  fn decode(&mut self, buf: &mut EasyBuf) -> Result<Option<String>, io::Error> {
    // read lines...
   if let Some(i) = buf.as_slice().iter().position(|&b| b == b'\n' || b == b'\r') {
        // remove the serialized frame from the buffer.
        let line = buf.drain_to(i);

        // Also remove the '\n' / '\r'
        let crlf_buf = buf.drain_to(1);
        let crlf = crlf_buf.as_slice();

        if buf.len() > 0 {
            // check if next char is a \n or \r
            if crlf[0] == b'\r' && buf.as_slice()[0] == b'\n' {
               buf.drain_to(1);
            }
            if crlf[0] == b'\n' && buf.as_slice()[0] == b'\r' {
               buf.drain_to(1);
            }
        }

        // we have a line and can return it
        return match ASCII.decode(line.as_slice(), DecoderTrap::Ignore) {
          Ok(str) => Ok(Some(str)),
          Err(_) => Err(io::Error::new(io::ErrorKind::Other, "invalid string")),
        }
    }
    Ok(None)
  }

  // Encode a line over the wire
  //
  fn encode (&mut self, data: String, buf: &mut Vec<u8>) -> io::Result<()> {
    match ASCII.encode(&data, EncoderTrap::Ignore) {
      Ok(bytes) => buf.extend(bytes.iter()),
      Err(_) => ()
    }
    buf.extend([b'\r', b'\n'].iter());
    Ok(())
  }
}

//
// Struct to hold data about email being consumed
// mailData is not suppossed to hold full email, strategy is:
//  mailData holds up to "Message-Id" header, then mailFile
//  is created and all mailData is written there.
// When mailFile is established then mailData is flushed to 
//  mailFile for every 100 lines, or when mail is done.
// If no MessageId is identified and all headers are read
//  then mailFile is created by using a Random UUID filename
pub struct EmailData {
  client_helo: String,
  mail_from: String,
  archive_path: String,
  rcpt_to: Vec<String>,
  mail_data: Vec<String>,   // mail data lines
  mail_file: Option<File>,          // mail backup file
  datetime: time::Tm,
  prefix: String,
  archivers: Vec<config::ArchiverSetup>,
}

pub fn clear_emaildata(mut md: EmailData) -> EmailData {
  md.mail_from = "".to_string();
  md.archive_path = "".to_string();
  md.rcpt_to = Vec::new();
  md.mail_data = Vec::new();
  md.mail_file = None;
  md.datetime = time::empty_tm();
  md
}


pub fn make_emaildata(prefix: String, archivers: Vec<config::ArchiverSetup>) -> EmailData {
  EmailData {
    client_helo: "".to_string(),
    mail_from: "".to_string(),
    rcpt_to: Vec::new(),
    archive_path: "".to_string(),
    mail_data: Vec::new(),
    mail_file: None,
    datetime: time::empty_tm(),
    archivers: archivers,
    prefix: prefix,
  }
}


pub trait Chatty<T: 'static>: ServerProto<T> {
    type State: 'static;

    fn map_future(fut: Box<Future<Item = Self::Transport, Error = io::Error>>) -> Self::BindTransport;

    // Send a response, then launch next step
    fn send_line(transport: Self::Transport, st: Self::State, response: Self::Response, nextstep: Box<Fn(Self::Transport, Self::State) -> Self::BindTransport>) -> Self::BindTransport {
       let hs = Box::new(transport.send(response).into_future().map_err(|e| e)
            .and_then(move |tx| nextstep(tx, st))) as Box<Future<Item = Self::Transport, Error = io::Error>>;
       Self::map_future(hs)
    }

    // await the response
    fn await_line(transport: Self::Transport, st: Self::State, action: Box<Fn(Self::Transport, Self::Request, Self::State) -> Self::BindTransport>) -> Self::BindTransport {
      let hs = Box::new(transport.into_future().map_err(|(e, _)| e).and_then(move |(line, tx)| {
        match line {
          Some(msg) => action(tx, msg, st),
          None => Self::await_line(tx, st, action),
        }
      })) as Box<Future<Item = Self::Transport, Error = io::Error>>;
      Self::map_future(hs)
    }


    // send a response and await something
/*
   fn send_await_line(transport: Self::Transport, st: Self::State, response: Self::Response, action: Box<Fn(Self::Transport, Self::Request, Self::State) -> Self::BindTransport>) -> Self::BindTransport 
   {
     Self::send_line(transport, st, response, Box::new({
        move |tx, st| Self::await_line(tx, st, action) }))
   }
*/

   
}

pub struct SmtpProto {
  archivers: Vec<config::ArchiverSetup>,
  servername: String,
}

impl<T: Io + 'static> ServerProto<T> for SmtpProto {
  type Request = String;
  type Response = String;

  type Transport = Framed<T, ASCIILineBased>;
  type BindTransport = Box<Future<Item = Self::Transport, Error = io::Error>>;

  fn bind_transport(&self, io: T) -> Self::BindTransport {
    let transport = io.framed(ASCIILineBased);
    let md = make_emaildata("none".to_string(), self.archivers.clone());
    Self::greet(transport, md, self.servername.clone())
  }
}

impl SmtpProto {

  pub fn bind_transport<T>(&self, io: T, md: EmailData) -> <Self as ServerProto<T>>::BindTransport 
   where T: Io + 'static {
     let transport = io.framed(ASCIILineBased);
     Self::greet(transport, md, self.servername.clone())
  }

  pub fn new (servername: String, archivers: Vec<config::ArchiverSetup>) -> SmtpProto {
    SmtpProto { servername: servername, archivers: archivers }
  } 

  pub fn set_archivers (&mut self, archivers: Vec<config::ArchiverSetup>) -> () {
    self.archivers = archivers;
  }

  pub fn set_servername (&mut self, servername: String) -> () {
    self.servername = servername;
  }

/*
  pub fn lookup_archivepath (&mut self, recipient: String) -> Option<String> {
    for m in self.archivers.iter() {
      if m.recipient == recipient {
        return Some(m.archive_path.clone());
      }
    }
    return None;
  }
*/
}


impl<T: Io + 'static> Chatty<T> for SmtpProto {
  type State = EmailData;

  fn map_future(fut: Box<Future<Item = Self::Transport, Error = io::Error>>) -> Self::BindTransport {
    fut as Self::BindTransport
  }

} 


impl SmtpProto {

  fn greet<T: Io + 'static> (tx: <Self as ServerProto<T>>::Transport, md: <Self as Chatty<T>>::State, servername: String) -> <Self as ServerProto<T>>::BindTransport {
    debug!("Connection from {} sending 220 greeting", "clientname");
    Self::send_line(tx, md, format!("220 {}", servername), Box::new(Self::wait_for_client_helo))
  }

  fn respond_to_quit<T: Io + 'static> (tx: <Self as ServerProto<T>>::Transport) -> <Self as ServerProto<T>>::BindTransport {
    Box::new(tx.send("221 Bye".to_string()).and_then(|_| Err(io::Error::new(io::ErrorKind::Other, "Client closed"))))
    // in tokio-core 0.2 we'll have the opportunity to signal connection shutdown
  }

  fn wait_for_client_helo<T: Io + 'static> (tx: <Self as ServerProto<T>>::Transport, md: <Self as Chatty<T>>::State) -> <Self as ServerProto<T>>::BindTransport {
    Self::await_line(tx, md, Box::new(move |tx,line,mut st| {
         if line.starts_with("HELO") {
           st.client_helo = line;
           Self::send_line(tx, st, "250 Ok".to_string(), Box::new(Self::wait_for_mail_from))
         } else if line.starts_with("EHLO") {
           st.client_helo = line;
           Self::send_line(tx, st, "250 Ok".to_string(), Box::new(Self::wait_for_mail_from))
         } else if line.starts_with("QUIT") {
           Self::respond_to_quit(tx) 
         } else {
           Self::send_line(tx, st, "502 invalid helo".to_string(), Box::new(Self::wait_for_client_helo))
         }
       }))
  }

  fn wait_for_mail_from<T: Io + 'static> (tx: <Self as ServerProto<T>>::Transport, md: <Self as Chatty<T>>::State) -> <Self as ServerProto<T>>::BindTransport {
    Self::await_line(tx, md, Box::new(move |tx, line, mut st| {
      if line.starts_with("MAIL FROM:") {
        st.mail_from = line;
        Self::send_line(tx, st, "250 Ok".to_string(), Box::new(Self::wait_for_rcpt_to))
      } else if line.starts_with("QUIT") {
        Self::respond_to_quit(tx)
      } else {
        Self::send_line(tx, st, "502 Invalid mail from".to_string(), Box::new(Self::wait_for_mail_from))
      }
    }))
  }

  fn wait_for_rcpt_to<T: Io + 'static> (tx: <Self as ServerProto<T>>::Transport, md: <Self as Chatty<T>>::State) -> <Self as ServerProto<T>>::BindTransport {
    Self::await_line(tx, md, Box::new(move |tx, line, mut st| {
      if line.starts_with("RCPT TO:") {
        st.rcpt_to.push(line.clone());
        let (_,trimmed) = line.split_at(9);
        // lookup archive path
        for m in st.archivers.iter() {
          if m.recipient == trimmed {
            st.archive_path = m.archive_path.clone();
            debug!("Setting archive path for recipient {} to {}", trimmed, m.archive_path);
          }
        }
        Self::send_line(tx, st, "250 Ok".to_string(), Box::new(Self::wait_for_rcpt_to))
      } else
      if line.starts_with("DATA") && st.rcpt_to.len() > 0 {
        st.datetime = time::now_utc();
        Self::send_line(tx, st, "354 End data with <CR><LF>.<CR><LF>".to_string(), Box::new(Self::get_data))
      } else {
        Self::send_line(tx, st, "502 Invalid command".to_string(), Box::new(Self::wait_for_rcpt_to))
      }
    }))
  }


  fn get_data<T: Io + 'static> (tx: <Self as ServerProto<T>>::Transport, md: <Self as Chatty<T>>::State) -> <Self as ServerProto<T>>::BindTransport {
    Self::await_line(tx, md, Box::new(move |tx, line, mut st| {
      if line == "." {
        // spool data
        // .. and close file
        st = Self::drain_lines (st);
        match st.mail_file {
          Some(ref mut file) => {
            if let Ok(bytes) = file.seek(SeekFrom::Current(0)) {
              info!("Spooled {} bytes to file", bytes);
            } else {
              info!("Spooled mail data to file");
            }
          },
          None => {
            // no message id found, no distinction betw headers and body
            // spool to some uuid v4 determined filename
            let uuid = Uuid::new_v4().hyphenated().to_string();
            let file = Self::make_file(&st, &uuid);
            st.mail_file = Some(file);
            st = Self::drain_lines(st);
            match st.mail_file.some().seek(SeekFrom::Current(0)) {
              Ok(bytes) => info!("Spooled {} bytes to file", bytes),
              _ => info!("Spooled mail data to file"),
            };
          }
        };

        let md = clear_emaildata(st);
        Self::send_line(tx, md, "250 Ok: queued".to_string(), Box::new(Self::wait_for_mail_from))
      } else {
        if line.starts_with("Message-ID:") {
          match st.mail_file {
             None => {
               match Self::parse_messageid(&line) {
                 None => {},
                 Some((_messageid, safe)) => {
                   // now make a mail file
                   let file = Self::make_file(&st, &safe);
                   st.mail_file = Some(file);
                   st = Self::drain_lines(st);
                 }
               }
             },
             _ => () // we already have a mail file
          }
        } else
        if line == "" {  // header done, make a decision on destination file
          match st.mail_file {
            Some(_) => { // all good we're spooling i
            },
            None => { // No suitable message id found, make uuid
              let uuid = Uuid::new_v4().hyphenated().to_string();
              let file = Self::make_file(&st, &uuid);
              st.mail_file = Some(file);
              st = Self::drain_lines(st);
            }
          }
        }
        st.mail_data.push(line);

        // flush stored lines
        if st.mail_data.len() > 64 {
          st = Self::drain_lines(st);
        }
        Self::get_data(tx, st)
      }
    }))
  }

  fn drain_lines (mut md: EmailData) -> EmailData {
    // easiest solution to "cannot move out of borrowed content" was to 'take'
    // the value and put it back in...
    match md.mail_file.take() {
      None => {
      },
      Some(mut file) => {
        for m in md.mail_data.drain(..) {
          let _ = file.write_all(m.as_bytes());
          let _ = file.write_all(b"\r\n");
        }
        md.mail_file = Some(file);
      }
    }
    md
  }


  fn make_file (md: &EmailData, name: &String) -> File {
    let tmpath = match time::strftime(&md.archive_path, &md.datetime) {
      Ok(p) => p,
      _ => { "/tmp".to_string() }
    };
    let filepath = format!("{}/{}-{}.eml", tmpath, md.prefix, name);
    info!("Spooling mail to {}", filepath.clone());
    match File::create(filepath.clone()) {
      Ok(file) => {
        return file;
      },
      Err(_) => {
        // perhaps dir is not created...
        let _ = fs::create_dir_all(tmpath);
        // now try again before failing.
        return File::create(filepath).unwrap();
      }
    }
  }
     
  fn parse_messageid(line: &String) -> Option<(String, String)> {
    
    let myline = line.clone();
    let (_,trimmed) = myline.split_at(11);
    // remove leading and trailing spaces and <, >
    let trimmed = trimmed.trim_matches(|c| c == ' ' || c == '<' || c == '>').to_string();

    // sanity check; message ids with less than 12 chars
    // will not guarantee uniqueness good enough, do something else
    if trimmed.len() < 12 {
      return None;
    }

    let safe: String = trimmed.chars().map(|c| match c {
        x @ 'A'...'Z' => x,
        x @ 'a'...'z' => x,
        x @ '0'...'9' => x,
        x @ '.'| x @ '-'| x @ '+'| x @ '@'| x @ '=' => x,
        _ => 'X' }).collect();
    
    Some((trimmed, safe))
  }
}
