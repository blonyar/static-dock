use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

const COPY_BUFFER_SIZE: usize = 256 * 1024;
const WORKER_STACK_SIZE: usize = 512 * 1024;

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
    println!("Example JSON:      http://127.0.0.1:{selected_port}/loadtest/data/manifest.json");
    println!("Example image:     http://127.0.0.1:{selected_port}/loadtest/images/test_image_01_1280x720.png");
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
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    }

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                stream.set_nodelay(true).ok();
                if sender.send(stream).is_err() {
                    break;
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
    let mut port = 8000u16;
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
                workers = parse_positive_usize(&value, "workers")?;
            }
            "--queue" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "missing value for --queue")
                })?;
                queue_size = Some(parse_positive_usize(&value, "queue")?);
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
                workers = parse_positive_usize(&arg["--workers=".len()..], "workers")?;
            }
            _ if arg.starts_with("--queue=") => {
                queue_size = Some(parse_positive_usize(&arg["--queue=".len()..], "queue")?);
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

    let queue_size = queue_size.unwrap_or_else(|| workers.saturating_mul(16).max(256));

    Ok(Config {
        root,
        port,
        workers,
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

fn parse_positive_usize(value: &str, name: &str) -> io::Result<usize> {
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
    println!("  static-dock.exe PATH -p 8000 --workers 128");
}

fn bind_listener(preferred_port: u16) -> io::Result<(TcpListener, u16)> {
    let candidates = [preferred_port, 8000, 8080, 8010, 8020, 3000, 5173, 9000];
    let mut tried = HashSet::new();

    for port in candidates {
        if !tried.insert(port) {
            continue;
        }

        match TcpListener::bind(("0.0.0.0", port)) {
            Ok(listener) => return Ok((listener, port)),
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => continue,
            Err(err) => return Err(err),
        }
    }

    let listener = TcpListener::bind(("0.0.0.0", 0))?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

fn handle_client(mut stream: TcpStream, root: &Path) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream
        .set_write_timeout(Some(Duration::from_secs(120)))
        .ok();

    let request = match read_request(&mut stream)? {
        Some(request) => request,
        None => return Ok(()),
    };

    let head_only = request.method == "HEAD";

    if request.method == "OPTIONS" {
        send_headers(&mut stream, 204, "No Content", &[("Content-Length", "0")])?;
        return Ok(());
    }

    if request.method != "GET" && request.method != "HEAD" {
        send_text(
            &mut stream,
            405,
            "Method Not Allowed",
            "Method not allowed.",
            "text/plain; charset=utf-8",
            head_only,
        )?;
        return Ok(());
    }

    let path = match resolve_request_path(root, &request.target) {
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
            let html = directory_listing(&path, &request.target)?;
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

fn read_request(stream: &mut TcpStream) -> io::Result<Option<Request>> {
    let mut buffer = Vec::with_capacity(8192);
    let mut chunk = [0u8; 1024];

    loop {
        let read = stream.read(&mut chunk)?;
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
    let path_part = target
        .split('?')
        .next()
        .unwrap_or("")
        .trim_start_matches('/');
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

fn send_file(
    stream: &mut TcpStream,
    path: &Path,
    headers: &HashMap<String, String>,
    head_only: bool,
) -> io::Result<()> {
    let metadata = fs::metadata(path)?;
    let file_len = metadata.len();
    let content_type = content_type(path);

    let range = if file_len > 0 {
        match headers
            .get("range")
            .and_then(|value| parse_range(value, file_len))
        {
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
    let last_modified = format_http_date(metadata.modified().ok());

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

    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    copy_limited(&mut file, stream, content_len)
}

fn parse_range(value: &str, file_len: u64) -> Option<io::Result<(u64, u64)>> {
    let first_range = value
        .trim()
        .strip_prefix("bytes=")?
        .split(',')
        .next()?
        .trim();
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
        "Access-Control-Allow-Headers: Origin, X-Requested-With, Content-Type, Accept, Range\r\n"
    )?;
    write!(stream, "Accept-Ranges: bytes\r\n")?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(stream, "\r\n")?;
    Ok(())
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

    let mut body = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>StaticDock resources</title></head><body><h1>StaticDock resources</h1><ul>",
    );

    for entry in entries {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let display_name = if is_dir {
            format!("{}/", file_name)
        } else {
            file_name.clone()
        };
        let href = if is_dir {
            format!("{}{}{}", base, percent_encode(&file_name), "/")
        } else {
            format!("{}{}", base, percent_encode(&file_name))
        };
        body.push_str("<li><a href=\"");
        body.push_str(&html_escape(&href));
        body.push_str("\">");
        body.push_str(&html_escape(&display_name));
        body.push_str("</a></li>");
    }

    body.push_str("</ul></body></html>");
    Ok(body)
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

fn format_http_date(_time: Option<std::time::SystemTime>) -> Option<String> {
    None
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
    Some(addr.ip().to_string())
}

#[cfg(windows)]
fn platform_lan_addresses() -> Vec<(String, String)> {
    let script = r#"Get-NetIPAddress -AddressFamily IPv4 |
Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } |
Sort-Object @{Expression={if ($_.InterfaceAlias -match 'VMware|Virtual|Loopback') { 1 } else { 0 }}}, InterfaceMetric, InterfaceAlias |
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
