use std::fmt;
use std::io::{self, Write as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use crate::contract::{Credits, Limit};

pub const REQUEST_ID: &str = "gigawatts-usage";

#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub limits: Vec<Limit>,
    pub behaviors: Option<Value>,
}

#[derive(Debug)]
pub enum Error {
    Spawn(io::Error),
    TimedOut(Duration),
    NoResponse { status: String, stderr: String },
    Answered(String),
    Unparsed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Spawn(error) => write!(f, "can't run `claude`: {error}"),
            Error::TimedOut(after) => {
                write!(
                    f,
                    "`claude -p` didn't answer within {} seconds",
                    after.as_secs()
                )
            }
            Error::NoResponse { status, stderr } if stderr.is_empty() => {
                write!(f, "`claude -p` exited ({status}) without answering")
            }
            Error::NoResponse { status, stderr } => {
                write!(
                    f,
                    "`claude -p` exited ({status}) without answering: {stderr}"
                )
            }
            Error::Answered(message) => write!(f, "Claude Code answered with an error: {message}"),
            Error::Unparsed(message) => {
                write!(f, "Claude Code's usage answer doesn't parse: {message}")
            }
        }
    }
}

impl std::error::Error for Error {}

pub trait Runner {
    fn run(&self, config_dir: Option<&Path>, request: &str) -> Result<String, Error>;
}

pub fn fetch(
    runner: &dyn Runner,
    config_dir: Option<&Path>,
    with_behaviors: bool,
) -> Result<Usage, Error> {
    let request = serde_json::json!({
        "type": "control_request",
        "request_id": REQUEST_ID,
        "request": {"subtype": "get_usage", "skip_behaviors": !with_behaviors},
    });
    let stream = runner.run(config_dir, &request.to_string())?;
    parse_stream(&stream)
}

pub struct Claude {
    pub program: PathBuf,
    pub working_dir: PathBuf,
    pub timeout: Duration,
}

impl Runner for Claude {
    fn run(&self, config_dir: Option<&Path>, request: &str) -> Result<String, Error> {
        let mut command = Command::new(&self.program);
        command
            .args([
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
            ])
            .current_dir(&self.working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            // Claude Code signs in with these ahead of the config directory's login.
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .env_remove("CLAUDE_CODE_OAUTH_TOKEN");
        match config_dir {
            Some(config_dir) => command.env("CLAUDE_CONFIG_DIR", config_dir),
            None => command.env_remove("CLAUDE_CONFIG_DIR"),
        };
        let mut child = command.spawn().map_err(Error::Spawn)?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let _ = writeln!(stdin, "{request}");
        drop(stdin);
        let stdout = read_in_background(child.stdout.take().expect("stdout is piped"));
        let stderr = read_in_background(child.stderr.take().expect("stderr is piped"));
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(Error::Spawn)? {
                break status;
            }
            if Instant::now() >= deadline {
                kill_group(child.id());
                let _ = child.wait();
                return Err(Error::TimedOut(self.timeout));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        kill_group(child.id());
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .max(PIPES_CLOSE_WITHIN);
        let (Ok(stdout), Ok(stderr)) = (stdout.recv_timeout(wait), stderr.recv_timeout(wait))
        else {
            return Err(Error::TimedOut(self.timeout));
        };
        if !stdout.contains(REQUEST_ID) {
            return Err(Error::NoResponse {
                status: status.to_string(),
                stderr: last_line(&stderr),
            });
        }
        Ok(stdout)
    }
}

const PIPES_CLOSE_WITHIN: Duration = Duration::from_secs(1);

fn read_in_background(mut pipe: impl io::Read + Send + 'static) -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        let _ = sender.send(text);
    });
    receiver
}

fn kill_group(leader: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{leader}")])
        .stderr(Stdio::null())
        .status();
}

fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[derive(Deserialize)]
struct StreamLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    response: Option<ControlResponse>,
}

#[derive(Deserialize)]
struct ControlResponse {
    subtype: Option<String>,
    request_id: Option<String>,
    error: Option<String>,
    response: Option<Value>,
}

pub fn parse_stream(stream: &str) -> Result<Usage, Error> {
    let mut unparsed_answer = None;
    for text in stream.lines() {
        let line = match serde_json::from_str::<StreamLine>(text) {
            Ok(line) => line,
            Err(error) => {
                if text.contains(REQUEST_ID) {
                    unparsed_answer.get_or_insert(error.to_string());
                }
                continue;
            }
        };
        let Some(response) = line
            .response
            .filter(|_| line.kind.as_deref() == Some("control_response"))
        else {
            continue;
        };
        if response.request_id.as_deref() != Some(REQUEST_ID) {
            continue;
        }
        if response.subtype.as_deref() != Some("success") {
            return Err(Error::Answered(
                response
                    .error
                    .unwrap_or_else(|| "no error message".to_string()),
            ));
        }
        let response = response
            .response
            .ok_or_else(|| Error::Unparsed("the answer has no response".to_string()))?;
        return usage_from(response);
    }
    Err(match unparsed_answer {
        Some(error) => Error::Unparsed(error),
        None => Error::NoResponse {
            status: String::new(),
            stderr: String::new(),
        },
    })
}

pub fn parse_usage(text: &str) -> Result<Usage, Error> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| Error::Unparsed(error.to_string()))?;
    usage_from(value)
}

#[derive(Deserialize)]
struct RawUsage {
    rate_limits_available: Option<bool>,
    rate_limits: Option<RateLimits>,
    behaviors: Option<Value>,
}

fn usage_from(value: Value) -> Result<Usage, Error> {
    let raw: RawUsage =
        serde_json::from_value(value).map_err(|error| Error::Unparsed(error.to_string()))?;
    let Some(rate_limits) = raw
        .rate_limits
        .filter(|_| raw.rate_limits_available != Some(false))
    else {
        return Err(Error::Answered(
            "no limits are available for this account".to_string(),
        ));
    };
    Ok(Usage {
        limits: rate_limits.limits(),
        behaviors: raw.behaviors.filter(|behaviors| !behaviors.is_null()),
    })
}

#[derive(Deserialize)]
pub struct RateLimits {
    #[serde(default)]
    limits: Vec<RawLimit>,
    spend: Option<Spend>,
    extra_usage: Option<ExtraUsage>,
}

#[derive(Deserialize)]
struct RawLimit {
    kind: String,
    group: Option<String>,
    percent: f64,
    severity: Option<String>,
    resets_at: Option<String>,
    scope: Option<Scope>,
    #[serde(default)]
    is_active: bool,
}

#[derive(Deserialize)]
struct Scope {
    model: Option<ScopeModel>,
    surface: Option<Value>,
}

#[derive(Deserialize)]
struct ScopeModel {
    id: Option<String>,
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct Spend {
    #[serde(default)]
    enabled: bool,
    used: Option<Money>,
    limit: Option<Money>,
    percent: Option<f64>,
    severity: Option<String>,
}

#[derive(Deserialize)]
struct Money {
    amount_minor: i64,
    currency: Option<String>,
    exponent: i32,
}

impl Money {
    fn amount(&self) -> f64 {
        self.amount_minor as f64 / 10_f64.powi(self.exponent)
    }
}

#[derive(Deserialize)]
struct ExtraUsage {
    #[serde(default)]
    is_enabled: bool,
    monthly_limit: Option<i64>,
    used_credits: Option<i64>,
    utilization: Option<f64>,
    currency: Option<String>,
    decimal_places: Option<i32>,
}

impl RateLimits {
    pub fn limits(&self) -> Vec<Limit> {
        let mut limits: Vec<Limit> = self.limits.iter().map(RawLimit::limit).collect();
        limits.extend(self.spend_credits().or_else(|| self.extra_usage_credits()));
        limits
    }

    fn spend_credits(&self) -> Option<Limit> {
        let spend = self.spend.as_ref().filter(|spend| spend.enabled)?;
        let used = spend.used.as_ref()?;
        let limit = spend.limit.as_ref();
        Some(credits_limit(
            spend.percent.or_else(|| {
                limit
                    .filter(|limit| limit.amount_minor > 0)
                    .map(|limit| used.amount() / limit.amount() * 100.0)
            })?,
            spend.severity.clone(),
            Credits {
                used: used.amount(),
                limit: limit.map(Money::amount),
                currency: used.currency.clone().unwrap_or_default(),
            },
        ))
    }

    fn extra_usage_credits(&self) -> Option<Limit> {
        let extra = self.extra_usage.as_ref().filter(|extra| extra.is_enabled)?;
        let scale = 10_f64.powi(extra.decimal_places.unwrap_or(2));
        Some(credits_limit(
            extra.utilization?,
            None,
            Credits {
                used: extra.used_credits? as f64 / scale,
                limit: extra.monthly_limit.map(|limit| limit as f64 / scale),
                currency: extra.currency.clone().unwrap_or_default(),
            },
        ))
    }
}

fn credits_limit(percent: f64, severity: Option<String>, credits: Credits) -> Limit {
    Limit {
        kind: "credits".to_string(),
        group: None,
        scope: None,
        percent,
        severity,
        resets_at: None,
        active: false,
        credits: Some(credits),
    }
}

impl RawLimit {
    fn limit(&self) -> Limit {
        Limit {
            kind: self.kind.clone(),
            group: self.group.clone(),
            scope: self.scope.as_ref().and_then(Scope::name),
            percent: self.percent,
            severity: self.severity.clone(),
            resets_at: self.resets_at.clone(),
            active: self.is_active,
            credits: None,
        }
    }
}

impl Scope {
    fn name(&self) -> Option<String> {
        let model = self
            .model
            .as_ref()
            .and_then(|model| model.display_name.clone().or_else(|| model.id.clone()));
        let surface = self.surface.as_ref().and_then(|surface| match surface {
            Value::String(name) => Some(name.clone()),
            Value::Object(fields) => ["display_name", "name", "id"]
                .iter()
                .find_map(|field| fields.get(*field)?.as_str().map(str::to_string)),
            _ => None,
        });
        match (model, surface) {
            (Some(model), Some(surface)) => Some(format!("{model}, {surface}")),
            (model, surface) => model.or(surface),
        }
    }
}
