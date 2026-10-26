//! YouTube Data API v3: one-time OAuth, then private draft uploads.
//!
//! Uploads are ALWAYS private. A person watches the draft in YouTube Studio and
//! publishes it there. That review step is deliberate: it is your quality gate and
//! the human judgement YouTube's monetisation review looks for.
//!
//! Credentials live in .secrets/youtube.json (gitignored):
//!   { "client_id": "...", "client_secret": "...", "refresh_token": "..." }
//! `forge auth` fills in refresh_token. In CI, set YT_CLIENT_ID / YT_CLIENT_SECRET /
//! YT_REFRESH_TOKEN as environment variables instead (GitHub Actions secrets).
//!
//! NOTE: until your Google Cloud project passes YouTube's API audit, videos uploaded
//! through the API are locked to private. That's fine for this flow, but you must
//! pass the audit before the channel can publish what forge uploads. Apply early.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

const SCOPES: &str = "https://www.googleapis.com/auth/youtube.upload https://www.googleapis.com/auth/youtube.force-ssl";

#[derive(Serialize, Deserialize, Default)]
pub struct Secrets {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub refresh_token: String,
}

pub fn secrets_path(root: &Path) -> PathBuf {
    root.join(".secrets").join("youtube.json")
}

fn load_secrets(root: &Path) -> Result<Secrets> {
    // CI: credentials come from environment variables (GitHub Actions secrets).
    if let (Ok(id), Ok(secret)) = (std::env::var("YT_CLIENT_ID"), std::env::var("YT_CLIENT_SECRET")) {
        return Ok(Secrets { client_id: id, client_secret: secret, refresh_token: std::env::var("YT_REFRESH_TOKEN").unwrap_or_default() });
    }
    let p = secrets_path(root);
    let s: Secrets = serde_json::from_str(&fs::read_to_string(&p).with_context(|| {
        format!("{} missing. Create an OAuth client (Desktop app) in Google Cloud Console and save client_id + client_secret there.", p.display())
    })?)?;
    Ok(s)
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn api_err(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, resp) => anyhow!("HTTP {code}: {}", resp.into_string().unwrap_or_default()),
        other => anyhow!(other),
    }
}

/// Loopback OAuth flow for a Desktop OAuth client. Sign in with the Google account that
/// manages the channel; on the consent screen pick the channel (Brand Account) itself.
pub fn auth(root: &Path) -> Result<()> {
    let mut s = load_secrets(root)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        enc(&s.client_id), enc(&redirect), enc(SCOPES)
    );
    println!("Open this URL in your browser and approve access:\n\n{url}\n\nWaiting for the redirect on {redirect} ...");

    let (mut stream, _) = listener.accept()?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let code = line
        .split_whitespace()
        .nth(1)
        .and_then(|path| path.split_once('?'))
        .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("code=")))
        .map(dec)
        .ok_or_else(|| anyhow!("no ?code= in redirect: {line}"))?;
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nforge: authorised. You can close this tab.\n")?;

    let resp: Value = ureq::post("https://oauth2.googleapis.com/token")
        .send_form(&[
            ("code", code.as_str()),
            ("client_id", s.client_id.as_str()),
            ("client_secret", s.client_secret.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .map_err(api_err)?
        .into_json()?;
    s.refresh_token = resp["refresh_token"].as_str().ok_or_else(|| anyhow!("no refresh_token in {resp}"))?.to_string();
    fs::write(secrets_path(root), serde_json::to_string_pretty(&s)?)?;
    println!("Saved refresh token to {}", secrets_path(root).display());
    Ok(())
}

fn access_token(s: &Secrets) -> Result<String> {
    if s.refresh_token.is_empty() {
        bail!("no refresh_token yet: run `forge auth` first");
    }
    let resp: Value = ureq::post("https://oauth2.googleapis.com/token")
        .send_form(&[
            ("client_id", s.client_id.as_str()),
            ("client_secret", s.client_secret.as_str()),
            ("refresh_token", s.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .map_err(api_err)?
        .into_json()?;
    Ok(resp["access_token"].as_str().ok_or_else(|| anyhow!("no access_token in {resp}"))?.to_string())
}

pub struct UploadInput<'a> {
    pub video: &'a Path,
    pub srt: Option<&'a Path>,
    pub thumbnail: Option<&'a Path>,
    pub metadata: &'a Value,
}

/// Resumable upload as a PRIVATE video, then captions and thumbnail. Returns the video id.
pub fn upload(root: &Path, input: UploadInput) -> Result<String> {
    let s = load_secrets(root)?;
    let token = access_token(&s)?;
    let bearer = format!("Bearer {token}");
    let m = input.metadata;
    let size = fs::metadata(input.video)?.len();

    let body = json!({
        "snippet": {
            "title": m["title"], "description": m["description"], "tags": m["tags"],
            "categoryId": m["categoryId"], "defaultLanguage": "en", "defaultAudioLanguage": "en"
        },
        "status": {
            "privacyStatus": "private",
            "selfDeclaredMadeForKids": m["madeForKids"],
            "containsSyntheticMedia": m["containsSyntheticMedia"]
        }
    });
    let init = ureq::post("https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status")
        .set("Authorization", &bearer)
        .set("X-Upload-Content-Type", "video/mp4")
        .set("X-Upload-Content-Length", &size.to_string())
        .send_json(body)
        .map_err(api_err)
        .context("starting upload")?;
    let session = init.header("Location").ok_or_else(|| anyhow!("no upload session URL"))?.to_string();

    println!("[upload] sending {:.1} MB ...", size as f64 / 1e6);
    let resp: Value = ureq::put(&session)
        .set("Authorization", &bearer)
        .set("Content-Type", "video/mp4")
        .send(fs::File::open(input.video)?)
        .map_err(api_err)
        .context("uploading video bytes")?
        .into_json()?;
    let id = resp["id"].as_str().ok_or_else(|| anyhow!("no video id in {resp}"))?.to_string();
    println!("[upload] video {id} uploaded (private)");

    if let Some(srt) = input.srt {
        let boundary = "forge-boundary-7d1e";
        let meta = json!({"snippet": {"videoId": id, "language": "en", "name": "English", "isDraft": false}});
        let mut body = Vec::new();
        write!(body, "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n")?;
        write!(body, "--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n")?;
        body.extend(fs::read(srt)?);
        write!(body, "\r\n--{boundary}--\r\n")?;
        match ureq::post("https://www.googleapis.com/upload/youtube/v3/captions?uploadType=multipart&part=snippet")
            .set("Authorization", &bearer)
            .set("Content-Type", &format!("multipart/related; boundary={boundary}"))
            .send_bytes(&body)
        {
            Ok(_) => println!("[upload] captions attached"),
            Err(e) => eprintln!("[upload] captions failed (upload the .srt in Studio): {}", api_err(e)),
        }
    }

    if let Some(thumb) = input.thumbnail {
        match ureq::post(&format!("https://www.googleapis.com/upload/youtube/v3/thumbnails/set?videoId={id}"))
            .set("Authorization", &bearer)
            .set("Content-Type", "image/jpeg")
            .send_bytes(&fs::read(thumb)?)
        {
            Ok(_) => println!("[upload] thumbnail set"),
            Err(e) => eprintln!("[upload] thumbnail failed (custom thumbnails need a verified channel): {}", api_err(e)),
        }
    }
    Ok(id)
}
