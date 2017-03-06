extern crate futures;
extern crate tokio_core;
extern crate tokio_proto;
extern crate tokio_service;
extern crate tokio_signal;
extern crate service_fn;
extern crate encoding;
extern crate time;
extern crate uuid;
extern crate getopts;
extern crate yaml_rust;
extern crate libc;

#[macro_use]
extern crate log;
extern crate badlog;

use std::thread;
use futures::Future;
use futures::stream::Stream;
use tokio_core::reactor::{Core, Handle};
use tokio_core::net::TcpListener;
use tokio_core::io::{IoStream, IoFuture};
use tokio_signal::unix;
use getopts::Options;
use std::env;
use std::process;
use std::sync::Arc;
use tokio_proto::BindServer;
use tokio_proto::TcpServer;
use service_fn::service_fn;
use std::ffi::CString;
use libc::{getpid, setgid, setuid, getgrnam, getpwnam};


mod codec;
mod config;
mod service;

fn print_usage(opts: Options) {
  let brief = "Usage: mail-archiver --config [YAML-CONFIG]";
  print!("{}", opts.usage(&brief));
  println!("");
}


enum Incoming<T> {
  Connection(T),
  Usr1,
}

fn main() {

    // make options structure
    let mut opts = Options::new();
    opts.optopt("c", "config", "Yaml configuration file for mail-archiver", "FILE");
    opts.optflag("t", "template", "print out a template configuration file and exit");
    opts.optflag("h", "help", "print this help");
    let args: Vec<String> = env::args().collect();
    let matches = match opts.parse(&args[1..]) {
      Ok(m) => m,
      Err(f) => {
         print_usage(opts);
         println!("Cannot parse command line: {}", f.to_string());
         process::exit(2);
       },
    };

    // should we just print a template yml file?
    if matches.opt_present("t") {
      println!("---
listen: 192.168.1.77:25
servername: server.domain.com
user: mailarchive
group: mailarchive
log_level: DEBUG
archivers:
    - recipient: archive@domain.com
      archive_path: /mnt/storage/archive/%Y/%m-%d/%H:00
    - recipient: smallarchive@domain.com
      archive_path: /mnt/storage/smallarchive/%Y/%m-%d
");
       process::exit(0);
    }

    if matches.opt_present("h") {
      print_usage(opts);
      process::exit(0);
    }

    let config_file = {
      match matches.opt_str("c") {
        Some(s) => s,
        None => {
          println!("Supply -c/--config parameter");
          process::exit(1);
        },
      }};
    let mut config = match config::read_config(&config_file) {
      Ok(c) => c,
      Err(e) => {
        panic!("Cannot read configuration file: {}, due to {}", config_file, e);
      }};


    badlog::init(Some(config.log_level.clone()));
    let pid = unsafe { getpid() };
    
    info!("mail-archiver starting up, pid is {}, read config from {}, listening on {}, {} archiver setups configured, log level set to {}",
           pid, config_file, config.listen, config.archivers.len(), config.log_level);
    // Specify the localhost address
    let addr = config.listen.parse().unwrap();

    // make the core
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let socket = TcpListener::bind(&addr, &handle).unwrap();

    // downgrade uid/gid
    if config.user.is_some() {
      let c_str = CString::new(config.user.clone().unwrap()).unwrap();
      let pw_uid = unsafe {
        let pw = getpwnam(c_str.as_ptr());
        (*pw).pw_uid
      };
      info!("doing setuid to user {} (uid: {})", config.user.clone().unwrap(), pw_uid);
      unsafe { setuid(pw_uid); };
    }


    if config.group.is_some() {
      let c_str = CString::new(config.group.clone().unwrap()).unwrap();
      let pw_gid = unsafe {
        let pw = getgrnam(c_str.as_ptr());
        (*pw).gr_gid
      };
      info!("doing setgid to group {} (gid: {})", config.group.clone().unwrap(), pw_gid);
      unsafe { setgid(pw_gid); };
    }

    let usr1 = sig_usr1(&handle);
    // make the stream
    let usr1_stream = core.run(usr1).unwrap();

    let prg_prefix = time::strftime("%H%M%S", &time::now_utc()).unwrap();
    let connection_counter = 0u64;

    // combine all streams to one
    let all = socket.incoming().map(|c| Incoming::Connection(c))
             .select(usr1_stream.map(|_| Incoming::Usr1));

    let mut binder = codec::SmtpProto::new(config.servername.clone(), config.archivers.clone());
    // let new_service = service::new_service(&handle);
    let server = all.for_each(move |m| {
      match m {
        Incoming::Connection((socket, addr)) => {
          debug!("incoming connection from {}", addr);
          let this_prefix = format!("{}-{:06x}", &prg_prefix, &connection_counter);
          // we need to pass this prefix to service, but service is stateless :/
          // i.e. re-implement without the use of service!
          let connection_counter = connection_counter + 1;
          let service = service::MailArchiver { };
          binder.bind_server(&handle, socket, service);
          Ok(())
        },
        Incoming::Usr1 => {
          debug!("signal usr1 receieved, reloading config {}", &config_file);
          match config::read_config(&config_file) {
            Ok(c) => {
              config = c;
              info!("reloaded config from {} on signal usr1", &config_file);
              binder.set_archivers(config.archivers.clone());
              binder.set_servername(config.servername.clone());
            },
            Err(e) => {
              error!("Cannot use configuration file: {}, due to {}", config_file, e);
            }
          };
          Ok(())
        },
      }
    });
    core.run(server).unwrap();

    // The builder requires a protocol and an address
    // would be nice to use TcpServer, but it lacks features.
    // let server = TcpServer::new(codec::SmtpProto, addr);

    // instantiate signal handlers
    
    // We provide a way to *instantiate* the service for each new
    // connection; here, we just immediately return a new instance.
}


// signal handlers
pub fn sig_usr1(handle: &Handle) -> IoFuture<IoStream<()>> {
    return sig_usr1_imp(handle);

    fn sig_usr1_imp(handle: &Handle) -> IoFuture<IoStream<()>> {
        unix::Signal::new(unix::libc::SIGUSR1, handle).map(|x| {
            x.map(|_| ()).boxed()
        }).boxed()
    }
}
