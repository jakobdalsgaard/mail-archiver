
use std::fs::File;
use std::io::prelude::*;
use yaml_rust::YamlLoader;

pub struct Config {
  pub listen: String,
  pub servername: String,
  pub user: Option<String>,
  pub group: Option<String>,
  pub log_level: String,
  pub archivers: Vec<ArchiverSetup>,
}

pub struct ArchiverSetup {
  pub recipient: String,
  pub archive_path: String,
}

impl Clone for ArchiverSetup {
  fn clone (&self) -> ArchiverSetup {
    ArchiverSetup { recipient: self.recipient.clone(), archive_path: self.archive_path.clone() }
  }
}

fn libc_gethostname() -> String {
  "gethostname_to_be_implemented".to_string()
}

pub fn read_config(config_file: &String) -> Result<Config, String> {
  let mut file = match File::open(config_file) {
    Ok(f) => f,
    Err(err) => {
      return Err(format!("Cannot open configuration file due to: {}", err));
    },
  };
  let mut content = String::new();
  let _ = file.read_to_string(&mut content);
  let yaml = YamlLoader::load_from_str(&content).unwrap();
  let doc = &yaml[0];

  let config_listen = match doc["listen"].as_str() {
    Some(value) => value.to_string(),
    None => {
      return Err("Required configuration parameter 'listen' not found".to_string());
    }
  };

  let config_servername = match doc["servername"].as_str() {
    Some(value) => value.to_string(),
    None => libc_gethostname(),
  };

  let config_log_level = match doc["log_level"].as_str() {
    Some(value) => value.to_string(),
    None => "INFO".to_string(),
  };


  let config_user = match doc["user"].as_str() {
    None => None,
    Some(str) => Some(str.to_string()),
  };

  let config_group = match doc["group"].as_str() {
    None => None,
    Some(str) => Some(str.to_string()),
  };
 
  let mut config_archivers: Vec<ArchiverSetup> = Vec::new();
  let mut i:i32 = 0;
  for yaml in doc["archivers"].as_vec().unwrap() {
    match (yaml["recipient"].as_str(), yaml["archive_path"].as_str()) {
          (Some(r), Some(a)) => config_archivers.push(ArchiverSetup{ recipient: r.to_string(), archive_path: a.to_string() }),
          (Some(r), None) => return Err(format!("found recipient {}, but no archive path, in 'archivers[{}]'", r, i)),
          (None, Some(a)) => return Err(format!("found archive_path {}, but no recipient, in 'archivers[{}]'", a, i)),
          (None, None) => return Err(format!("malformed entries in 'archivers[{}]'", i)),
        }
    i = i + 1;
  };

  Ok(Config {
    listen: config_listen,
    servername: config_servername,
    log_level: config_log_level,
    user: config_user,
    group: config_group,
    archivers: config_archivers,
  })
}


