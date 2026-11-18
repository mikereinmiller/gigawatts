use std::io::{self, Read as _, Write as _};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use clap::ValueEnum as _;

use crate::config::Config;
use crate::contract::{GroupBy, Level, Range};
use crate::data::{self, Days};
use crate::engine::{self, Paths};
use crate::store::{Filter, Store};

const PAGE: &str = include_str!("page.html");
const MAX_REQUEST_BYTES: usize = 8 * 1024;
const REQUEST_WITHIN: Duration = Duration::from_secs(5);
const ALLOWED_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "gigawatts.localhost"];
const POLICY: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
    connect-src 'self'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

pub fn bind(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
}

pub fn serve(listener: TcpListener, paths: Paths) -> io::Result<()> {
    let store = Store::open_read_only(&paths.database()).map_err(io::Error::other)?;
    std::thread::Builder::new()
        .name("report".to_string())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                answer(stream, &store, &paths);
            }
        })?;
    Ok(())
}

struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

struct Response {
    status: &'static str,
    content_type: &'static str,
    body: String,
}

impl Response {
    fn error(status: &'static str, message: &str) -> Response {
        Response {
            status,
            content_type: "application/json",
            body: serde_json::json!({ "error": message }).to_string(),
        }
    }
}

fn answer(mut stream: TcpStream, store: &Store, paths: &Paths) {
    let _ = stream.set_write_timeout(Some(REQUEST_WITHIN));
    let response = match read_request(&mut stream) {
        Ok(request) => respond(&request, store, paths),
        Err(Unread::TooLarge) => Response::error(
            "431 Request Header Fields Too Large",
            "the request is too large",
        ),
        Err(Unread::Malformed) => Response::error("400 Bad Request", "the request is malformed"),
        Err(Unread::Gone) => return,
    };
    let head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\n\
         Content-Security-Policy: {POLICY}\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(response.body.as_bytes());
}

enum Unread {
    TooLarge,
    Malformed,
    Gone,
}

fn read_request(stream: &mut TcpStream) -> Result<Request, Unread> {
    let deadline = Instant::now() + REQUEST_WITHIN;
    let mut raw: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if let Some(end) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        if raw.len() > MAX_REQUEST_BYTES {
            return Err(Unread::TooLarge);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || stream.set_read_timeout(Some(remaining)).is_err() {
            return Err(Unread::Gone);
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(Unread::Gone),
            Ok(read) => raw.extend_from_slice(&chunk[..read]),
        }
    };
    if head_end > MAX_REQUEST_BYTES {
        return Err(Unread::TooLarge);
    }
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| Unread::Malformed)?;
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(target), Some(_version), None) = (
        request_line.next(),
        request_line.next(),
        request_line.next(),
        request_line.next(),
    ) else {
        return Err(Unread::Malformed);
    };
    let headers = lines
        .map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
                .ok_or(Unread::Malformed)
        })
        .collect::<Result<_, _>>()?;
    Ok(Request {
        method: method.to_string(),
        target: target.to_string(),
        headers,
    })
}

fn respond(request: &Request, store: &Store, paths: &Paths) -> Response {
    if let Some(refusal) = refusal(request) {
        return Response::error("403 Forbidden", refusal);
    }
    if request.method != "GET" {
        return Response::error("405 Method Not Allowed", "the report only answers GET");
    }
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((&request.target, ""));
    match path {
        "/" => Response {
            status: "200 OK",
            content_type: "text/html",
            body: PAGE.to_string(),
        },
        "/api/data" => api_data(query, store, paths),
        "/api/series" => api_series(query, store),
        "/api/agents" => match parse_query(query) {
            Ok(asked) => json_answer(data::agent_models(store, &asked.days, &asked.scope)),
            Err(message) => Response::error("400 Bad Request", &message),
        },
        "/api/now" => api_now(store, paths),
        "/api/sessions" => match parse_query(query) {
            Ok(asked) => {
                let drill: Vec<&str> = asked.drill.iter().map(String::as_str).collect();
                json_answer(data::sessions(
                    store,
                    &asked.days,
                    &asked.scope,
                    asked.group_by,
                    &drill,
                    asked.limit,
                ))
            }
            Err(message) => Response::error("400 Bad Request", &message),
        },
        "/api/mix" => api_mix(query, store, paths),
        "/api/advice" => json_answer(data::advice(store)),
        "/api/insights" => {
            json_answer(crate::insights::answer(store, None, None).map(|insights| {
                crate::contract::InsightsPage {
                    takeaways: crate::insights::takeaways(&insights),
                    insights,
                }
            }))
        }
        "/api/limits" => match parse_query(query) {
            Ok(asked) => json_answer(data::limit_history(store, &asked.days)),
            Err(message) => Response::error("400 Bad Request", &message),
        },
        _ => Response::error("404 Not Found", "no such page"),
    }
}

fn refusal(request: &Request) -> Option<&'static str> {
    let host_allowed = request.header("host").is_some_and(is_this_machine);
    if !host_allowed {
        return Some("the report answers only requests addressed to localhost");
    }
    let origin_allowed = request
        .header("origin")
        .is_none_or(|origin| origin.strip_prefix("http://").is_some_and(is_this_machine));
    let fetched_from_here = request
        .header("sec-fetch-site")
        .is_none_or(|site| site == "same-origin" || site == "none");
    if !origin_allowed || !fetched_from_here {
        return Some("the report answers only its own page");
    }
    None
}

fn is_this_machine(host_and_port: &str) -> bool {
    let host = match host_and_port.rsplit_once(':') {
        Some((host, port)) if port.parse::<u16>().is_ok() => host,
        Some(_) => return false,
        None => host_and_port,
    };
    ALLOWED_HOSTS
        .iter()
        .any(|allowed| host.eq_ignore_ascii_case(allowed))
}

struct Asked {
    days: Days,
    scope: Vec<Filter>,
    group_by: GroupBy,
    drill: Vec<String>,
    top: usize,
    limit: usize,
}

const DEFAULT_TOP: usize = 8;
const DEFAULT_SESSIONS: usize = 10;
const MOST_SESSIONS: usize = 1000;
const MOST_TOP: usize = 20;

fn parse_query(query: &str) -> Result<Asked, String> {
    let mut range = None;
    let mut since = None;
    let mut until = None;
    let mut group_by = GroupBy::Model;
    let mut drill = Vec::new();
    let mut scope = Vec::new();
    let mut top = DEFAULT_TOP;
    let mut limit = DEFAULT_SESSIONS;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decoded(value)?;
        match key {
            "range" => {
                range = Some(Range::from_str(&value, false).map_err(|_| {
                    format!("no range named {value:?}. Use today, week, month, or all")
                })?);
            }
            "by" => {
                group_by = GroupBy::from_str(&value, false).map_err(|_| {
                    format!("no grouping named {value:?}. Use repo, branch, skill, agent, or model")
                })?;
            }
            "since" => since = Some(data::local_date(&value)?),
            "until" => until = Some(data::local_date(&value)?),
            "drill" => drill.push(value),
            "repo" | "branch" | "skill" | "model" | "agent" => scope.push(Filter {
                level: scope_level(key),
                key: value,
            }),
            "limit" => {
                limit = value
                    .parse()
                    .ok()
                    .filter(|limit| (1..=MOST_SESSIONS).contains(limit))
                    .ok_or_else(|| format!("limit must be a number from 1 to {MOST_SESSIONS}"))?;
            }
            "top" => {
                top = value
                    .parse()
                    .ok()
                    .filter(|top| (1..=MOST_TOP).contains(top))
                    .ok_or_else(|| format!("top must be a number from 1 to {MOST_TOP}"))?;
            }
            _ => {}
        }
    }
    let days = match (range, since, until) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            return Err("give either range or since and until, not both".to_string());
        }
        (Some(range), None, None) => Days::Range(range),
        (None, None, None) => Days::Range(Range::Today),
        (None, Some(since), Some(until)) if until < since => {
            return Err(format!("until {until} is before since {since}"));
        }
        (None, since, until) => Days::Between(since, until),
    };
    Ok(Asked {
        days,
        scope,
        group_by,
        drill,
        top,
        limit,
    })
}

fn scope_level(name: &str) -> Level {
    match name {
        "repo" => Level::Repo,
        "branch" => Level::RepoBranch,
        "skill" => Level::Skill,
        "model" => Level::Model,
        _ => Level::Agent,
    }
}

fn percent_decoded(value: &str) -> Result<String, String> {
    let malformed = || format!("{value:?} isn't encoded correctly");
    let mut bytes = Vec::with_capacity(value.len());
    let mut rest = value.bytes();
    while let Some(byte) = rest.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = rest.next().and_then(|digit| (digit as char).to_digit(16));
                let low = rest.next().and_then(|digit| (digit as char).to_digit(16));
                let (Some(high), Some(low)) = (high, low) else {
                    return Err(malformed());
                };
                bytes.push((high * 16 + low) as u8);
            }
            byte => bytes.push(byte),
        }
    }
    String::from_utf8(bytes).map_err(|_| malformed())
}

fn json_answer(answer: Result<impl serde::Serialize, data::Error>) -> Response {
    match answer {
        Ok(response) => Response {
            status: "200 OK",
            content_type: "application/json",
            body: serde_json::to_string(&response).expect("contract types always serialize"),
        },
        Err(data::Error::Store(error)) => Response::error(
            "500 Internal Server Error",
            &format!("can't query the history database: {error}"),
        ),
        Err(error) => Response::error("400 Bad Request", &error.to_string()),
    }
}

fn api_data(query: &str, store: &Store, paths: &Paths) -> Response {
    let asked = match parse_query(query) {
        Ok(asked) => asked,
        Err(message) => return Response::error("400 Bad Request", &message),
    };
    let config = match load_config(paths) {
        Ok(config) => config,
        Err(response) => return response,
    };
    let drill: Vec<&str> = asked.drill.iter().map(String::as_str).collect();
    json_answer(data::answer(
        store,
        &config,
        &asked.days,
        &asked.scope,
        asked.group_by,
        &drill,
        engine::running_problems(paths),
    ))
}

fn load_config(paths: &Paths) -> Result<Config, Response> {
    Config::load(&paths.config(), &paths.home).map_err(|error| {
        Response::error(
            "500 Internal Server Error",
            &format!("can't read the config: {error}"),
        )
    })
}

fn api_mix(query: &str, store: &Store, paths: &Paths) -> Response {
    let asked = match parse_query(query) {
        Ok(asked) => asked,
        Err(message) => return Response::error("400 Bad Request", &message),
    };
    let config = match load_config(paths) {
        Ok(config) => config,
        Err(response) => return response,
    };
    let drill: Vec<&str> = asked.drill.iter().map(String::as_str).collect();
    json_answer(data::mix(
        store,
        &config,
        &asked.days,
        &asked.scope,
        asked.group_by,
        &drill,
    ))
}

fn api_now(store: &Store, paths: &Paths) -> Response {
    let config = match load_config(paths) {
        Ok(config) => config,
        Err(response) => return response,
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64);
    json_answer(engine::snapshot::build(
        store,
        &config,
        now_ms,
        &engine::running_problems(paths),
    ))
}

fn api_series(query: &str, store: &Store) -> Response {
    let asked = match parse_query(query) {
        Ok(asked) => asked,
        Err(message) => return Response::error("400 Bad Request", &message),
    };
    json_answer(data::series(
        store,
        &asked.days,
        &asked.scope,
        asked.group_by.levels()[0],
        asked.top,
    ))
}
