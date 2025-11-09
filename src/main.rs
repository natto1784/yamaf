use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};

use rand::{Rng, distr::Alphanumeric};
use std::{env, net::SocketAddr, sync::LazyLock};
use std::{env::VarError, path::PathBuf};
use tokio::fs::File;
use tokio_util::io::ReaderStream;

struct Config {
    root_dir: String,
    key: Result<String, VarError>,
    title: String,
    internal_host: String,
    internal_port: u16,
    external_host: String,
    external_protocol: &'static str,
    max_filesize: usize,
    max_bodysize: usize,
}

static CONFIG: LazyLock<Config> = LazyLock::new(|| {
    let root_dir = env::var("ROOT_DIR").unwrap_or_else(|_| "/var/files".to_string());
    let key = env::var("KEY");
    let title = env::var("TITLE").unwrap_or_else(|_| "Simpler Filehost".to_string());

    let internal_host = env::var("INTERNAL_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let internal_port = env::var("INTERNAL_PORT")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(8000);

    let external_host = env::var("EXTERNAL_HOST").unwrap_or_else(|_| internal_host.clone());
    let external_protocol = if env::var("EXTERNAL_HAS_TLS").is_ok() {
        "https"
    } else {
        "http"
    };

    let max_files = env::var("MAX_FILES")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(10);
    let max_filesize = env::var("MAX_FILESIZE_MB")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(100)
        << 20;
    let max_bodysize = max_files * max_filesize * 2;

    Config {
        root_dir,
        key,
        title,
        internal_host,
        internal_port,
        external_host,
        external_protocol,
        max_filesize,
        max_bodysize,
    }
});

#[tokio::main]
async fn main() {
    let addr = SocketAddr::new(
        CONFIG.internal_host.parse().expect("Invalid Host Bind"),
        CONFIG.internal_port,
    );

    let app = Router::new()
        .route("/", get(index).post(upload))
        .route("/{filename}", get(serve_file))
        .layer(DefaultBodyLimit::max(CONFIG.max_bodysize));

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    println!(
        "Starting server on {} for directory {}",
        addr, CONFIG.root_dir
    );

    axum::serve(listener, app).await.unwrap();
}

static INDEX_HTML: LazyLock<String> = LazyLock::new(|| {
    let mut html = include_str!("./index.html").to_string();

    html = html.replace("{{TITLE}}", &CONFIG.title);

    html = html.replace(
        "{{USER_URL}}",
        &format!("{}://{}", CONFIG.external_protocol, CONFIG.external_host),
    );

    html = html.replace(
        "{{KEY_FIELD}}",
        if CONFIG.key.is_ok() {
            r#"<input type="password" name="key" placeholder="Upload Key" required><br><br>"#
        } else {
            ""
        },
    );

    html
});

async fn index() -> Html<&'static str> {
    Html(&INDEX_HTML)
}

#[derive(Debug)]
enum YamafError {
    BadRequest(String),
    InternalError(String),
    FileTooBig(String),
    FileNotFound,
}

impl IntoResponse for YamafError {
    fn into_response(self) -> Response {
        match self {
            YamafError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            YamafError::InternalError(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
            YamafError::FileTooBig(filename) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("File {} is too big!", filename),
            )
                .into_response(),
            YamafError::FileNotFound => (StatusCode::NOT_FOUND, "File Not Found!").into_response(),
        }
    }
}

fn random(len: usize) -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn clean_filename(filename: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;

    for c in filename.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            prev_dash = false;
        } else if c == '.' {
            slug.push(c);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }

    slug.trim_matches('-').to_string()
}

async fn upload(mut payload: Multipart) -> Result<impl IntoResponse, YamafError> {
    let mut responses = Vec::new();
    let mut found_key = false;

    while let Some(mut field) = payload.next_field().await.unwrap() {
        match field.name() {
            Some("key") => {
                if let Ok(ref key) = CONFIG.key {
                    let bytes = field
                        .bytes()
                        .await
                        .map_err(|e| YamafError::BadRequest(format!("Error reading key: {e}")))?;

                    let s = String::from_utf8(bytes.to_vec())
                        .map_err(|_| YamafError::InternalError("Invalid key format".into()))?;

                    if s != *key {
                        return Err(YamafError::BadRequest("Wrong key".into()));
                    }

                    found_key = true;
                }
            }

            Some("file") => {
                if CONFIG.key.is_ok() && found_key == false {
                    return Err(YamafError::BadRequest("Missing key".into()));
                }

                let filename = field
                    .file_name()
                    .map_or(format!("{}-upload", random(10)), |filename| {
                        format!("{}-{}", random(4), clean_filename(filename))
                    });

                let save_path = std::path::Path::new(&CONFIG.root_dir).join(&filename);

                let mut file = File::create(&save_path)
                    .await
                    .map_err(|_| YamafError::InternalError("Internal i/o error".into()))?;

                let mut written: usize = 0;

                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|err| YamafError::InternalError(err.to_string()))?
                {
                    use tokio::io::AsyncWriteExt;

                    written = written
                        .checked_add(chunk.len())
                        .ok_or_else(|| YamafError::BadRequest("File too large".into()))?;

                    if written > CONFIG.max_filesize {
                        _ = tokio::fs::remove_file(&save_path).await;

                        return Err(YamafError::FileTooBig(filename));
                    }

                    file.write_all(&chunk)
                        .await
                        .map_err(|_| YamafError::InternalError("Internal i/o error".into()))?;
                }

                responses.push(format!(
                    r#"<a href="{proto}://{host}/{file}">{proto}://{host}/{file}</a> (size ~ {size:.2}k)"#,
                    proto = CONFIG.external_protocol,
                    host = CONFIG.external_host,
                    file = filename,
                    size = written as f64 / 1024 as f64
                ));
            }

            None | Some(_) => {}
        }
    }

    if responses.is_empty() {
        return Err(YamafError::BadRequest("No files uploaded".into()));
    }

    Ok(Html(format!(
        "Here are your file(s):<br>{}",
        responses.join("<br>")
    ))
    .into_response())
}

async fn serve_file(Path(filename): Path<String>) -> Result<impl IntoResponse, YamafError> {
    let path = PathBuf::from(&CONFIG.root_dir).join(&filename);

    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|_| YamafError::FileNotFound)?;
    let file = File::open(&path)
        .await
        .map_err(|_| YamafError::FileNotFound)?;
    let mime = mime_guess::from_path(&path).first_or_octet_stream();

    let content_type = mime
        .as_ref()
        .parse()
        .map_err(|_| YamafError::InternalError("Something went wrong".into()))?;
    let content_length = metadata
        .len()
        .to_string()
        .parse()
        .map_err(|_| YamafError::InternalError("Something went wrong".into()))?;

    let headers = HeaderMap::from_iter([
        (header::CONTENT_TYPE, content_type),
        (header::CONTENT_LENGTH, content_length),
        (header::ACCEPT_RANGES, "bytes".parse().unwrap()),
    ]);

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok((StatusCode::OK, headers, body).into_response())
}
