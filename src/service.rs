use tokio_service::Service;
use std::io;
use futures::future;
use futures::Future;

pub struct MailArchiver;

impl Service for MailArchiver {
  type Request = String;
  type Response = String;
  type Error = io::Error;
  type Future = Box<Future<Item = String, Error = Self::Error>>; // Box<Future<Item = Self::Response, Error = Self::Error>>;

  fn call(&self, _: Self::Request) -> Self::Future {
    future::done(Err(io::Error::new(io::ErrorKind::Other, "Client closed"))).boxed()
  }
}

