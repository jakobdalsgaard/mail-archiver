# mail-archiver

SMTP mimicking mail archiver daemon; implemented in Rust/Tokio for total and absolute awesomeness.

Unfinished mail-archive daemon, mimicking the SMTP protocol; useful for when mail archiving is needed in
an existing mail infrastructure. Just have existing incoming and outgoing mailservers send all email
to this daemon, and configure it with proper archiving paths.

Setting up the application is fairly easy. Check out the repository, and issue:

```
cargo build
```

The `mail-archiver` binary is now built; it basically supports the following command line arguments:

```
Usage: mail-archiver --config [YAML-CONFIG]

Options:
    -c, --config FILE   Yaml configuration file for mail-archiver
    -t, --template      print out a template configuration file and exit
    -h, --help          print this help
```
    
Currently it logs on stderr, colored, it reloads the servername and archivers configuration on signal USR1.
The application has support for setgid/setuid to happen after TCP port has been acquired.

Application is totally untested. Use at your own risk.

Known problems, 2017-02-28:

1. It does not parse "RCPT TO" email addresses, i.e. "RCPT TO" values must be exactly what is configured
   in setup yml file. 
2. On client dropping the connection, the application breaks.
