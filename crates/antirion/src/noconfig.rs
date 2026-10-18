//! A node started without `--config`: a standalone gateway with the embedded store in a data
//! directory, both planes on localhost, and a bootstrap token and an encryption key generated into
//! the data directory on the first start. Every setting comes from a flag, else an environment
//! variable, else a default; a banner on stdout says where the console is and how to sign in.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use serde_json::{Value, json};

/// The settings of a node without `--config`; each also reads the environment variable it names.
#[derive(Debug, Default, Clone, clap::Args)]
#[command(next_help_heading = "Without --config")]
pub struct NoConfigArgs {
    /// The data directory: the embedded store, its spool and the generated keys
    /// [env: ANTIRION_DATA_DIR] [default: $XDG_DATA_HOME/antirion, else ~/.antirion]
    #[arg(long, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,
    /// The address both planes listen on [env: ANTIRION_HOST] [default: 127.0.0.1]
    #[arg(long, value_name = "ADDRESS")]
    pub host: Option<String>,
    /// The data plane's port, the `/v1` API [env: ANTIRION_DATA_PORT] [default: 8080]
    #[arg(long, value_name = "PORT")]
    pub data_port: Option<u16>,
    /// The admin plane's port, the Admin API and the console [env: ANTIRION_ADMIN_PORT] [default: 8081]
    #[arg(long, value_name = "PORT")]
    pub admin_port: Option<u16>,
    /// The origin browsers open the console at [env: ANTIRION_PUBLIC_ORIGIN]
    /// [default: http://localhost:<admin port>]
    #[arg(long, value_name = "URL")]
    pub public_origin: Option<String>,
    /// The data plane's URL as clients see it, shown in the console [env: ANTIRION_DATA_PLANE_URL]
    /// [default: http://localhost:<data port>]
    #[arg(long, value_name = "URL")]
    pub data_plane_url: Option<String>,
}

impl NoConfigArgs {
    /// The first flag given, by name: these flags apply only without `--config`.
    #[must_use]
    pub fn first_given(&self) -> Option<&'static str> {
        [
            (self.data_dir.is_some(), "--data-dir"),
            (self.host.is_some(), "--host"),
            (self.data_port.is_some(), "--data-port"),
            (self.admin_port.is_some(), "--admin-port"),
            (self.public_origin.is_some(), "--public-origin"),
            (self.data_plane_url.is_some(), "--data-plane-url"),
        ]
        .into_iter()
        .find_map(|(given, name)| given.then_some(name))
    }
}

/// Where the bootstrap token comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenSource {
    /// `auth.admin.bootstrap_token` of the config document.
    Configured,
    /// `ANTIRION_BOOTSTRAP_TOKEN`.
    Env,
    /// Generated on this start into the file: the banner shows it, this once.
    Generated(PathBuf),
    /// Generated on an earlier start, kept in the file.
    Kept(PathBuf),
}

/// The resolved settings of a node without `--config`.
#[derive(Clone)]
pub struct NoConfig {
    pub data_dir: PathBuf,
    pub host: String,
    pub data_port: u16,
    pub admin_port: u16,
    pub public_origin: String,
    pub data_plane_url: String,
    /// `ANTIRION_BOOTSTRAP_TOKEN`: the bootstrap token instead of a generated one.
    pub bootstrap_token: Option<String>,
    /// `ANTIRION_ENCRYPTION_KEY`, decoded: the key that encrypts stored secrets instead of a
    /// generated one.
    pub encryption_key: Option<Vec<u8>>,
}

impl std::fmt::Debug for NoConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secret = |set: bool| if set { "<set>" } else { "<none>" };
        f.debug_struct("NoConfig")
            .field("data_dir", &self.data_dir)
            .field("host", &self.host)
            .field("data_port", &self.data_port)
            .field("admin_port", &self.admin_port)
            .field("public_origin", &self.public_origin)
            .field("data_plane_url", &self.data_plane_url)
            .field("bootstrap_token", &secret(self.bootstrap_token.is_some()))
            .field("encryption_key", &secret(self.encryption_key.is_some()))
            .finish()
    }
}

/// The text a node without `--config` prints once it listens; it may hold the admin token, so its
/// `Debug` does not show it.
#[derive(Clone)]
pub struct Banner(pub String);

impl std::fmt::Debug for Banner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Banner(..)")
    }
}

impl NoConfig {
    /// Resolves each setting from its flag, else `env`, else the default.
    pub fn resolve(args: &NoConfigArgs, env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<NoConfig> {
        let env = |name: &str| env(name).filter(|v| !v.trim().is_empty());
        let data_dir = match args
            .data_dir
            .clone()
            .or_else(|| env("ANTIRION_DATA_DIR").map(PathBuf::from))
        {
            Some(d) => d,
            None => default_data_dir(&env),
        };
        let data_dir = std::path::absolute(&data_dir)
            .with_context(|| format!("the data directory {} has no absolute path", data_dir.display()))?;
        let host = args
            .host
            .clone()
            .or_else(|| env("ANTIRION_HOST"))
            .unwrap_or_else(|| "127.0.0.1".to_owned());
        let host = host.trim_start_matches('[').trim_end_matches(']').to_owned();
        let port = |flag: Option<u16>, name: &str, default: u16| -> anyhow::Result<u16> {
            match flag {
                Some(p) => Ok(p),
                None => env(name).map_or(Ok(default), |v| {
                    v.trim()
                        .parse()
                        .map_err(|_| anyhow!("{name} must be a port number, not {v:?}"))
                }),
            }
        };
        let data_port = port(args.data_port, "ANTIRION_DATA_PORT", 8080)?;
        let admin_port = port(args.admin_port, "ANTIRION_ADMIN_PORT", 8081)?;
        let browser_host = browser_host(&host);
        let public_origin = args
            .public_origin
            .clone()
            .or_else(|| env("ANTIRION_PUBLIC_ORIGIN"))
            .unwrap_or_else(|| format!("http://{browser_host}:{admin_port}"));
        let public_origin = origin(&public_origin, "the public origin")?;
        let data_plane_url = args
            .data_plane_url
            .clone()
            .or_else(|| env("ANTIRION_DATA_PLANE_URL"))
            .unwrap_or_else(|| format!("http://{browser_host}:{data_port}"));
        let data_plane_url = origin(&data_plane_url, "the data plane URL")?;
        let encryption_key = env("ANTIRION_ENCRYPTION_KEY")
            .map(|k| {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD
                    .decode(k.trim())
                    .ok()
                    .filter(|k| k.len() == 32)
                    .ok_or_else(|| anyhow!("ANTIRION_ENCRYPTION_KEY must be the base64 of 32 bytes"))
            })
            .transpose()?;
        Ok(NoConfig {
            data_dir,
            host,
            data_port,
            admin_port,
            public_origin,
            data_plane_url,
            bootstrap_token: env("ANTIRION_BOOTSTRAP_TOKEN").map(|t| t.trim().to_owned()),
            encryption_key,
        })
    }

    /// The startup document: static keys only. Dynamic keys in it would be applied on every start
    /// and would take back what the console changed.
    #[must_use]
    pub fn document(&self) -> Value {
        let data = self.data_dir.to_string_lossy();
        json!({
            "version": 1,
            "server": {"listen": listen(&self.host, self.data_port)},
            "admin": {
                "listen": listen(&self.host, self.admin_port),
                "ui": {"data_plane_url": self.data_plane_url},
            },
            "store": {
                "backend": "embedded",
                "embedded": {"path": data},
                "spool": {"path": self.data_dir.join("spool").to_string_lossy()},
            },
        })
    }

    /// Creates the data directory, readable by its owner only, when it does not exist.
    pub fn prepare(&self) -> anyhow::Result<()> {
        create_private_dir(&self.data_dir)
            .with_context(|| format!("cannot create the data directory {}", self.data_dir.display()))
    }

    /// The text printed once the node listens.
    #[must_use]
    pub fn banner(&self, version: &str, token: &str, source: &TokenSource) -> Banner {
        let console = format!("{}/ui/", self.public_origin);
        let mut lines = vec![
            format!("Antirion {version} is running."),
            String::new(),
            format!("  Console      {console}"),
        ];
        match source {
            TokenSource::Generated(file) => {
                lines.push(format!("  Admin token  {token}"));
                lines.push("               Shown on this first start only; it is kept in".to_owned());
                lines.push(format!("               {}", file.display()));
            }
            TokenSource::Kept(file) => {
                lines.push(format!("  Admin token  in {}", file.display()));
            }
            TokenSource::Env => {
                lines.push("  Admin token  the value of ANTIRION_BOOTSTRAP_TOKEN".to_owned());
            }
            TokenSource::Configured => {
                lines.push("  Admin token  auth.admin.bootstrap_token of the config file".to_owned());
            }
        }
        lines.push(format!("  Data plane   {}/v1", self.data_plane_url));
        lines.push(format!("  Data dir     {}", self.data_dir.display()));
        lines.push(String::new());
        lines.push(format!(
            "Open {console} and sign in with the admin token, then add a provider, a model and an API key."
        ));
        let mut text = lines.join("\n");
        text.push('\n');
        Banner(text)
    }
}

/// `$XDG_DATA_HOME/antirion`, else `~/.antirion`, else `./antirion-data`.
fn default_data_dir(env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(xdg) = env("XDG_DATA_HOME").filter(|d| Path::new(d).is_absolute()) {
        return PathBuf::from(xdg).join("antirion");
    }
    let home = env("HOME").or_else(|| env("USERPROFILE"));
    match home {
        Some(h) => PathBuf::from(h).join(".antirion"),
        None => PathBuf::from("antirion-data"),
    }
}

/// The host a browser on this machine reaches a listener on: `localhost` for a loopback or
/// unspecified address, which the console's secure cookie needs over plain HTTP.
fn browser_host(host: &str) -> String {
    match host.parse::<IpAddr>() {
        Ok(ip) if ip.is_loopback() || ip.is_unspecified() => "localhost".to_owned(),
        Ok(IpAddr::V6(ip)) => format!("[{ip}]"),
        _ => host.to_owned(),
    }
}

fn listen(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// An `http(s)://host[:port]` origin, without a trailing slash.
fn origin(url: &str, what: &str) -> anyhow::Result<String> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| anyhow!("{what} must start with http:// or https://, not {url:?}"))?;
    if rest.is_empty() || rest.contains('/') {
        bail!("{what} must be a scheme, a host and an optional port, such as http://localhost:8081");
    }
    Ok(url.to_owned())
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn the_defaults_are_a_local_gateway_in_the_home_directory() {
        let nc = NoConfig::resolve(&NoConfigArgs::default(), &env(&[("HOME", "/home/me")])).unwrap();
        assert_eq!(nc.data_dir, PathBuf::from("/home/me/.antirion"));
        assert_eq!(nc.public_origin, "http://localhost:8081");
        assert_eq!(nc.data_plane_url, "http://localhost:8080");
        let doc = nc.document();
        assert_eq!(doc["server"]["listen"], "127.0.0.1:8080");
        assert_eq!(doc["admin"]["listen"], "127.0.0.1:8081");
        assert_eq!(doc["store"]["embedded"]["path"], "/home/me/.antirion");
        assert_eq!(doc["admin"]["ui"]["data_plane_url"], "http://localhost:8080");
        assert!(doc.get("auth").is_none(), "no dynamic key: {doc}");
        let mut checked = doc.clone();
        let validation = antirion_config::validate(&mut checked, &Value::Null);
        assert!(validation.is_valid(), "{:?}", validation.errors);
    }

    #[test]
    fn xdg_data_home_wins_over_home_and_the_environment_over_defaults() {
        let nc = NoConfig::resolve(
            &NoConfigArgs::default(),
            &env(&[
                ("HOME", "/home/me"),
                ("XDG_DATA_HOME", "/xdg"),
                ("ANTIRION_HOST", "0.0.0.0"),
                ("ANTIRION_ADMIN_PORT", "9081"),
            ]),
        )
        .unwrap();
        assert_eq!(nc.data_dir, PathBuf::from("/xdg/antirion"));
        assert_eq!(nc.document()["admin"]["listen"], "0.0.0.0:9081");
        assert_eq!(nc.public_origin, "http://localhost:9081");
    }

    #[test]
    fn flags_win_over_the_environment() {
        let args = NoConfigArgs {
            data_dir: Some("/data".into()),
            host: Some("::1".into()),
            data_port: Some(18080),
            public_origin: Some("https://gw.example.com/".into()),
            ..NoConfigArgs::default()
        };
        let nc = NoConfig::resolve(
            &args,
            &env(&[("ANTIRION_DATA_DIR", "/elsewhere"), ("ANTIRION_DATA_PORT", "1")]),
        )
        .unwrap();
        assert_eq!(nc.data_dir, PathBuf::from("/data"));
        assert_eq!(nc.document()["server"]["listen"], "[::1]:18080");
        assert_eq!(nc.public_origin, "https://gw.example.com");
        assert_eq!(nc.data_plane_url, "http://localhost:18080");
        assert_eq!(args.first_given(), Some("--data-dir"));
        assert_eq!(NoConfigArgs::default().first_given(), None);
    }

    #[test]
    fn bad_values_are_refused() {
        let bad = |vars: &[(&str, &str)]| NoConfig::resolve(&NoConfigArgs::default(), &env(vars)).is_err();
        assert!(bad(&[("ANTIRION_DATA_PORT", "http")]));
        assert!(bad(&[("ANTIRION_PUBLIC_ORIGIN", "localhost:8081")]));
        assert!(bad(&[("ANTIRION_PUBLIC_ORIGIN", "http://localhost:8081/ui")]));
        assert!(bad(&[("ANTIRION_ENCRYPTION_KEY", "c2hvcnQ=")]));
        let key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
        let nc = NoConfig::resolve(
            &NoConfigArgs::default(),
            &env(&[("ANTIRION_ENCRYPTION_KEY", key)]),
        )
        .unwrap();
        assert_eq!(nc.encryption_key.map(|k| k.len()), Some(32));
    }

    #[test]
    fn a_specific_address_is_the_browser_host() {
        assert_eq!(browser_host("0.0.0.0"), "localhost");
        assert_eq!(browser_host("::"), "localhost");
        assert_eq!(browser_host("192.168.1.5"), "192.168.1.5");
        assert_eq!(browser_host("fd00::1"), "[fd00::1]");
        assert_eq!(browser_host("gw.lan"), "gw.lan");
    }

    #[test]
    fn the_banner_shows_a_generated_token_only_on_the_start_that_made_it() {
        let nc = NoConfig::resolve(&NoConfigArgs::default(), &env(&[("HOME", "/home/me")])).unwrap();
        let file = PathBuf::from("/home/me/.antirion/bootstrap-token");
        let first = nc
            .banner("1.0.0", "secret-token", &TokenSource::Generated(file.clone()))
            .0;
        assert!(first.contains("http://localhost:8081/ui/"), "{first}");
        assert!(first.contains("secret-token"), "{first}");
        assert!(first.contains("http://localhost:8080/v1"), "{first}");
        assert!(first.contains("/home/me/.antirion/bootstrap-token"), "{first}");
        let later = nc.banner("1.0.0", "secret-token", &TokenSource::Kept(file)).0;
        assert!(!later.contains("secret-token"), "{later}");
        assert!(later.contains("in /home/me/.antirion/bootstrap-token"), "{later}");
        let env_token = nc.banner("1.0.0", "secret-token", &TokenSource::Env).0;
        assert!(!env_token.contains("secret-token"), "{env_token}");
        let shown = nc.banner(
            "1.0.0",
            "secret-token",
            &TokenSource::Generated(PathBuf::from("/t")),
        );
        assert!(!format!("{shown:?}").contains("secret-token"));
        let with_env = NoConfig {
            bootstrap_token: Some("secret-token".into()),
            ..nc
        };
        assert!(!format!("{with_env:?}").contains("secret-token"));
    }
}
