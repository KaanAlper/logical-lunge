use std::{
  collections::HashMap,
  io::Cursor,
  path::{Path, PathBuf},
  sync::LazyLock,
  time::{Duration, Instant},
};

use rocket::{
  fs::NamedFile,
  http::{ContentType, Cookie, CookieJar, Header, SameSite, Status},
  request::{FromRequest, Outcome},
  response::{self, Redirect, Responder, Response},
  Request,
};
use tokio::{sync::Mutex, task};
use uuid::Uuid;

use crate::common::{glob_util, PathExt};

/// Port for the localhost asset server.
const ASSET_SERVER_PORT: u16 = 6124;

/// Map of tokens to their corresponding path and file patterns.
static ASSET_SERVER_TOKENS: LazyLock<Mutex<HashMap<String, TokenAccess>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));

/// Access information for a given token.
#[derive(Clone, Debug)]
struct TokenAccess {
  /// Base directory for the token.
  base_dir: PathBuf,

  /// File patterns for accessible files.
  file_patterns: Vec<String>,
}

fn build_asset_server() -> rocket::Rocket<rocket::Build> {
  rocket::build()
    .configure(
      rocket::Config::figment().merge(("port", ASSET_SERVER_PORT)),
    )
    .mount("/", routes![sw_js, normalize_css, init, instance, serve])
}

/// Logical Lunge: identifies the process serving the port (see
/// `setup_asset_server`).
#[get("/__zebar/instance")]
fn instance() -> String {
  std::process::id().to_string()
}

/// Whether this process's asset server answers on the port.
///
/// A connection alone isn't enough: during a restart the port can still be
/// served by the previous Zebar, which doesn't know this instance's widget
/// tokens (widgets then showed an error page and the bar stayed empty).
async fn is_own_server_up() -> bool {
  use tokio::io::{AsyncReadExt, AsyncWriteExt};

  let Ok(mut stream) =
    tokio::net::TcpStream::connect(("127.0.0.1", ASSET_SERVER_PORT)).await
  else {
    return false;
  };

  let request = "GET /__zebar/instance HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
  if stream.write_all(request.as_bytes()).await.is_err() {
    return false;
  }

  let mut response = String::new();
  let read = tokio::time::timeout(
    Duration::from_millis(500),
    stream.read_to_string(&mut response),
  )
  .await;

  matches!(read, Ok(Ok(_)))
    && response.starts_with("HTTP/1.1 200")
    && response
      .rsplit("\r\n\r\n")
      .next()
      .is_some_and(|body| body.trim() == std::process::id().to_string())
}

pub async fn setup_asset_server() -> anyhow::Result<()> {
  // Launch in the background and keep retrying if the port can't be bound
  // (e.g. it is still held by a previous Zebar instance that is shutting
  // down). Previously a failed bind was only logged and Zebar ran without
  // an asset server, so every widget showed a "connection refused" page.
  task::spawn(async move {
    let mut attempt: u32 = 0;
    loop {
      match build_asset_server().launch().await {
        Ok(_) => break,
        Err(err) => {
          attempt += 1;
          error!("Asset server failed to start (attempt {attempt}): {:?}", err);
          let delay = if attempt < 40 { 250 } else { 2000 };
          tokio::time::sleep(Duration::from_millis(delay)).await;
        }
      }
    }
  });

  // Don't open any widget before this process's server answers, so a
  // widget never loads a "connection refused" page or reaches the previous
  // instance's server.
  let deadline = Instant::now() + Duration::from_secs(30);
  while Instant::now() < deadline {
    if is_own_server_up().await {
      return Ok(());
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  warn!("Asset server is still not reachable; opening widgets anyway.");
  Ok(())
}

pub async fn create_init_url(
  parent_dir: &Path,
  html_path: &Path,
  file_patterns: Vec<String>,
) -> anyhow::Result<tauri::Url> {
  // Generate a unique token to identify requests from the widget to the
  // asset server.
  let token = upsert_or_get_token(parent_dir, file_patterns).await;

  let redirect = format!(
    "/{}",
    html_path.strip_prefix(parent_dir)?.to_unicode_string()
  );

  let url = tauri::Url::parse_with_params(
    &format!("http://127.0.0.1:{}/__zebar/init", ASSET_SERVER_PORT),
    &[("token", &token), ("redirect", &redirect)],
  )?;

  Ok(url)
}

/// Returns an asset server token for a given directory.
///
/// If the directory does not have an existing token, a new one is
/// generated and inserted.
async fn upsert_or_get_token(
  directory: &Path,
  file_patterns: Vec<String>,
) -> String {
  let mut asset_server_tokens = ASSET_SERVER_TOKENS.lock().await;

  // Find existing token for this path.
  let found_token = asset_server_tokens
    .iter()
    .find(|(_, token)| token.base_dir == directory)
    .map(|(token, _)| token.clone());

  if let Some(token) = found_token {
    // Update the file patterns for the existing token.
    if let Some(access) = asset_server_tokens.get_mut(&token) {
      access.file_patterns = file_patterns;
    }

    token
  } else {
    let new_token = Uuid::new_v4().to_string();

    asset_server_tokens.insert(
      new_token.clone(),
      TokenAccess {
        base_dir: directory.to_path_buf(),
        file_patterns,
      },
    );

    new_token
  }
}

#[get("/__zebar/init?<token>&<redirect>")]
pub fn init(
  token: String,
  redirect: String,
  cookies: &CookieJar<'_>,
) -> Redirect {
  // Create a http-only cookie with the widget's token.
  cookies.add(
    Cookie::build(("ZEBAR_TOKEN", token))
      .http_only(true)
      .same_site(SameSite::Strict)
      .path("/"),
  );

  Redirect::to(redirect)
}

#[get("/__zebar/sw.js")]
pub fn sw_js() -> SwResponse {
  SwResponse(include_str!("../resources/sw.js"))
}

#[derive(Debug)]
pub struct SwResponse(&'static str);

#[rocket::async_trait]
impl<'r> Responder<'r, 'static> for SwResponse {
  fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
    Response::build()
      .header(Header::new("Content-Type", "text/javascript"))
      .header(Header::new("Service-Worker-Allowed", "/"))
      .sized_body(self.0.len(), Cursor::new(self.0))
      .ok()
  }
}

#[get("/__zebar/normalize.css")]
pub fn normalize_css() -> (ContentType, &'static str) {
  (ContentType::CSS, include_str!("../resources/normalize.css"))
}

#[rocket::get("/<path..>", rank = 100)]
pub async fn serve(
  path: Option<PathBuf>,
  token: ServerToken,
) -> Option<NamedFile> {
  // Retrieve access information for the corresponding token.
  let token_access =
    { ASSET_SERVER_TOKENS.lock().await.get(&token.0).cloned() }?;

  let relative_path = path.unwrap_or("index.html".into());
  let absolute_path = token_access
    .base_dir
    .join(relative_path.clone())
    .canonicalize_pretty()
    .ok()?;

  // Allow access if:
  // - The asset path is within the base directory.
  // - The asset path matches any of the file patterns of the widget pack.
  if !absolute_path.starts_with(&token_access.base_dir)
    || !glob_util::is_match(&relative_path, &token_access.file_patterns)
      .ok()?
  {
    tracing::warn!(
      "Asset path {} is inaccessable with token {:?}.",
      absolute_path.display(),
      token_access
    );

    return None;
  }

  // Attempt to open and serve the requested file. Currently returns HTML
  // `Content-Type` if not found.
  NamedFile::open(absolute_path).await.ok()
}

/// Token for identifying which directory is being accessed.
#[derive(Debug)]
pub struct ServerToken(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ServerToken {
  type Error = anyhow::Error;

  async fn from_request(
    request: &'r Request<'_>,
  ) -> Outcome<Self, Self::Error> {
    let token = request.cookies().get("ZEBAR_TOKEN");

    match token {
      Some(token) => {
        Outcome::Success(ServerToken(token.value_trimmed().to_string()))
      }
      None => Outcome::Error((
        Status::Unauthorized,
        anyhow::anyhow!("Missing token for accessing directory."),
      )),
    }
  }
}
