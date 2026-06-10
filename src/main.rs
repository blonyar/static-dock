use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const COPY_BUFFER_SIZE: usize = 256 * 1024;
const WORKER_STACK_SIZE: usize = 512 * 1024;
const MAX_WORKERS: usize = 256;
const MAX_QUEUE_SIZE: usize = 4096;
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(2);
const HEADER_TOTAL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct Config {
    root: PathBuf,
    port: u16,
    workers: usize,
    queue_size: usize,
}

struct Request {
    method: String,
    target: String,
    headers: HashMap<String, String>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let config = parse_args()?;
    let (listener, selected_port) = bind_listener(config.port)?;
    let root = fs::canonicalize(&config.root)?;

    println!();
    println!("Resource root: {}", display_path(&root));
    println!("Port: {selected_port}");
    println!("Workers: {}", config.workers);
    println!("Queue size: {}", config.queue_size);
    println!();
    println!("PC local:          http://127.0.0.1:{selected_port}/");

    let lan_addresses = get_lan_addresses();
    if !lan_addresses.is_empty() {
        println!("Phone/LAN URLs:");
        for (ip, alias) in lan_addresses {
            println!("  http://{ip}:{selected_port}/  ({alias})");
        }
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

    let (sender, receiver) = mpsc::sync_channel::<TcpStream>(config.queue_size);
    let receiver = Arc::new(Mutex::new(receiver));

    for worker_id in 0..config.workers {
        let receiver = Arc::clone(&receiver);
        let root = root.clone();
        thread::Builder::new()
            .name(format!("static-dock-worker-{worker_id}"))
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || worker_loop(worker_id, receiver, root))
            .map_err(io::Error::other)?;
    }

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                stream.set_nodelay(true).ok();
                match sender.try_send(stream) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(_stream)) => {
                        eprintln!("Request queue is full; dropping connection");
                    }
                    Err(mpsc::TrySendError::Disconnected(_stream)) => break,
                }
            }
            Err(err) => eprintln!("Connection failed: {err}"),
        }
    }

    Ok(())
}

fn worker_loop(worker_id: usize, receiver: Arc<Mutex<mpsc::Receiver<TcpStream>>>, root: PathBuf) {
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
            Ok(stream) => {
                if let Err(err) = handle_client(stream, &root) {
                    eprintln!("Worker {worker_id} request failed: {err}");
                }
            }
            Err(_) => return,
        }
    }
}

fn parse_args() -> io::Result<Config> {
    let mut root: Option<PathBuf> = None;
    let mut port = 8787u16;
    let mut workers = default_worker_count();
    let mut queue_size: Option<usize> = None;
    let mut args = env::args().skip(1);

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
    println!("  static-dock.exe [--root PATH] [--port PORT] [--workers N] [--queue N]");
    println!("  static-dock.exe PATH -p 8787 --workers 128");
}

fn bind_listener(preferred_port: u16) -> io::Result<(TcpListener, u16)> {
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

        match TcpListener::bind(("0.0.0.0", port)) {
            Ok(listener) => {
                let selected_port = listener.local_addr()?.port();
                return Ok((listener, selected_port));
            }
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => continue,
            Err(err) => return Err(err),
        }
    }

    let listener = TcpListener::bind(("0.0.0.0", 0))?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

fn handle_client(mut stream: TcpStream, root: &Path) -> io::Result<()> {
    stream.set_read_timeout(Some(HEADER_READ_TIMEOUT)).ok();
    stream
        .set_write_timeout(Some(Duration::from_secs(120)))
        .ok();

    let request = match read_request(&mut stream, HEADER_TOTAL_TIMEOUT)? {
        Some(request) => request,
        None => return Ok(()),
    };

    let head_only = request.method == "HEAD";
    let normalized_target = normalize_request_target(&request.target);

    if request.method == "OPTIONS" {
        send_headers(&mut stream, 204, "No Content", &[("Content-Length", "0")])?;
        return Ok(());
    }

    if request.method != "GET" && request.method != "HEAD" {
        send_text_with_headers(
            &mut stream,
            405,
            "Method Not Allowed",
            "Method not allowed.",
            "text/plain; charset=utf-8",
            head_only,
            &[("Allow", "GET, HEAD, OPTIONS")],
        )?;
        return Ok(());
    }

    if normalized_target.starts_with("/__staticdock/api/list") {
        send_api_list(&mut stream, root, &normalized_target, head_only)?;
        return Ok(());
    }

    if normalized_target.starts_with("/__staticdock/api/info") {
        send_api_info(&mut stream, root, head_only)?;
        return Ok(());
    }

    let path = match resolve_request_path(root, &normalized_target) {
        Some(path) => path,
        None => {
            send_text(
                &mut stream,
                403,
                "Forbidden",
                "Forbidden.",
                "text/plain; charset=utf-8",
                head_only,
            )?;
            return Ok(());
        }
    };

    if !path.exists() {
        send_text(
            &mut stream,
            404,
            "Not Found",
            "Not found.",
            "text/plain; charset=utf-8",
            head_only,
        )?;
        return Ok(());
    }

    let path = match ensure_inside_root(root, path) {
        Some(path) => path,
        None => {
            send_text(
                &mut stream,
                403,
                "Forbidden",
                "Forbidden.",
                "text/plain; charset=utf-8",
                head_only,
            )?;
            return Ok(());
        }
    };

    if path.is_dir() {
        let index = path.join("index.html");
        if index.is_file() {
            send_file(&mut stream, &index, &request.headers, head_only)?;
        } else {
            let html = directory_listing(&path, &normalized_target)?;
            send_text(
                &mut stream,
                200,
                "OK",
                &html,
                "text/html; charset=utf-8",
                head_only,
            )?;
        }
        return Ok(());
    }

    if !path.is_file() {
        send_text(
            &mut stream,
            404,
            "Not Found",
            "Not found.",
            "text/plain; charset=utf-8",
            head_only,
        )?;
        return Ok(());
    }

    send_file(&mut stream, &path, &request.headers, head_only)
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

fn ensure_inside_root(root: &Path, path: PathBuf) -> Option<PathBuf> {
    let root = fs::canonicalize(root).ok()?;
    let canonical = fs::canonicalize(&path).ok()?;
    if canonical.starts_with(&root) {
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
        )?;
        return Ok(());
    }

    let file_len = metadata.len();
    let content_type = content_type(path);
    let last_modified = format_http_date(metadata.modified().ok());

    let range_header = headers.get("range");
    if range_header.is_some() && file_len == 0 {
        send_headers(
            stream,
            416,
            "Range Not Satisfiable",
            &[("Content-Range", "bytes */0"), ("Content-Length", "0")],
        )?;
        return Ok(());
    }

    let range_allowed_by_if_range = match headers.get("if-range") {
        Some(if_range) => last_modified
            .as_deref()
            .map(|value| if_range.trim() == value)
            .unwrap_or(false),
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
        ("Cache-Control", "no-cache, no-store, must-revalidate"),
    ];
    if let Some(last_modified) = last_modified.as_deref() {
        response_headers.push(("Last-Modified", last_modified));
    }

    let content_range;
    if status == 206 {
        content_range = format!("bytes {start}-{end}/{file_len}");
        response_headers.push(("Content-Range", content_range.as_str()));
    }

    send_headers(stream, status, reason, &response_headers)?;

    if head_only || content_len == 0 {
        return Ok(());
    }

    file.seek(SeekFrom::Start(start))?;
    copy_limited(&mut file, stream, content_len)
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

fn copy_limited(input: &mut File, output: &mut TcpStream, mut remaining: u64) -> io::Result<()> {
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    while remaining > 0 {
        let to_read = remaining.min(buffer.len() as u64) as usize;
        let read = input.read(&mut buffer[..to_read])?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
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
) -> io::Result<()> {
    let bytes = body.as_bytes();
    let content_len = bytes.len().to_string();
    let mut headers = vec![
        ("Content-Type", content_type),
        ("Content-Length", content_len.as_str()),
    ];
    headers.extend_from_slice(extra_headers);
    send_headers(stream, status, reason, &headers)?;
    if !head_only {
        stream.write_all(bytes)?;
    }
    Ok(())
}

fn send_headers(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
) -> io::Result<()> {
    write!(stream, "HTTP/1.1 {status} {reason}\r\n")?;
    write!(stream, "Server: StaticDock/1.0\r\n")?;
    write!(stream, "Connection: close\r\n")?;
    write!(stream, "Access-Control-Allow-Origin: *\r\n")?;
    write!(
        stream,
        "Access-Control-Allow-Methods: GET, HEAD, OPTIONS\r\n"
    )?;
    write!(
        stream,
        "Access-Control-Allow-Headers: Origin, X-Requested-With, Content-Type, Accept, Range, If-Range\r\n"
    )?;
    write!(stream, "Accept-Ranges: bytes\r\n")?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(stream, "\r\n")?;
    Ok(())
}

fn send_api_info(stream: &mut TcpStream, root: &Path, head_only: bool) -> io::Result<()> {
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
    )
}

fn send_api_list(
    stream: &mut TcpStream,
    root: &Path,
    target: &str,
    head_only: bool,
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
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => "image",
        "mp4" | "webm" | "mov" | "m4v" => "video",
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
:root{--blue:#1e40af;--pink:#be185d;--ink:#111827;--muted:#64748b;--line:rgba(148,163,184,.28);--card:rgba(255,255,255,.9)}*{box-sizing:border-box}body{margin:0;font-family:"Microsoft YaHei UI",Segoe UI,system-ui,sans-serif;color:var(--ink);background:radial-gradient(circle at 8% 15%,rgba(225,29,114,.12),transparent 28%),radial-gradient(circle at 90% 8%,rgba(6,182,212,.14),transparent 30%),linear-gradient(145deg,#fff 0%,#f8fafc 44%,#fff1f5 100%)}main{width:min(1120px,calc(100% - 42px));margin:0 auto;padding:26px 0 44px}.hero{border-radius:32px;padding:34px;background:linear-gradient(135deg,rgba(255,255,255,.92),rgba(255,255,255,.68));box-shadow:0 24px 60px rgba(15,23,42,.12);border:1px solid rgba(255,255,255,.72)}.eyebrow{color:var(--pink);font-weight:900;letter-spacing:.12em;text-transform:uppercase;font-size:12px}h1{margin:10px 0 8px;font-size:38px;letter-spacing:-1.4px}.path{font-family:Consolas,monospace;color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.actions{display:flex;gap:10px;flex-wrap:wrap;margin-top:18px}.btn,button{border:0;border-radius:999px;background:var(--blue);color:#fff;padding:10px 15px;text-decoration:none;font-weight:800;cursor:pointer}.ghost{background:#fff;color:var(--ink);border:1px solid var(--line)}.grid{margin-top:20px;display:grid;grid-template-columns:repeat(auto-fill,minmax(210px,1fr));gap:16px}.item{border:1px solid var(--line);border-radius:24px;background:var(--card);overflow:hidden;box-shadow:0 12px 34px rgba(15,23,42,.07)}.thumb{height:118px;display:grid;place-items:center;font-size:42px;text-decoration:none;background:linear-gradient(135deg,#e0f2fe,#fce7f3)}.meta{padding:12px}.name{font-weight:900;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.sub{color:var(--muted);font-size:12px;margin-top:5px}.row{display:flex;gap:8px;margin-top:12px}.row .btn{font-size:12px;padding:7px 10px}.empty{margin-top:20px;border-radius:22px;background:#fff;padding:24px;color:var(--muted)}@media(max-width:640px){main{width:min(100% - 28px,1120px)}h1{font-size:30px}.grid{grid-template-columns:1fr}}</style></head><body><main><section class="hero"><div class="eyebrow">StaticDock Directory</div><h1>资源目录</h1><div class="path">"#,
    );
    body.push_str(&html_escape(current));
    body.push_str(
        r#"</div><div class="actions"><a class="btn ghost" href="/">资源首页</a><button onclick="navigator.clipboard&&navigator.clipboard.writeText(location.href)">复制地址</button></div></section>"#,
    );

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
            let icon = file_icon(kind);
            let metadata = entry.metadata().ok();
            let bytes = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let display_name = if is_dir {
                format!("{}/", file_name)
            } else {
                file_name.clone()
            };
            body.push_str("<article class=\"item\"><a class=\"thumb\" href=\"");
            body.push_str(&html_escape(&href));
            body.push_str("\">");
            body.push_str(icon);
            body.push_str("</a><div class=\"meta\"><div class=\"name\">");
            body.push_str(&html_escape(&display_name));
            body.push_str("</div><div class=\"sub\">");
            body.push_str(kind);
            body.push_str(" · ");
            body.push_str(&format_bytes(bytes));
            body.push_str("</div><div class=\"row\"><a class=\"btn\" href=\"");
            body.push_str(&html_escape(&href));
            body.push_str("\">打开</a><button class=\"ghost\" onclick=\"navigator.clipboard&&navigator.clipboard.writeText(location.origin+'");
            body.push_str(&html_escape(&href));
            body.push_str("')\">复制</button></div></div></article>");
        }
        body.push_str("</section>");
    }
    body.push_str("</main></body></html>");
    Ok(body)
}

fn file_icon(kind: &str) -> &'static str {
    match kind {
        "dir" => "📁",
        "image" => "🖼️",
        "video" => "🎬",
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
    let script = r#"Get-NetIPAddress -AddressFamily IPv4 |
Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' -and $_.IPAddress -notlike '198.18.*' -and $_.IPAddress -notlike '198.19.*' -and $_.PrefixOrigin -ne 'WellKnown' } |
Sort-Object @{Expression={if ($_.IPAddress -match '^(10\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[01])\.)') { 0 } elseif ($_.InterfaceAlias -match 'VMware|Virtual|Loopback|VPN|TAP|TUN|WSL|Docker|Hyper-V') { 2 } else { 1 }}}, InterfaceMetric, InterfaceAlias |
ForEach-Object { "$($_.IPAddress)|$($_.InterfaceAlias)" }"#;

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output();

    parse_address_lines(output.ok().as_ref().map(|out| out.stdout.as_slice()))
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
    fn address_lines_are_deduplicated() {
        let input = b"192.168.1.2|Wi-Fi\n192.168.1.2|Other\n10.0.0.2|LAN\n";
        assert_eq!(parse_address_lines(Some(input)).len(), 2);
    }
}
