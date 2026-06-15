use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use qrcodegen::{QrCode, QrCodeEcc};

const COPY_BUFFER_SIZE: usize = 256 * 1024;
const WORKER_STACK_SIZE: usize = 512 * 1024;
const MAX_WORKERS: usize = 256;
const MAX_QUEUE_SIZE: usize = 4096;
const HEADER_TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
const KEEP_ALIVE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const CHUNK_SIZE: u64 = 1024 * 1024; // 1 MB upload chunk
const UPLOAD_EXPIRY_SECS: u64 = 86400; // 24 hours

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static ACTIVE_REQUESTS: AtomicUsize = AtomicUsize::new(0);
static UPLOADS_ENABLED: AtomicBool = AtomicBool::new(true);

thread_local! {
    static COPY_BUF: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(vec![0u8; COPY_BUFFER_SIZE]);
}

#[derive(Clone)]
struct Config {
    root: PathBuf,
    port: u16,
    bind: IpAddr,
    uploads_enabled: bool,
    workers: usize,
    queue_size: usize,
}

struct Request {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    /// Body bytes already consumed while reading headers (for POST/PUT bodies)
    body_prefix: Vec<u8>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    register_shutdown_handler();
    let config = parse_args()?;
    UPLOADS_ENABLED.store(config.uploads_enabled, Ordering::SeqCst);
    let (listener, selected_port) = bind_listener(config.bind, config.port)?;
    let root = fs::canonicalize(&config.root)?;

    println!();
    println!("Resource root: {}", display_path(&root));
    println!("Port: {selected_port}");
    println!("Bind address: {}", config.bind);
    println!(
        "Uploads: {}",
        if config.uploads_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!("Workers: {}", config.workers);
    println!("Queue size: {}", config.queue_size);
    println!();
    println!("PC local:          http://127.0.0.1:{selected_port}/");

    if config.bind.is_unspecified() {
        let lan_addresses = get_lan_addresses();
        if !lan_addresses.is_empty() {
            println!("Phone/LAN URLs:");
            for (ip, alias) in lan_addresses {
                println!("  http://{ip}:{selected_port}/  ({alias})");
            }
        }
    } else {
        println!("Phone/LAN URLs: disabled by bind address");
    }

    println!("Android emulator:  http://10.0.2.2:{selected_port}/");
    println!();
    println!("Resource manager:  http://127.0.0.1:{selected_port}/");
    println!("Example JSON:      http://127.0.0.1:{selected_port}/data/manifest.json");
    println!("Example image:     http://127.0.0.1:{selected_port}/images/harbor-dashboard.svg");
    println!("Videos folder:     http://127.0.0.1:{selected_port}/videos/");
    println!();
    println!("Rust static server. No Node.js, Python, npm packages, or manual dependency install required.");
    println!("Keep this window open while using the server. Press Ctrl+C to stop.");
    println!();

    if config.uploads_enabled {
        cleanup_expired_uploads(&root);
    }

    let (sender, receiver) = mpsc::sync_channel::<TcpStream>(config.queue_size);
    let receiver = Arc::new(Mutex::new(receiver));

    for worker_id in 0..config.workers {
        let receiver = Arc::clone(&receiver);
        let root = root.clone();
        thread::Builder::new()
            .name(format!("static-dock-worker-{worker_id}"))
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || worker_loop(worker_id, receiver, root, config.uploads_enabled))
            .map_err(io::Error::other)?;
    }

    listener.set_nonblocking(true).ok();
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            println!("\nShutting down gracefully...");
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).ok(); // accepted stream inherits non-blocking on Windows
                stream.set_nodelay(true).ok();
                match sender.try_send(stream) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(_stream)) => {
                        eprintln!("Request queue is full; dropping connection");
                    }
                    Err(mpsc::TrySendError::Disconnected(_stream)) => break,
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(err) => eprintln!("Connection failed: {err}"),
        }
    }

    drop(sender);
    let wait_start = Instant::now();
    while ACTIVE_REQUESTS.load(Ordering::SeqCst) > 0
        && wait_start.elapsed() < Duration::from_secs(30)
    {
        thread::sleep(Duration::from_millis(100));
    }
    println!("All requests completed. Goodbye.");
    Ok(())
}

fn worker_loop(
    worker_id: usize,
    receiver: Arc<Mutex<mpsc::Receiver<TcpStream>>>,
    root: PathBuf,
    uploads_enabled: bool,
) {
    loop {
        let stream = {
            let guard = match receiver.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    eprintln!("Worker {worker_id} stopped: request queue lock was poisoned");
                    return;
                }
            };
            guard.recv()
        };

        match stream {
            Ok(mut stream) => {
                ACTIVE_REQUESTS.fetch_add(1, Ordering::SeqCst);
                COPY_BUF.with(|buf_cell| {
                    let mut buf = buf_cell.borrow_mut();
                    if let Err(err) =
                        handle_connection(&mut stream, &root, uploads_enabled, &mut buf)
                    {
                        if !is_benign_error(&err) {
                            eprintln!("Worker {worker_id} request failed: {err}");
                        }
                    }
                });
                ACTIVE_REQUESTS.fetch_sub(1, Ordering::SeqCst);
            }
            Err(_) => return,
        }
    }
}

fn is_benign_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::TimedOut
            | io::ErrorKind::WouldBlock
    )
}

fn handle_connection(
    stream: &mut TcpStream,
    root: &Path,
    uploads_enabled: bool,
    buf: &mut [u8],
) -> io::Result<()> {
    let canonical_root = root.to_path_buf();

    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            break;
        }

        stream.set_read_timeout(Some(KEEP_ALIVE_IDLE_TIMEOUT)).ok();
        stream
            .set_write_timeout(Some(Duration::from_secs(120)))
            .ok();

        let request = match read_request(stream, HEADER_TOTAL_TIMEOUT)? {
            Some(request) => request,
            None => return Ok(()),
        };

        let wants_keepalive = request
            .headers
            .get("connection")
            .map(|v| !v.to_ascii_lowercase().contains("close"))
            .unwrap_or(true);

        let head_only = request.method == "HEAD";
        let normalized_target = normalize_request_target(&request.target);

        let keep_alive = wants_keepalive;

        if request.method == "OPTIONS" {
            send_headers_with_uploads(
                stream,
                204,
                "No Content",
                &[("Content-Length", "0")],
                keep_alive,
                uploads_enabled,
            )?;
            if !keep_alive {
                return Ok(());
            }
            continue;
        }

        // Upload API (POST allowed)
        if normalized_target.starts_with("/__staticdock/api/upload/") {
            if uploads_enabled {
                handle_upload_api(stream, root, &normalized_target, &request, buf, keep_alive)?;
            } else {
                send_text(
                    stream,
                    403,
                    "Forbidden",
                    "{\"error\":\"uploads disabled\"}",
                    "application/json; charset=utf-8",
                    head_only,
                    false,
                )?;
                return Ok(());
            }
            if !keep_alive {
                return Ok(());
            }
            continue;
        }

        if request.method != "GET" && request.method != "HEAD" {
            send_text_with_headers(
                stream,
                405,
                "Method Not Allowed",
                "Method not allowed.",
                "text/plain; charset=utf-8",
                head_only,
                &[allow_methods_header(uploads_enabled)],
                keep_alive,
            )?;
            if !keep_alive {
                return Ok(());
            }
            continue;
        }

        if normalized_target.starts_with("/__staticdock/api/list") {
            send_api_list(stream, root, &normalized_target, head_only, keep_alive)?;
            if !keep_alive {
                return Ok(());
            }
            continue;
        }

        if normalized_target.starts_with("/__staticdock/api/info") {
            send_api_info(stream, root, head_only, keep_alive)?;
            if !keep_alive {
                return Ok(());
            }
            continue;
        }

        if normalized_target.starts_with("/__staticdock/api/qr") {
            send_api_qr(stream, &normalized_target, head_only, keep_alive)?;
            if !keep_alive {
                return Ok(());
            }
            continue;
        }

        let path = match resolve_request_path(root, &normalized_target) {
            Some(path) => path,
            None => {
                send_text(
                    stream,
                    403,
                    "Forbidden",
                    "Forbidden.",
                    "text/plain; charset=utf-8",
                    head_only,
                    keep_alive,
                )?;
                if !keep_alive {
                    return Ok(());
                }
                continue;
            }
        };

        if !path.exists() {
            send_text(
                stream,
                404,
                "Not Found",
                "Not found.",
                "text/plain; charset=utf-8",
                head_only,
                keep_alive,
            )?;
            if !keep_alive {
                return Ok(());
            }
            continue;
        }

        let path = match ensure_inside_root(&canonical_root, path) {
            Some(path) => path,
            None => {
                send_text(
                    stream,
                    403,
                    "Forbidden",
                    "Forbidden.",
                    "text/plain; charset=utf-8",
                    head_only,
                    keep_alive,
                )?;
                if !keep_alive {
                    return Ok(());
                }
                continue;
            }
        };

        if path.is_dir() {
            let index = path.join("index.html");
            if index.is_file() {
                send_file(stream, &index, &request.headers, head_only, keep_alive, buf)?;
            } else {
                let html = directory_listing(&path, &normalized_target)?;
                send_text(
                    stream,
                    200,
                    "OK",
                    &html,
                    "text/html; charset=utf-8",
                    head_only,
                    keep_alive,
                )?;
            }
            if !keep_alive {
                return Ok(());
            }
            continue;
        }

        if !path.is_file() {
            send_text(
                stream,
                404,
                "Not Found",
                "Not found.",
                "text/plain; charset=utf-8",
                head_only,
                keep_alive,
            )?;
            if !keep_alive {
                return Ok(());
            }
            continue;
        }

        send_file(stream, &path, &request.headers, head_only, keep_alive, buf)?;
        if !keep_alive {
            return Ok(());
        }
    }
    Ok(())
}

fn parse_args() -> io::Result<Config> {
    parse_args_from(env::args().skip(1))
}

fn parse_args_from<I, S>(args: I) -> io::Result<Config>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut root: Option<PathBuf> = None;
    let mut port = 8787u16;
    let mut bind = IpAddr::from([0, 0, 0, 0]);
    let mut uploads_enabled = true;
    let mut workers = default_worker_count();
    let mut queue_size: Option<usize> = None;
    let mut args = args.into_iter().map(Into::into);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-p" | "--port" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "missing value for --port")
                })?;
                port = parse_port(&value)?;
            }
            "--root" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "missing value for --root")
                })?;
                root = Some(PathBuf::from(value));
            }
            "--bind" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "missing value for --bind")
                })?;
                bind = parse_bind_addr(&value)?;
            }
            "--no-upload" | "--no-uploads" => {
                uploads_enabled = false;
            }
            "--uploads" => {
                uploads_enabled = true;
            }
            "--workers" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "missing value for --workers")
                })?;
                workers = parse_limited_usize(&value, "workers", MAX_WORKERS)?;
            }
            "--queue" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "missing value for --queue")
                })?;
                queue_size = Some(parse_limited_usize(&value, "queue", MAX_QUEUE_SIZE)?);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ if arg.starts_with("--port=") => {
                port = parse_port(&arg["--port=".len()..])?;
            }
            _ if arg.starts_with("--root=") => {
                root = Some(PathBuf::from(&arg["--root=".len()..]));
            }
            _ if arg.starts_with("--bind=") => {
                bind = parse_bind_addr(&arg["--bind=".len()..])?;
            }
            _ if arg.starts_with("--workers=") => {
                workers = parse_limited_usize(&arg["--workers=".len()..], "workers", MAX_WORKERS)?;
            }
            _ if arg.starts_with("--queue=") => {
                queue_size = Some(parse_limited_usize(
                    &arg["--queue=".len()..],
                    "queue",
                    MAX_QUEUE_SIZE,
                )?);
            }
            _ => {
                root = Some(PathBuf::from(arg));
            }
        }
    }

    let root = match root {
        Some(path) => path,
        None => env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
    };

    let queue_size = queue_size
        .unwrap_or_else(|| workers.saturating_mul(16).max(256))
        .min(MAX_QUEUE_SIZE);

    Ok(Config {
        root,
        port,
        bind,
        uploads_enabled,
        workers: workers.min(MAX_WORKERS),
        queue_size,
    })
}

fn parse_port(value: &str) -> io::Result<u16> {
    value.parse::<u16>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid port: {value}"),
        )
    })
}

fn parse_bind_addr(value: &str) -> io::Result<IpAddr> {
    value.parse::<IpAddr>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid bind address: {value}"),
        )
    })
}

fn parse_limited_usize(value: &str, name: &str, max: usize) -> io::Result<usize> {
    let parsed = value.parse::<usize>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {name}: {value}"),
        )
    })?;

    if parsed == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be greater than zero"),
        ));
    }

    if parsed > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be less than or equal to {max}"),
        ));
    }

    Ok(parsed)
}

fn default_worker_count() -> usize {
    let cpus = thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4);
    (cpus * 16).clamp(64, 128)
}

fn print_help() {
    println!("Usage:");
    println!("  static-dock.exe [--root PATH] [--port PORT] [--bind ADDR] [--no-upload|--uploads] [--workers N] [--queue N]");
    println!("  static-dock.exe PATH -p 8787 --bind 127.0.0.1 --no-upload");
}

fn bind_listener(bind_addr: IpAddr, preferred_port: u16) -> io::Result<(TcpListener, u16)> {
    let candidates = [
        preferred_port,
        8787,
        8000,
        8080,
        8010,
        8020,
        3000,
        5173,
        9000,
    ];
    let mut tried = HashSet::new();

    for port in candidates {
        if !tried.insert(port) {
            continue;
        }

        match TcpListener::bind((bind_addr, port)) {
            Ok(listener) => {
                let selected_port = listener.local_addr()?.port();
                return Ok((listener, selected_port));
            }
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => continue,
            Err(err) => return Err(err),
        }
    }

    let listener = TcpListener::bind((bind_addr, 0))?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

fn allow_methods_header(uploads_enabled: bool) -> (&'static str, &'static str) {
    if uploads_enabled {
        ("Allow", "GET, HEAD, OPTIONS, POST")
    } else {
        ("Allow", "GET, HEAD, OPTIONS")
    }
}

fn read_request(stream: &mut TcpStream, total_timeout: Duration) -> io::Result<Option<Request>> {
    let deadline = Instant::now() + total_timeout;
    let mut buffer = Vec::with_capacity(8192);
    let mut chunk = [0u8; 1024];

    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "request header timed out",
            ));
        }
        let read = match stream.read(&mut chunk) {
            Ok(read) => read,
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(err) => return Err(err),
        };
        if read == 0 {
            if buffer.is_empty() {
                return Ok(None);
            }
            break;
        }

        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 65536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request header is too large",
            ));
        }

        if header_end(&buffer).is_some() {
            break;
        }
    }

    let end = header_end(&buffer)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad request header"))?;
    let text = String::from_utf8_lossy(&buffer[..end]);
    let body_prefix = if end + 4 < buffer.len() {
        buffer[end + 4..].to_vec()
    } else {
        Vec::new()
    };
    let mut lines = text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty request"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing method"))?
        .to_ascii_uppercase();
    let target = request_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing target"))?
        .to_string();

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    Ok(Some(Request {
        method,
        target,
        headers,
        body_prefix,
    }))
}

fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn resolve_request_path(root: &Path, target: &str) -> Option<PathBuf> {
    let path_part = request_path_part(target)?.trim_start_matches('/');
    let decoded = percent_decode(path_part);
    let normalized = decoded.replace('\\', "/");
    let mut path = root.to_path_buf();

    for component in normalized.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." || component.contains('\0') {
            return None;
        }
        path.push(component);
    }

    Some(path)
}

fn normalize_request_target(target: &str) -> String {
    let (before_query, query) = target.split_once('?').unwrap_or((target, ""));
    let path = request_path_part(before_query).unwrap_or(before_query);
    if query.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{query}")
    }
}

fn request_path_part(target: &str) -> Option<&str> {
    let before_query = target.split('?').next().unwrap_or("");
    let lower = before_query.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let scheme_len = lower.find("://")? + 3;
        let rest = &before_query[scheme_len..];
        return Some(match rest.find('/') {
            Some(index) => &rest[index..],
            None => "/",
        });
    }
    if before_query.contains("://") {
        return None;
    }
    Some(before_query)
}

fn ensure_inside_root(canonical_root: &Path, path: PathBuf) -> Option<PathBuf> {
    let canonical = fs::canonicalize(&path).ok()?;
    if canonical.starts_with(canonical_root) {
        Some(canonical)
    } else {
        None
    }
}

fn send_file(
    stream: &mut TcpStream,
    path: &Path,
    headers: &HashMap<String, String>,
    head_only: bool,
    keep_alive: bool,
    buf: &mut [u8],
) -> io::Result<()> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        send_text(
            stream,
            404,
            "Not Found",
            "Not found.",
            "text/plain; charset=utf-8",
            head_only,
            keep_alive,
        )?;
        return Ok(());
    }

    let file_len = metadata.len();
    let content_type = content_type(path);
    let last_modified = format_http_date(metadata.modified().ok());
    let etag = metadata
        .modified()
        .ok()
        .map(|mtime| generate_etag(file_len, mtime))
        .unwrap_or_default();

    // ETag conditional: If-None-Match
    if !etag.is_empty() {
        if let Some(if_none_match) = headers.get("if-none-match") {
            if etag_matches(if_none_match, &etag) {
                send_headers(
                    stream,
                    304,
                    "Not Modified",
                    &[("ETag", &etag), ("Cache-Control", "public, max-age=3600")],
                    keep_alive,
                )?;
                return Ok(());
            }
        }
    }

    let range_header = headers.get("range");
    if range_header.is_some() && file_len == 0 {
        send_headers(
            stream,
            416,
            "Range Not Satisfiable",
            &[("Content-Range", "bytes */0"), ("Content-Length", "0")],
            keep_alive,
        )?;
        return Ok(());
    }

    let range_allowed_by_if_range = match headers.get("if-range") {
        Some(if_range) => {
            let v = if_range.trim();
            last_modified.as_deref().map(|lm| v == lm).unwrap_or(false)
                || (!etag.is_empty() && v == etag)
        }
        None => true,
    };

    let range = if range_allowed_by_if_range {
        match range_header.and_then(|value| parse_range(value, file_len)) {
            Some(Ok(range)) => Some(range),
            Some(Err(_)) => {
                let content_range = format!("bytes */{file_len}");
                send_headers(
                    stream,
                    416,
                    "Range Not Satisfiable",
                    &[
                        ("Content-Range", content_range.as_str()),
                        ("Content-Length", "0"),
                    ],
                    keep_alive,
                )?;
                return Ok(());
            }
            None => None,
        }
    } else {
        None
    };

    let (status, reason, start, end) = match range {
        Some((start, end)) => (206, "Partial Content", start, end),
        None => {
            if file_len == 0 {
                (200, "OK", 0, 0)
            } else {
                (200, "OK", 0, file_len - 1)
            }
        }
    };

    let content_len = if file_len == 0 { 0 } else { end - start + 1 };
    let content_len_string = content_len.to_string();

    let mut response_headers = vec![
        ("Content-Type", content_type.as_str()),
        ("Content-Length", content_len_string.as_str()),
        ("Cache-Control", "public, max-age=3600"),
    ];
    if let Some(last_modified) = last_modified.as_deref() {
        response_headers.push(("Last-Modified", last_modified));
    }
    if !etag.is_empty() {
        response_headers.push(("ETag", &etag));
    }

    let content_range;
    if status == 206 {
        content_range = format!("bytes {start}-{end}/{file_len}");
        response_headers.push(("Content-Range", content_range.as_str()));
    }

    send_headers(stream, status, reason, &response_headers, keep_alive)?;

    if head_only || content_len == 0 {
        return Ok(());
    }

    file.seek(SeekFrom::Start(start))?;
    copy_limited(&mut file, stream, content_len, buf)
}

fn parse_range(value: &str, file_len: u64) -> Option<io::Result<(u64, u64)>> {
    let range_spec = value.trim().strip_prefix("bytes=")?;
    if range_spec.contains(',') {
        return None;
    }
    if file_len == 0 {
        return Some(Err(range_not_satisfiable(file_len)));
    }
    let first_range = range_spec.trim();
    let (start_text, end_text) = first_range.split_once('-')?;

    let result = if start_text.is_empty() {
        let suffix_len = end_text.parse::<u64>().ok()?;
        if suffix_len == 0 {
            Err(range_not_satisfiable(file_len))
        } else {
            let start = file_len.saturating_sub(suffix_len);
            Ok((start, file_len - 1))
        }
    } else {
        let start = start_text.parse::<u64>().ok()?;
        let mut end = if end_text.is_empty() {
            file_len - 1
        } else {
            end_text.parse::<u64>().ok()?
        };
        if end >= file_len {
            end = file_len - 1;
        }
        if start >= file_len || start > end {
            Err(range_not_satisfiable(file_len))
        } else {
            Ok((start, end))
        }
    };

    Some(result)
}

fn range_not_satisfiable(file_len: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("range not satisfiable for file length {file_len}"),
    )
}

fn copy_limited(
    input: &mut File,
    output: &mut TcpStream,
    mut remaining: u64,
    buf: &mut [u8],
) -> io::Result<()> {
    while remaining > 0 {
        let to_read = remaining.min(buf.len() as u64) as usize;
        let read = input.read(&mut buf[..to_read])?;
        if read == 0 {
            break;
        }
        output.write_all(&buf[..read])?;
        remaining -= read as u64;
    }
    Ok(())
}

fn send_text(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &str,
    content_type: &str,
    head_only: bool,
    keep_alive: bool,
) -> io::Result<()> {
    let bytes = body.as_bytes();
    let content_len = bytes.len().to_string();
    send_headers(
        stream,
        status,
        reason,
        &[
            ("Content-Type", content_type),
            ("Content-Length", content_len.as_str()),
        ],
        keep_alive,
    )?;
    if !head_only {
        stream.write_all(bytes)?;
    }
    Ok(())
}

fn send_text_with_headers(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &str,
    content_type: &str,
    head_only: bool,
    extra_headers: &[(&str, &str)],
    keep_alive: bool,
) -> io::Result<()> {
    let bytes = body.as_bytes();
    let content_len = bytes.len().to_string();
    let mut headers = vec![
        ("Content-Type", content_type),
        ("Content-Length", content_len.as_str()),
    ];
    headers.extend_from_slice(extra_headers);
    send_headers(stream, status, reason, &headers, keep_alive)?;
    if !head_only {
        stream.write_all(bytes)?;
    }
    Ok(())
}

fn send_binary(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    bytes: &[u8],
    content_type: &str,
    head_only: bool,
    keep_alive: bool,
) -> io::Result<()> {
    let content_len = bytes.len().to_string();
    send_headers(
        stream,
        status,
        reason,
        &[
            ("Content-Type", content_type),
            ("Content-Length", content_len.as_str()),
        ],
        keep_alive,
    )?;
    if !head_only {
        stream.write_all(bytes)?;
    }
    Ok(())
}

fn cors_methods(uploads_enabled: bool) -> &'static str {
    if uploads_enabled {
        "GET, HEAD, OPTIONS, POST"
    } else {
        "GET, HEAD, OPTIONS"
    }
}

fn send_headers_with_uploads(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
    keep_alive: bool,
    uploads_enabled: bool,
) -> io::Result<()> {
    write!(stream, "HTTP/1.1 {status} {reason}\r\n")?;
    write!(stream, "Server: StaticDock/1.0\r\n")?;
    write!(
        stream,
        "Connection: {}\r\n",
        if keep_alive { "keep-alive" } else { "close" }
    )?;
    write!(stream, "Access-Control-Allow-Origin: *\r\n")?;
    write!(
        stream,
        "Access-Control-Allow-Methods: {}\r\n",
        cors_methods(uploads_enabled)
    )?;
    write!(
        stream,
        "Access-Control-Allow-Headers: Origin, X-Requested-With, Content-Type, Accept, Range, If-Range, If-None-Match, X-Upload-Id, X-Chunk-Offset\r\n"
    )?;
    write!(stream, "Accept-Ranges: bytes\r\n")?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(stream, "\r\n")?;
    Ok(())
}

fn send_headers(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
    keep_alive: bool,
) -> io::Result<()> {
    send_headers_with_uploads(
        stream,
        status,
        reason,
        headers,
        keep_alive,
        UPLOADS_ENABLED.load(Ordering::SeqCst),
    )
}

fn send_api_qr(
    stream: &mut TcpStream,
    target: &str,
    head_only: bool,
    keep_alive: bool,
) -> io::Result<()> {
    let text = query_param(target, "text").unwrap_or_else(|| "/".to_string());
    let format = query_param(target, "format").unwrap_or_else(|| "svg".to_string());
    match format.as_str() {
        "bmp" => match qr_bmp(&text) {
            Some(bytes) => send_binary(
                stream,
                200,
                "OK",
                &bytes,
                "image/bmp",
                head_only,
                keep_alive,
            ),
            None => send_text(
                stream,
                400,
                "Bad Request",
                "QR text is too long.",
                "text/plain; charset=utf-8",
                head_only,
                keep_alive,
            ),
        },
        _ => match qr_svg(&text) {
            Some(svg) => send_text(
                stream,
                200,
                "OK",
                &svg,
                "image/svg+xml; charset=utf-8",
                head_only,
                keep_alive,
            ),
            None => send_text(
                stream,
                400,
                "Bad Request",
                "QR text is too long.",
                "text/plain; charset=utf-8",
                head_only,
                keep_alive,
            ),
        },
    }
}

fn qr_code(text: &str) -> Option<QrCode> {
    if text.len() > 512 {
        return None;
    }
    QrCode::encode_text(text, QrCodeEcc::Low).ok()
}

fn qr_svg(text: &str) -> Option<String> {
    let qr = qr_code(text)?;
    let qr_size = qr.size();
    let border = 4;
    let size = qr_size + border * 2;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" shape-rendering=\"crispEdges\"><rect width=\"100%\" height=\"100%\" fill=\"#fff\"/><path fill=\"#111827\" d=\""
    );
    for y in 0..qr_size {
        for x in 0..qr_size {
            if qr.get_module(x, y) {
                svg.push_str(&format!("M{} {}h1v1h-1z", x + border, y + border));
            }
        }
    }
    svg.push_str("\"/></svg>");
    Some(svg)
}

fn qr_bmp(text: &str) -> Option<Vec<u8>> {
    let qr = qr_code(text)?;
    let qr_size = qr.size();
    let border = 4i32;
    let scale = 8i32;
    let pixels = (qr_size + border * 2) * scale;
    let row_stride = ((pixels * 3 + 3) / 4 * 4) as usize;
    let image_size = row_stride * pixels as usize;
    let file_size = 54usize + image_size;
    let mut bmp = Vec::with_capacity(file_size);

    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&(54u32).to_le_bytes());
    bmp.extend_from_slice(&(40u32).to_le_bytes());
    bmp.extend_from_slice(&pixels.to_le_bytes());
    bmp.extend_from_slice(&pixels.to_le_bytes());
    bmp.extend_from_slice(&(1u16).to_le_bytes());
    bmp.extend_from_slice(&(24u16).to_le_bytes());
    bmp.extend_from_slice(&(0u32).to_le_bytes());
    bmp.extend_from_slice(&(image_size as u32).to_le_bytes());
    bmp.extend_from_slice(&(2835i32).to_le_bytes());
    bmp.extend_from_slice(&(2835i32).to_le_bytes());
    bmp.extend_from_slice(&(0u32).to_le_bytes());
    bmp.extend_from_slice(&(0u32).to_le_bytes());

    for y in (0..pixels).rev() {
        let module_y = y / scale - border;
        for x in 0..pixels {
            let module_x = x / scale - border;
            let dark = module_x >= 0
                && module_y >= 0
                && module_x < qr_size
                && module_y < qr_size
                && qr.get_module(module_x, module_y);
            if dark {
                bmp.extend_from_slice(&[0x27, 0x1f, 0x11]);
            } else {
                bmp.extend_from_slice(&[0xff, 0xff, 0xff]);
            }
        }
        while (bmp.len() - 54) % row_stride != 0 {
            bmp.push(0);
        }
    }
    Some(bmp)
}

fn send_api_info(
    stream: &mut TcpStream,
    root: &Path,
    head_only: bool,
    keep_alive: bool,
) -> io::Result<()> {
    let body = format!(
        "{{\"name\":\"StaticDock\",\"root\":\"{}\"}}",
        json_escape(&display_path(root))
    );
    send_text(
        stream,
        200,
        "OK",
        &body,
        "application/json; charset=utf-8",
        head_only,
        keep_alive,
    )
}

fn send_api_list(
    stream: &mut TcpStream,
    root: &Path,
    target: &str,
    head_only: bool,
    keep_alive: bool,
) -> io::Result<()> {
    let query_path = query_param(target, "path").unwrap_or_else(|| "/".to_string());
    let request_path = if query_path.starts_with('/') {
        query_path
    } else {
        format!("/{query_path}")
    };

    let path = match resolve_request_path(root, &request_path) {
        Some(path) => path,
        None => {
            send_text(
                stream,
                403,
                "Forbidden",
                "{\"error\":\"forbidden\"}",
                "application/json; charset=utf-8",
                head_only,
                keep_alive,
            )?;
            return Ok(());
        }
    };

    if !path.exists() {
        send_text(
            stream,
            404,
            "Not Found",
            "{\"error\":\"not found\"}",
            "application/json; charset=utf-8",
            head_only,
            keep_alive,
        )?;
        return Ok(());
    }

    let path = match ensure_inside_root(root, path) {
        Some(path) => path,
        None => {
            send_text(
                stream,
                403,
                "Forbidden",
                "{\"error\":\"forbidden\"}",
                "application/json; charset=utf-8",
                head_only,
                keep_alive,
            )?;
            return Ok(());
        }
    };

    if !path.is_dir() {
        send_text(
            stream,
            404,
            "Not Found",
            "{\"error\":\"not found\"}",
            "application/json; charset=utf-8",
            head_only,
            keep_alive,
        )?;
        return Ok(());
    }

    let mut entries = fs::read_dir(&path)?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        let a_is_file = a.file_type().map(|t| t.is_file()).unwrap_or(true);
        let b_is_file = b.file_type().map(|t| t.is_file()).unwrap_or(true);
        a_is_file
            .cmp(&b_is_file)
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });

    let mut base = request_path.split('?').next().unwrap_or("/").to_string();
    if !base.starts_with('/') {
        base.insert(0, '/');
    }
    if !base.ends_with('/') {
        base.push('/');
    }

    let json_path = if base == "/" {
        "/"
    } else {
        base.trim_end_matches('/')
    };
    let mut body = String::new();
    body.push_str("{\"path\":\"");
    body.push_str(&json_escape(json_path));
    body.push_str("\",\"entries\":[");

    let mut first = true;
    for entry in entries {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let file_type = entry.file_type().ok();
        let is_dir = file_type.as_ref().map(|t| t.is_dir()).unwrap_or(false);
        let metadata = entry.metadata().ok();
        let bytes = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified = metadata
            .and_then(|m| m.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let href = if is_dir {
            format!("{}{}{}", base, percent_encode(&file_name), "/")
        } else {
            format!("{}{}", base, percent_encode(&file_name))
        };
        let kind = entry_kind(&entry.path(), is_dir);
        if !first {
            body.push(',');
        }
        first = false;
        body.push_str("{\"name\":\"");
        body.push_str(&json_escape(&file_name));
        body.push_str("\",\"url\":\"");
        body.push_str(&json_escape(&href));
        body.push_str("\",\"kind\":\"");
        body.push_str(kind);
        body.push_str("\",\"bytes\":");
        body.push_str(&bytes.to_string());
        body.push_str(",\"modified\":");
        body.push_str(&modified.to_string());
        body.push('}');
    }

    body.push_str("]}");
    send_text(
        stream,
        200,
        "OK",
        &body,
        "application/json; charset=utf-8",
        head_only,
        keep_alive,
    )
}

fn query_param(target: &str, name: &str) -> Option<String> {
    let query = target.split_once('?')?.1;
    for part in query.split('&') {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if query_percent_decode(key) == name {
            return Some(query_percent_decode(value));
        }
    }
    None
}

fn entry_kind(path: &Path, is_dir: bool) -> &'static str {
    if is_dir {
        return "dir";
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" => "image",
        "mp4" | "webm" | "mov" | "m4v" | "avi" | "mkv" => "video",
        "mp3" | "wav" | "flac" | "ogg" | "m4a" => "audio",
        "zip" | "rar" | "7z" | "tar" | "gz" => "archive",
        "exe" | "msi" | "apk" | "dmg" => "app",
        "pdf" => "pdf",
        "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => "doc",
        "rs" | "js" | "ts" | "tsx" | "jsx" | "cs" | "py" | "java" | "go" | "cpp" | "c" | "h"
        | "css" | "toml" | "yaml" | "yml" => "code",
        "json" => "json",
        "html" | "htm" => "html",
        _ => "file",
    }
}

fn json_escape(input: &str) -> String {
    let mut output = String::new();
    for ch in input.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch.is_control() => output.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => output.push(ch),
        }
    }
    output
}

fn directory_listing(path: &Path, target: &str) -> io::Result<String> {
    let mut entries = fs::read_dir(path)?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        let a_is_file = a.file_type().map(|t| t.is_file()).unwrap_or(true);
        let b_is_file = b.file_type().map(|t| t.is_file()).unwrap_or(true);
        a_is_file
            .cmp(&b_is_file)
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });

    let mut base = target.split('?').next().unwrap_or("/").to_string();
    if !base.starts_with('/') {
        base.insert(0, '/');
    }
    if !base.ends_with('/') {
        base.push('/');
    }

    let current = if base == "/" {
        "/"
    } else {
        base.trim_end_matches('/')
    };
    let mut body = String::from(
        r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>StaticDock resources</title><style>
:root{--blue:#1e40af;--pink:#be185d;--ink:#111827;--muted:#64748b;--line:rgba(148,163,184,.28);--card:rgba(255,255,255,.9)}*{box-sizing:border-box}body{margin:0;font-family:"Microsoft YaHei UI",Segoe UI,system-ui,sans-serif;color:var(--ink);background:radial-gradient(circle at 8% 15%,rgba(225,29,114,.12),transparent 28%),radial-gradient(circle at 90% 8%,rgba(6,182,212,.14),transparent 30%),linear-gradient(145deg,#fff 0%,#f8fafc 44%,#fff1f5 100%)}main{width:min(1120px,calc(100% - 42px));margin:0 auto;padding:26px 0 44px}.hero{border-radius:32px;padding:34px;background:linear-gradient(135deg,rgba(255,255,255,.92),rgba(255,255,255,.68));box-shadow:0 24px 60px rgba(15,23,42,.12);border:1px solid rgba(255,255,255,.72)}.eyebrow{color:var(--pink);font-weight:900;letter-spacing:.12em;text-transform:uppercase;font-size:12px}h1{margin:10px 0 8px;font-size:38px;letter-spacing:-1.4px}.path{font-family:Consolas,monospace;color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.actions{display:flex;gap:10px;flex-wrap:wrap;margin-top:18px}.btn,button{border:0;border-radius:999px;background:var(--blue);color:#fff;padding:10px 15px;text-decoration:none;font-weight:800;cursor:pointer}.ghost{background:#fff;color:var(--ink);border:1px solid var(--line)}.grid{margin-top:20px;display:grid;grid-template-columns:repeat(auto-fill,minmax(210px,1fr));gap:16px}.item{border:1px solid var(--line);border-radius:24px;background:var(--card);overflow:hidden;box-shadow:0 12px 34px rgba(15,23,42,.07)}.thumb{height:132px;display:grid;place-items:center;font-size:42px;text-decoration:none;background:linear-gradient(135deg,#e0f2fe,#fce7f3);overflow:hidden}.thumb img,.thumb video{width:100%;height:100%;object-fit:cover}.file-icon{width:74px;height:74px;border-radius:24px;display:grid;place-items:center;font-size:34px;background:rgba(255,255,255,.72);box-shadow:inset 0 0 0 1px rgba(255,255,255,.8),0 12px 28px rgba(15,23,42,.08)}.archive{background:#fef3c7}.app{background:#dbeafe}.doc{background:#e0e7ff}.pdf{background:#fee2e2}.audio{background:#dcfce7}.code{background:#e0f2fe}.json{background:#f3e8ff}.meta{padding:12px}.name{font-weight:900;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.info-row{margin-top:8px;display:flex;align-items:center;justify-content:space-between;gap:8px}.kind-badge{max-width:58%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;border-radius:999px;padding:4px 8px;background:#f1f5f9;color:#475569;font-size:11px;font-weight:800}.size{color:var(--muted);font-size:12px;white-space:nowrap}.empty{margin-top:20px;border-radius:22px;background:#fff;padding:24px;color:var(--muted)}.ctx{position:fixed;z-index:20;display:none;min-width:150px;padding:8px;border:1px solid var(--line);border-radius:16px;background:white;box-shadow:0 20px 50px rgba(15,23,42,.2)}.ctx.open{display:block}.ctx button{display:block;width:100%;text-align:left;background:white;color:var(--ink);box-shadow:none;border-radius:10px;padding:9px 11px}.modal{position:fixed;inset:0;background:rgba(15,23,42,.42);display:none;align-items:center;justify-content:center;padding:22px;z-index:30;backdrop-filter:blur(8px)}.modal.open{display:flex}.modal-card{width:min(920px,100%);max-height:min(86vh,820px);overflow:auto;background:white;border-radius:28px;padding:24px;box-shadow:0 30px 80px rgba(15,23,42,.25);position:relative}.modal-close{position:absolute;right:14px;top:14px;width:34px;height:34px;padding:0}.preview-body{display:grid;place-items:center;min-height:220px;border-radius:22px;background:#f8fafc;border:1px solid var(--line);overflow:hidden}.preview-body img,.preview-body video,.preview-body iframe{max-width:100%;width:100%;height:68vh;max-height:68vh;object-fit:contain;border:0;background:#fff}.preview-body audio{width:min(680px,100%);margin:42px}.preview-body pre{width:100%;max-height:68vh;margin:0;overflow:auto;padding:18px;font:13px/1.55 Consolas,monospace;white-space:pre-wrap}@media(max-width:640px){main{width:min(100% - 28px,1120px)}h1{font-size:30px}.grid{grid-template-columns:1fr}}</style></head><body><main><section class="hero"><div class="eyebrow">StaticDock Directory</div><h1>资源目录</h1><div class="path">"#,
    );
    body.push_str(&html_escape(current));
    body.push_str(r#"</div><div class="actions"><a class="btn ghost" href="/">资源首页</a><button onclick="navigator.clipboard&&navigator.clipboard.writeText(location.href)">复制当前地址</button></div></section>"#);

    if entries.is_empty() {
        body.push_str("<div class=\"empty\">这个目录是空的。</div>");
    } else {
        body.push_str("<section class=\"grid\">");
        for entry in entries {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let href = if is_dir {
                format!("{}{}{}", base, percent_encode(&file_name), "/")
            } else {
                format!("{}{}", base, percent_encode(&file_name))
            };
            let kind = entry_kind(&entry.path(), is_dir);
            let metadata = entry.metadata().ok();
            let bytes = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let display_name = if is_dir {
                format!("{}/", file_name)
            } else {
                file_name.clone()
            };
            let href_escaped = html_escape(&href);
            body.push_str("<article class=\"item\" oncontextmenu=\"showCtx(event,'");
            body.push_str(&js_escape(&href));
            body.push_str("')\"><a class=\"thumb\" href=\"");
            body.push_str(&href_escaped);
            body.push_str("\">");
            if kind == "image" {
                body.push_str("<img loading=\"lazy\" src=\"");
                body.push_str(&href_escaped);
                body.push_str("\">");
            } else if kind == "video" {
                body.push_str("<video muted preload=\"metadata\" src=\"");
                body.push_str(&href_escaped);
                body.push_str("#t=0.1\"></video>");
            } else {
                body.push_str("<span class=\"file-icon ");
                body.push_str(kind);
                body.push_str("\">");
                body.push_str(file_icon(kind));
                body.push_str("</span>");
            }
            body.push_str("</a><div class=\"meta\"><div class=\"name\">");
            body.push_str(&html_escape(&display_name));
            body.push_str("</div><div class=\"sub\">");
            body.push_str(kind);
            body.push_str(" · ");
            body.push_str(&format_bytes(bytes));
            body.push_str("</div></div></article>");
        }
        body.push_str("</section>");
    }
    body.push_str(r#"<div class="ctx" id="ctx"><button onclick="previewCtx()">预览/播放</button><button onclick="openCtx()">打开</button><button onclick="openNewCtx()">新标签打开</button><button onclick="downloadCtx()">下载</button><button onclick="copyCtx()">复制 URL</button></div><div class="modal" id="previewModal" onclick="if(event.target===this)closePreview()"><div class="modal-card"><button class="ghost modal-close" onclick="closePreview()">×</button><h2 id="previewTitle">Preview</h2><div class="preview-body" id="previewBody"></div></div></div><script>let ctx='',ctxKind='file',ctxName='',ctxIsDir=false;function showCtx(e,u,k,n,d){e.preventDefault();ctx=u;ctxKind=k;ctxName=n||u;ctxIsDir=d;const m=document.getElementById('ctx');m.style.left=Math.min(e.clientX,innerWidth-170)+'px';m.style.top=Math.min(e.clientY,innerHeight-100)+'px';m.classList.add('open')}function hideCtx(){document.getElementById('ctx').classList.remove('open')}function previewCtx(){hideCtx();if(ctxIsDir)location.href=ctx;else previewFile(ctx,ctxKind,ctxName)}function openCtx(){hideCtx();if(ctxIsDir)location.href=ctx;else previewFile(ctx,ctxKind,ctxName)}function openNewCtx(){hideCtx();window.open(ctx,'_blank','noopener')}function downloadCtx(){hideCtx();const a=document.createElement('a');a.href=ctx;a.download=ctxName||'';document.body.appendChild(a);a.click();a.remove()}function copyCtx(){hideCtx();navigator.clipboard&&navigator.clipboard.writeText(location.origin+ctx)}function esc(s){return(s||'').replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]))}async function previewFile(url,kind,name){const modal=document.getElementById('previewModal'),body=document.getElementById('previewBody');document.getElementById('previewTitle').textContent=name||url;body.innerHTML='加载中...';modal.classList.add('open');if(kind==='image')body.innerHTML='<img src="'+esc(url)+'">';else if(kind==='video')body.innerHTML='<video src="'+esc(url)+'" controls preload="metadata" playsinline></video><div class="preview-help" style="padding:12px 18px;color:#64748b;text-align:center">浏览器若不支持该视频编码，可用右键菜单新标签打开或下载。</div>';else if(kind==='audio')body.innerHTML='<audio src="'+esc(url)+'" controls preload="metadata"></audio>';else if(kind==='pdf')body.innerHTML='<iframe src="'+esc(url)+'"></iframe><div class="preview-help" style="padding:12px 18px;color:#64748b;text-align:center">PDF 未显示时，可用右键菜单新标签打开或下载。</div>';else if(['json','code','html','file'].includes(kind)){try{const r=await fetch(url);const txt=await r.text();body.innerHTML='<pre>'+esc(txt.slice(0,200000))+(txt.length>200000?'\n...':'')+'</pre>'}catch(e){location.href=url}}else body.innerHTML='<div style="padding:24px;text-align:center"><p>此类型不支持直接预览。</p><p><a class="btn" href="'+esc(url)+'" target="_blank">新标签打开</a> <a class="btn ghost" href="'+esc(url)+'" download>下载</a></p></div>'}function closePreview(){document.getElementById('previewModal').classList.remove('open');document.getElementById('previewBody').innerHTML=''}document.addEventListener('click',hideCtx);document.addEventListener('keydown',e=>{if(e.key==='Escape'){hideCtx();closePreview()}});</script></main></body></html>"#);
    Ok(body)
}

fn js_escape(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "")
}

fn file_icon(kind: &str) -> &'static str {
    match kind {
        "dir" => "📁",
        "image" => "🖼️",
        "video" => "🎬",
        "audio" => "🎧",
        "archive" => "🗜️",
        "app" => "📦",
        "doc" => "📝",
        "pdf" => "📕",
        "code" => "⌘",
        "json" => "{}",
        "html" => "🌐",
        _ => "📄",
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn content_type(path: &Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "m4v" => "video/x-m4v",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m3u8" => "application/vnd.apple.mpegurl",
        "ts" => "video/mp2t",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn query_percent_decode(input: &str) -> String {
    percent_decode(&input.replace('+', " "))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                output.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        output.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn percent_encode(input: &str) -> String {
    let mut output = String::new();
    for byte in input.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(*byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

fn format_http_date(time: Option<SystemTime>) -> Option<String> {
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let seconds = time?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let days = seconds / 86_400;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let weekday = WEEKDAYS[(days % 7) as usize];
    Some(format!(
        "{weekday}, {day:02} {} {year:04} {hour:02}:{minute:02}:{second:02} GMT",
        MONTHS[(month - 1) as usize]
    ))
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}

fn get_lan_addresses() -> Vec<(String, String)> {
    let mut addresses = platform_lan_addresses();
    if addresses.is_empty() {
        if let Some(ip) = default_route_ip() {
            addresses.push((ip, "default route".to_string()));
        }
    }
    addresses
}

fn default_route_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    let ip = addr.ip().to_string();
    if is_special_ipv4(&ip) {
        None
    } else {
        Some(ip)
    }
}

fn is_special_ipv4(ip: &str) -> bool {
    let parts = ip
        .split('.')
        .filter_map(|part| part.parse::<u8>().ok())
        .collect::<Vec<_>>();
    if parts.len() != 4 {
        return true;
    }

    let a = parts[0];
    let b = parts[1];
    a == 0 || a == 127 || (a == 169 && b == 254) || (a == 198 && (b == 18 || b == 19)) || a >= 224
}

#[cfg(windows)]
fn platform_lan_addresses() -> Vec<(String, String)> {
    let output = Command::new("ipconfig").output();
    let binding = output.ok();
    let bytes = binding.as_ref().map(|o| o.stdout.as_slice());
    parse_ipconfig_output(bytes)
}

#[cfg(not(windows))]
fn platform_lan_addresses() -> Vec<(String, String)> {
    if let Ok(output) = Command::new("hostname").arg("-I").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        return text
            .split_whitespace()
            .filter(|ip| !ip.starts_with("127.") && !ip.starts_with("169.254."))
            .map(|ip| (ip.to_string(), "network".to_string()))
            .collect();
    }
    Vec::new()
}

fn parse_ipconfig_output(bytes: Option<&[u8]>) -> Vec<(String, String)> {
    let Some(bytes) = bytes else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(bytes);
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    let mut current_adapter = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            current_adapter.clear();
            continue;
        }

        // Detect adapter header: "Ethernet adapter Wi-Fi:" or "Wireless LAN adapter ..."
        if let Some(rest) = trimmed.strip_suffix(':') {
            if rest.contains("adapter") || rest.contains("Adapter") {
                current_adapter = rest.to_string();
                continue;
            }
        }

        // Detect IPv4 address line
        if trimmed.starts_with("IPv4 Address") || trimmed.starts_with("ipv4 address") {
            if let Some(ip) = trimmed.split(':').last() {
                let ip = ip.trim();
                if !ip.is_empty() && !is_special_ipv4(ip) && seen.insert(ip.to_string()) {
                    let alias = if current_adapter.is_empty() {
                        "network".to_string()
                    } else {
                        current_adapter.clone()
                    };
                    results.push((ip.to_string(), alias));
                }
            }
        }
    }

    // Sort: private LAN first, then others
    results.sort_by(|a, b| {
        let a_private = is_private_ipv4(&a.0);
        let b_private = is_private_ipv4(&b.0);
        b_private.cmp(&a_private).then_with(|| a.1.cmp(&b.1))
    });

    results
}

fn is_private_ipv4(ip: &str) -> bool {
    let parts: Vec<u8> = ip.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 4 {
        return false;
    }
    let (a, b) = (parts[0], parts[1]);
    a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
}

#[allow(dead_code)]
fn parse_address_lines(bytes: Option<&[u8]>) -> Vec<(String, String)> {
    let Some(bytes) = bytes else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(bytes);
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for line in text.lines() {
        if let Some((ip, alias)) = line.trim().split_once('|') {
            let ip = ip.trim();
            if !ip.is_empty() && seen.insert(ip.to_string()) {
                result.push((ip.to_string(), alias.trim().to_string()));
            }
        }
    }

    result
}

// ── ETag support ──────────────────────────────────────────────────────

fn generate_etag(size: u64, modified: SystemTime) -> String {
    let mtime_secs = modified
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in size.to_le_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    for byte in mtime_secs.to_le_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("W/\"{:016x}\"", hash)
}

fn etag_matches(header_value: &str, etag: &str) -> bool {
    let v = header_value.trim();
    if v == "*" {
        return true;
    }
    for part in v.split(',') {
        let tag = part.trim();
        // Weak comparison: strip W/" " prefix for comparison
        let a = strip_weak_prefix(etag);
        let b = strip_weak_prefix(tag);
        if a == b {
            return true;
        }
    }
    false
}

fn strip_weak_prefix(s: &str) -> &str {
    s.strip_prefix("W/").unwrap_or(s)
}

// ── Graceful shutdown handler ────────────────────────────────────────

#[cfg(windows)]
fn register_shutdown_handler() {
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }
    extern "system" fn handler(ctrl_type: u32) -> i32 {
        match ctrl_type {
            0 | 2 => {
                // CTRL_C_EVENT | CTRL_CLOSE_EVENT
                SHUTDOWN.store(true, Ordering::SeqCst);
                1
            }
            _ => 0,
        }
    }
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

#[cfg(not(windows))]
fn register_shutdown_handler() {
    unsafe {
        libc_signal(2, sig_handler); // SIGINT
        libc_signal(15, sig_handler); // SIGTERM
    }
}

#[cfg(not(windows))]
extern "C" fn sig_handler(_: i32) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

#[cfg(not(windows))]
unsafe fn libc_signal(signum: i32, handler: extern "C" fn(i32)) {
    extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }
    unsafe {
        signal(signum, handler);
    }
}

// ── Upload API ───────────────────────────────────────────────────────

fn read_body(
    stream: &mut TcpStream,
    content_length: usize,
    buf: &mut [u8],
    body_prefix: &[u8],
) -> io::Result<Vec<u8>> {
    let mut body = Vec::with_capacity(content_length.min(1024 * 1024));
    // Use any body bytes already consumed during header parsing
    let prefix_used = body_prefix.len().min(content_length);
    if prefix_used > 0 {
        body.extend_from_slice(&body_prefix[..prefix_used]);
    }
    let mut remaining = content_length.saturating_sub(prefix_used);
    while remaining > 0 {
        let to_read = remaining.min(buf.len());
        let read = stream.read(&mut buf[..to_read])?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&buf[..read]);
        remaining -= read;
    }
    Ok(body)
}

fn read_body_to_file(
    stream: &mut TcpStream,
    content_length: u64,
    dest: &Path,
    buf: &mut [u8],
    body_prefix: &[u8],
) -> io::Result<u64> {
    let mut file = File::create(dest)?;
    let mut written = 0u64;
    // Write any body bytes already consumed during header parsing
    let prefix_len = body_prefix.len() as u64;
    if prefix_len > 0 && prefix_len <= content_length {
        file.write_all(&body_prefix[..prefix_len as usize])?;
        written += prefix_len;
    }
    let mut remaining = content_length.saturating_sub(prefix_len);
    while remaining > 0 {
        let to_read = remaining.min(buf.len() as u64) as usize;
        let read = stream.read(&mut buf[..to_read])?;
        if read == 0 {
            break;
        }
        file.write_all(&buf[..read])?;
        remaining -= read as u64;
        written += read as u64;
    }
    Ok(written)
}

fn handle_upload_api(
    stream: &mut TcpStream,
    root: &Path,
    target: &str,
    request: &Request,
    buf: &mut [u8],
    keep_alive: bool,
) -> io::Result<()> {
    let content_length: usize = request
        .headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let prefix = &request.body_prefix;

    // Handle Expect: 100-continue
    if request
        .headers
        .get("expect")
        .map(|v| v.to_ascii_lowercase().contains("100-continue"))
        .unwrap_or(false)
    {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        stream.flush()?;
    }

    if target.contains("/upload/init") {
        let body = read_body(stream, content_length, buf, prefix)?;
        return handle_upload_init(stream, root, &body, keep_alive);
    }
    if target.contains("/upload/chunk") {
        return handle_upload_chunk(
            stream,
            root,
            &request.headers,
            content_length as u64,
            buf,
            prefix,
            keep_alive,
        );
    }
    if target.contains("/upload/finish") {
        let body = read_body(stream, content_length, buf, prefix)?;
        return handle_upload_finish(stream, root, &body, keep_alive);
    }
    if target.contains("/upload/status") {
        return handle_upload_status(stream, root, target, keep_alive);
    }
    if target.contains("/upload/cancel") {
        let body = read_body(stream, content_length, buf, prefix)?;
        return handle_upload_cancel(stream, root, &body, keep_alive);
    }

    send_text(
        stream,
        404,
        "Not Found",
        "{\"error\":\"unknown upload endpoint\"}",
        "application/json; charset=utf-8",
        false,
        keep_alive,
    )
}

fn uploads_dir(root: &Path) -> PathBuf {
    root.join(".staticdock-uploads")
}

fn session_dir(root: &Path, id: &str) -> PathBuf {
    uploads_dir(root).join(id)
}

fn generate_upload_id() -> String {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    // Simple pseudo-random hex from time + thread id
    let tid = thread::current().id();
    format!("{:016x}{:?}", t, tid)
        .replace("ThreadId(", "")
        .replace(")", "")
        .chars()
        .take(32)
        .collect()
}

fn read_upload_meta(root: &Path, id: &str) -> Option<HashMap<String, String>> {
    let meta_path = session_dir(root, id).join("meta.json");
    let content = fs::read_to_string(meta_path).ok()?;
    // Simple JSON parse for our known format
    let mut map = HashMap::new();
    for part in content
        .trim_matches(|c| c == '{' || c == '}' || c == ' ')
        .split(',')
    {
        if let Some((k, v)) = part.split_once(':') {
            let k = k.trim().trim_matches('"');
            let v = v.trim().trim_matches('"');
            map.insert(k.to_string(), v.to_string());
        }
    }
    Some(map)
}

fn write_upload_meta(root: &Path, id: &str, meta: &HashMap<String, String>) -> io::Result<()> {
    let dir = session_dir(root, id);
    fs::create_dir_all(&dir)?;
    let mut json = String::from("{");
    let mut first = true;
    for (k, v) in meta {
        if !first {
            json.push(',');
        }
        first = false;
        json.push('"');
        json.push_str(k);
        json.push_str("\":\"");
        json.push_str(&json_escape(v));
        json.push('"');
    }
    json.push('}');
    fs::write(dir.join("meta.json"), json)
}

fn handle_upload_init(
    stream: &mut TcpStream,
    root: &Path,
    body: &[u8],
    keep_alive: bool,
) -> io::Result<()> {
    let body_str = String::from_utf8_lossy(body);
    let mut meta = HashMap::new();

    // Simple JSON field extraction
    if let Some(path) = json_field(&body_str, "path") {
        meta.insert("path".to_string(), path);
    }
    if let Some(filename) = json_field(&body_str, "filename") {
        meta.insert("filename".to_string(), filename);
    }
    if let Some(size) = json_field(&body_str, "size") {
        meta.insert("size".to_string(), size);
    }
    if let Some(mime) = json_field(&body_str, "mime") {
        meta.insert("mime".to_string(), mime);
    }

    let filename = meta.get("filename").cloned().unwrap_or_default();
    let target_path_str = meta.get("path").cloned().unwrap_or_else(|| "/".to_string());

    // Validate filename
    if filename.is_empty()
        || filename.contains("..")
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
    {
        return send_text(
            stream,
            400,
            "Bad Request",
            "{\"error\":\"invalid filename\"}",
            "application/json; charset=utf-8",
            false,
            keep_alive,
        );
    }

    // Validate target path stays within root
    let resolved = match resolve_request_path(root, &target_path_str) {
        Some(p) => p,
        None => {
            return send_text(
                stream,
                403,
                "Forbidden",
                "{\"error\":\"path not allowed\"}",
                "application/json; charset=utf-8",
                false,
                keep_alive,
            );
        }
    };

    let id = generate_upload_id();
    meta.insert("id".to_string(), id.clone());
    meta.insert("received_bytes".to_string(), "0".to_string());
    meta.insert(
        "created_at".to_string(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string(),
    );
    // Strip Windows \\?\ extended path prefix for portable storage
    let target_dir_str = resolved.display().to_string();
    let target_dir_str = target_dir_str
        .strip_prefix(r"\\?\")
        .unwrap_or(&target_dir_str)
        .to_string();
    meta.insert("target_dir".to_string(), target_dir_str);

    if let Err(e) = write_upload_meta(root, &id, &meta) {
        return send_text(
            stream,
            500,
            "Internal Server Error",
            &format!("{{\"error\":\"{}\"}}", json_escape(&e.to_string())),
            "application/json; charset=utf-8",
            false,
            keep_alive,
        );
    }

    let response = format!(
        "{{\"id\":\"{}\",\"chunk_size\":{}}}",
        json_escape(&id),
        CHUNK_SIZE
    );
    send_text(
        stream,
        200,
        "OK",
        &response,
        "application/json; charset=utf-8",
        false,
        keep_alive,
    )
}

fn handle_upload_chunk(
    stream: &mut TcpStream,
    root: &Path,
    headers: &HashMap<String, String>,
    content_length: u64,
    buf: &mut [u8],
    body_prefix: &[u8],
    keep_alive: bool,
) -> io::Result<()> {
    let upload_id = match headers.get("x-upload-id") {
        Some(id) => id.trim().to_string(),
        None => {
            return send_text(
                stream,
                400,
                "Bad Request",
                "{\"error\":\"missing X-Upload-Id header\"}",
                "application/json; charset=utf-8",
                false,
                keep_alive,
            );
        }
    };

    let chunk_offset: u64 = headers
        .get("x-chunk-offset")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    let meta = match read_upload_meta(root, &upload_id) {
        Some(m) => m,
        None => {
            return send_text(
                stream,
                404,
                "Not Found",
                "{\"error\":\"upload session not found\"}",
                "application/json; charset=utf-8",
                false,
                keep_alive,
            );
        }
    };

    let chunk_index = chunk_offset / CHUNK_SIZE;
    let chunk_path = session_dir(root, &upload_id).join(format!("chunk-{:06}", chunk_index));

    // Stream body directly to file
    let written = read_body_to_file(stream, content_length, &chunk_path, buf, body_prefix)?;

    // Update received_bytes in meta
    let mut new_meta = meta;
    let prev_received: u64 = new_meta
        .get("received_bytes")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    new_meta.insert(
        "received_bytes".to_string(),
        (prev_received + written).to_string(),
    );
    write_upload_meta(root, &upload_id, &new_meta)?;

    let response = format!(
        "{{\"ok\":true,\"offset\":{},\"written\":{}}}",
        chunk_offset, written
    );
    send_text(
        stream,
        200,
        "OK",
        &response,
        "application/json; charset=utf-8",
        false,
        keep_alive,
    )
}

fn handle_upload_finish(
    stream: &mut TcpStream,
    root: &Path,
    body: &[u8],
    keep_alive: bool,
) -> io::Result<()> {
    let body_str = String::from_utf8_lossy(body);
    let upload_id = match json_field(&body_str, "id") {
        Some(id) => id,
        None => {
            return send_text(
                stream,
                400,
                "Bad Request",
                "{\"error\":\"missing id\"}",
                "application/json; charset=utf-8",
                false,
                keep_alive,
            );
        }
    };

    let meta = match read_upload_meta(root, &upload_id) {
        Some(m) => m,
        None => {
            return send_text(
                stream,
                404,
                "Not Found",
                "{\"error\":\"upload session not found\"}",
                "application/json; charset=utf-8",
                false,
                keep_alive,
            );
        }
    };

    let filename = meta.get("filename").cloned().unwrap_or_default();
    let target_dir = meta.get("target_dir").cloned().unwrap_or_default();
    let _total_size: u64 = meta.get("size").and_then(|v| v.parse().ok()).unwrap_or(0);

    let target_dir_path = PathBuf::from(&target_dir);
    if !target_dir_path.exists() {
        fs::create_dir_all(&target_dir_path)?;
    }

    let final_path = target_dir_path.join(&filename);
    // Verify final path is inside root
    let canonical_root = fs::canonicalize(root).ok();
    if let Some(ref cr) = canonical_root {
        let final_canonical =
            fs::canonicalize(final_path.parent().unwrap_or(&target_dir_path)).unwrap_or_default();
        if !final_canonical.starts_with(cr) {
            return send_text(
                stream,
                403,
                "Forbidden",
                "{\"error\":\"target path outside root\"}",
                "application/json; charset=utf-8",
                false,
                keep_alive,
            );
        }
    }

    // Merge chunks
    let sess = session_dir(root, &upload_id);
    let mut output = File::create(&final_path)?;
    let mut chunk_files: Vec<_> = fs::read_dir(&sess)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("chunk-"))
        .collect();
    chunk_files.sort_by_key(|e| e.file_name());

    let mut merged_bytes = 0u64;
    for chunk_entry in &chunk_files {
        let mut chunk_file = File::open(chunk_entry.path())?;
        let mut buf = vec![0u8; COPY_BUFFER_SIZE];
        loop {
            let read = chunk_file.read(&mut buf)?;
            if read == 0 {
                break;
            }
            output.write_all(&buf[..read])?;
            merged_bytes += read as u64;
        }
    }

    // Cleanup session
    fs::remove_dir_all(&sess).ok();

    let response = format!(
        "{{\"ok\":true,\"filename\":\"{}\",\"bytes\":{}}}",
        json_escape(&filename),
        merged_bytes
    );
    send_text(
        stream,
        200,
        "OK",
        &response,
        "application/json; charset=utf-8",
        false,
        keep_alive,
    )
}

fn handle_upload_status(
    stream: &mut TcpStream,
    root: &Path,
    target: &str,
    keep_alive: bool,
) -> io::Result<()> {
    let upload_id = match query_param(target, "id") {
        Some(id) => id,
        None => {
            return send_text(
                stream,
                400,
                "Bad Request",
                "{\"error\":\"missing id parameter\"}",
                "application/json; charset=utf-8",
                false,
                keep_alive,
            );
        }
    };

    let meta = match read_upload_meta(root, &upload_id) {
        Some(m) => m,
        None => {
            return send_text(
                stream,
                404,
                "Not Found",
                "{\"error\":\"upload session not found\"}",
                "application/json; charset=utf-8",
                false,
                keep_alive,
            );
        }
    };

    let received: u64 = meta
        .get("received_bytes")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let total: u64 = meta.get("size").and_then(|v| v.parse().ok()).unwrap_or(0);
    let filename = meta.get("filename").cloned().unwrap_or_default();

    let response = format!(
        "{{\"id\":\"{}\",\"filename\":\"{}\",\"received_bytes\":{},\"total_size\":{}}}",
        json_escape(&upload_id),
        json_escape(&filename),
        received,
        total
    );
    send_text(
        stream,
        200,
        "OK",
        &response,
        "application/json; charset=utf-8",
        false,
        keep_alive,
    )
}

fn handle_upload_cancel(
    stream: &mut TcpStream,
    root: &Path,
    body: &[u8],
    keep_alive: bool,
) -> io::Result<()> {
    let body_str = String::from_utf8_lossy(body);
    let upload_id = match json_field(&body_str, "id") {
        Some(id) => id,
        None => {
            return send_text(
                stream,
                400,
                "Bad Request",
                "{\"error\":\"missing id\"}",
                "application/json; charset=utf-8",
                false,
                keep_alive,
            );
        }
    };

    let sess = session_dir(root, &upload_id);
    if sess.exists() {
        fs::remove_dir_all(&sess).ok();
    }

    send_text(
        stream,
        200,
        "OK",
        "{\"ok\":true}",
        "application/json; charset=utf-8",
        false,
        keep_alive,
    )
}

fn cleanup_expired_uploads(root: &Path) {
    let uploads = uploads_dir(root);
    if !uploads.exists() {
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entries = match fs::read_dir(&uploads) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if let Some(meta) = read_upload_meta(root, &id) {
            let created: u64 = meta
                .get("created_at")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if now.saturating_sub(created) > UPLOAD_EXPIRY_SECS {
                fs::remove_dir_all(entry.path()).ok();
            }
        }
    }
}

fn json_field(json: &str, field: &str) -> Option<String> {
    let pattern = format!("\"{}\"", field);
    let idx = json.find(&pattern)?;
    let rest = &json[idx + pattern.len()..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(json_unescape(&rest[..end]))
}

fn json_unescape(s: &str) -> String {
    s.replace("\\\"", "\"")
        .replace("\\\\", "\\")
        .replace("\\n", "\n")
        .replace("\\r", "")
        .replace("\\t", "\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_range_supports_common_forms() {
        assert_eq!(parse_range("bytes=0-4", 10).unwrap().unwrap(), (0, 4));
        assert_eq!(parse_range("bytes=5-", 10).unwrap().unwrap(), (5, 9));
        assert_eq!(parse_range("bytes=-3", 10).unwrap().unwrap(), (7, 9));
        assert!(parse_range("bytes=20-30", 10).unwrap().is_err());
        assert!(parse_range("bytes=0-1,4-5", 10).is_none());
        assert!(parse_range("items=0-1", 10).is_none());
        assert!(parse_range("bytes=0-1", 0).unwrap().is_err());
    }

    #[test]
    fn request_paths_reject_traversal_and_support_absolute_form() {
        let root = Path::new("/root");
        assert_eq!(
            resolve_request_path(root, "/a/b.txt").unwrap(),
            Path::new("/root/a/b.txt")
        );
        assert_eq!(
            resolve_request_path(root, "http://example.test/a/b.txt?x=1").unwrap(),
            Path::new("/root/a/b.txt")
        );
        assert!(resolve_request_path(root, "/../secret").is_none());
        assert!(resolve_request_path(root, "ftp://example.test/a").is_none());
    }

    #[test]
    fn query_decoding_uses_form_plus_semantics() {
        assert_eq!(
            query_param("/__staticdock/api/list?path=/a+b", "path"),
            Some("/a b".to_string())
        );
        assert_eq!(
            query_param("/__staticdock/api/list?path=/a%2Bb", "path"),
            Some("/a+b".to_string())
        );
    }

    #[test]
    fn escaping_helpers_escape_special_characters() {
        assert_eq!(json_escape("a\\\"\n"), "a\\\\\\\"\\n");
        assert_eq!(html_escape("<a&b>\""), "&lt;a&amp;b&gt;&quot;");
    }

    #[test]
    fn http_date_formats_unix_epoch() {
        assert_eq!(
            format_http_date(Some(UNIX_EPOCH)).unwrap(),
            "Thu, 01 Jan 1970 00:00:00 GMT"
        );
    }

    #[test]
    fn special_ipv4_detection() {
        assert!(is_special_ipv4("127.0.0.1"));
        assert!(is_special_ipv4("169.254.1.1"));
        assert!(!is_special_ipv4("192.168.1.10"));
    }

    #[test]
    fn parse_args_supports_bind_and_upload_controls() {
        let config = parse_args_from([
            "--bind",
            "127.0.0.1",
            "--no-upload",
            "--port=9090",
            "--queue",
            "512",
            "C:/tmp/staticdock-root",
        ])
        .unwrap();
        assert_eq!(config.bind, IpAddr::from([127, 0, 0, 1]));
        assert!(!config.uploads_enabled);
        assert_eq!(config.port, 9090);
        assert_eq!(config.queue_size, 512);
        assert_eq!(config.root, PathBuf::from("C:/tmp/staticdock-root"));

        let config = parse_args_from(["--no-upload", "--uploads", "--bind=0.0.0.0"]).unwrap();
        assert_eq!(config.bind, IpAddr::from([0, 0, 0, 0]));
        assert!(config.uploads_enabled);
    }

    #[test]
    fn bind_listener_honors_loopback_bind_address() {
        let (listener, port) = bind_listener(IpAddr::from([127, 0, 0, 1]), 0).unwrap();
        assert!(port > 0);
        assert!(listener.local_addr().unwrap().ip().is_loopback());
    }

    #[test]
    fn allow_methods_reflect_upload_setting() {
        assert_eq!(allow_methods_header(true).1, "GET, HEAD, OPTIONS, POST");
        assert_eq!(allow_methods_header(false).1, "GET, HEAD, OPTIONS");
        assert_eq!(cors_methods(true), "GET, HEAD, OPTIONS, POST");
        assert_eq!(cors_methods(false), "GET, HEAD, OPTIONS");
    }
}
