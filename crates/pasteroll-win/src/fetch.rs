//! Downloads a release file. GitHub's download links answer with a redirect to its own file hosts,
//! so redirects are followed here by hand, under rules: HTTPS only, at most three hops, only to
//! GitHub's own hosts, no sign-in, nothing taken from the answer except where to go next. The body
//! is capped by size and by time, and hashed as it arrives.

use crate::update::{send, status_message, Endpoint};
use pasteroll_core::update_manifest::Hasher;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Set by the update window's Cancel button: the download of the update stops at its next chunk.
static CANCEL: AtomicBool = AtomicBool::new(false);
/// Never set: the small downloads (the note and the manifest) cannot be cancelled.
static NEVER: AtomicBool = AtomicBool::new(false);
/// What a cancelled download reports.
pub const CANCELLED: &str = "the download was cancelled";

/// Asks the running download of the update to stop.
pub fn request_cancel() {
    CANCEL.store(true, Ordering::Release);
}

/// Forgets an earlier cancel (before a new update starts).
pub fn reset_cancel() {
    CANCEL.store(false, Ordering::Release);
}

/// Whether Cancel was pressed since the last [`reset_cancel`].
pub fn cancel_requested() -> bool {
    CANCEL.load(Ordering::Acquire)
}

/// GitHub's own hosts: the release page, and the two names its file storage has used.
pub const GITHUB_HOSTS: [&str; 3] = ["github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"];

/// A release file may take this long to arrive, in all. (A server that trickles bytes cannot keep
/// resetting the per-operation timeouts: the whole download has this one limit.)
const DOWNLOAD_TIME: Duration = Duration::from_secs(300);
/// A small file (the newest-version note, a manifest or a signature) may take this long.
const SMALL_TIME: Duration = Duration::from_secs(45);
const MAX_HOPS: usize = 3;

const FILE_HEADERS: &str = "Accept: application/octet-stream";

/// Where the body goes.
pub enum Dest<'a> {
    /// Into memory, at most this many bytes (small files: the manifest and its signature).
    Memory(&'a mut Vec<u8>, usize),
    /// Into a file, at most the caller's limit (releases are held to `MAX_ASSET_BYTES`).
    File(&'a mut std::fs::File),
}

/// What arrived.
#[derive(Debug, PartialEq, Eq)]
pub struct Got {
    pub size: u64,
    pub sha256: [u8; 32],
}

/// The parts of an address that may be followed.
#[derive(Debug, PartialEq, Eq)]
struct Target {
    secure: bool,
    host: String,
    port: u16,
    path: String,
}

/// Only plain `https://host/path?query` addresses: no port, no user name, no fragment, no odd
/// characters. (Tests and the end-to-end build may also use `http://127.0.0.1:port/path` for a
/// server of their own.)
fn parse_target(url: &str) -> Option<Target> {
    if url.len() > 8192 || !url.is_ascii() || url.bytes().any(|b| b <= b' ' || b == 0x7f || b == b'\\' || b == b'#') {
        return None;
    }
    let (secure, rest) = match url.strip_prefix("https://") {
        Some(r) => (true, r),
        None => (false, url.strip_prefix("http://")?),
    };
    let slash = rest.find('/')?;
    let (authority, path) = rest.split_at(slash);
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    if secure {
        let ok = authority.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
        if !ok || authority.starts_with('.') || authority.ends_with('.') || authority.contains("..") {
            return None;
        }
        return Some(Target { secure, host: authority.to_owned(), port: 443, path: path.to_owned() });
    }
    // a plain-HTTP server of our own, for tests only
    if !cfg!(any(test, feature = "e2e-test-server")) {
        return None;
    }
    let (host, port) = authority.split_once(':')?;
    (host == "127.0.0.1").then_some(())?;
    Some(Target { secure, host: host.to_owned(), port: port.parse().ok()?, path: path.to_owned() })
}

fn host_allowed(host: &str, hosts: &[&str]) -> bool {
    hosts.iter().any(|h| *h == host)
}

/// Downloads `url` into `dest`, following up to three redirects to hosts in `hosts`. The body may
/// be at most `max` bytes. `progress` is told how many bytes have arrived and how many are
/// expected (when the server said). Nothing is written past the limit.
pub fn download(
    url: &str,
    hosts: &[&str],
    max: u64,
    dest: Dest,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Got, String> {
    download_within(DOWNLOAD_TIME, url, hosts, max, dest, progress, &CANCEL)
}

/// [`download`] with its own limit on the whole transfer.
fn download_within(
    time: Duration,
    url: &str,
    hosts: &[&str],
    max: u64,
    dest: Dest,
    progress: &mut dyn FnMut(u64, Option<u64>),
    cancel: &AtomicBool,
) -> Result<Got, String> {
    let started = Instant::now();
    let mut dest = dest;
    let mut current = url.to_owned();
    for hop in 0..=MAX_HOPS {
        let target = parse_target(&current).ok_or_else(|| "the download address is not acceptable".to_owned())?;
        if !host_allowed(&target.host, hosts) {
            return Err("the download would leave GitHub, so it was not followed".to_owned());
        }
        let ep = Endpoint { host: target.host, port: target.port, secure: target.secure, path: target.path, headers: FILE_HEADERS };
        let response = send(&ep, &format!("Pasteroll/{}", env!("CARGO_PKG_VERSION")))?;
        match response.status {
            200 => {
                let expected = response.content_length();
                if expected.is_some_and(|n| n > max) {
                    return Err("the file is larger than expected".to_owned());
                }
                let mut hasher = Hasher::new();
                let mut size = 0u64;
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    if cancel.load(Ordering::Acquire) {
                        return Err(CANCELLED.to_owned());
                    }
                    if started.elapsed() > time {
                        return Err("the download took too long".to_owned());
                    }
                    let n = response.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    size += n as u64;
                    if size > max {
                        return Err("the file is larger than expected".to_owned());
                    }
                    hasher.update(&buf[..n]);
                    match &mut dest {
                        Dest::Memory(v, cap) => {
                            if v.len() + n > *cap {
                                return Err("the answer was unexpectedly large".to_owned());
                            }
                            v.extend_from_slice(&buf[..n]);
                        }
                        Dest::File(f) => f.write_all(&buf[..n]).map_err(|e| format!("could not save the download ({e})"))?,
                    }
                    progress(size, expected);
                }
                if expected.is_some_and(|n| n != size) {
                    return Err("the download was cut short".to_owned());
                }
                if let Dest::File(f) = &mut dest {
                    f.flush().map_err(|e| format!("could not save the download ({e})"))?;
                }
                return Ok(Got { size, sha256: hasher.finish() });
            }
            301 | 302 | 303 | 307 | 308 => {
                if hop == MAX_HOPS {
                    return Err("the download was redirected too many times".to_owned());
                }
                // an absolute address only; it is checked like the first one
                current = response.location().ok_or_else(|| "the download was redirected without saying where".to_owned())?;
            }
            s => return Err(status_message(s)),
        }
    }
    Err("the download was redirected too many times".to_owned())
}

/// A small file (the manifest or its signature) into memory.
pub fn download_small(url: &str, hosts: &[&str], max: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    download_within(SMALL_TIME, url, hosts, max as u64, Dest::Memory(&mut out, max), &mut |_, _| {}, &NEVER)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};

    /// A local server that answers its connections in turn from a script of (status line, extra
    /// headers, body, content-length to claim). It returns its port and joins to the requests it got.
    fn serve(script: Vec<(&'static str, String, Vec<u8>, Option<usize>)>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, headers, body, claim) in script {
                let (mut s, _): (TcpStream, _) = listener.accept().unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = s.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                seen.push(String::from_utf8_lossy(&req).into_owned());
                let len = claim.unwrap_or(body.len());
                let head = format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {len}\r\nConnection: close\r\n\r\n");
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
            seen
        });
        (port, handle)
    }

    fn local(port: u16, path: &str) -> String {
        format!("http://127.0.0.1:{port}{path}")
    }

    const LOCAL: [&str; 1] = ["127.0.0.1"];

    fn get_small(url: &str, max: usize) -> Result<Vec<u8>, String> {
        download_small(url, &LOCAL, max)
    }

    #[test]
    fn a_plain_download_arrives_whole_and_hashed() {
        let body = vec![42u8; 200_000];
        let (port, server) = serve(vec![("200 OK", String::new(), body.clone(), None)]);
        let mut seen = Vec::new();
        let mut mem = Vec::new();
        let got = download(&local(port, "/f"), &LOCAL, 1 << 20, Dest::Memory(&mut mem, 1 << 20), &mut |n, t| seen.push((n, t))).unwrap();
        assert_eq!(got.size, 200_000);
        assert_eq!(got.sha256, pasteroll_core::update_manifest::sha256(&body));
        assert_eq!(mem, body);
        assert_eq!(seen.last(), Some(&(200_000, Some(200_000))), "progress reaches the end");
        let req = server.join().unwrap().remove(0);
        assert!(req.contains("Accept: application/octet-stream") && req.contains("User-Agent: Pasteroll/"), "{req}");
    }

    #[test]
    fn a_download_to_a_file_is_hashed_as_it_is_written() {
        let body: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
        let (port, server) = serve(vec![("200 OK", String::new(), body.clone(), None)]);
        let path = std::env::temp_dir().join(format!("pasteroll-fetch-{}.bin", std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        let got = download(&local(port, "/f"), &LOCAL, 1 << 20, Dest::File(&mut file), &mut |_, _| {}).unwrap();
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), body);
        assert_eq!(got.sha256, pasteroll_core::update_manifest::sha256(&body));
        let _ = std::fs::remove_file(&path);
        server.join().unwrap();
    }

    #[test]
    fn a_redirect_to_an_allowed_host_is_followed() {
        let (port, server) = {
            // the second hop is the same server, reached by a second connection; learn its port
            // first so the Location can name it
            let probe = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
            let handle = std::thread::spawn(move || {
                let mut seen = Vec::new();
                for i in 0..2 {
                    let (mut s, _) = listener.accept().unwrap();
                    let mut buf = [0u8; 2048];
                    let n = s.read(&mut buf).unwrap();
                    seen.push(String::from_utf8_lossy(&buf[..n]).into_owned());
                    let reply = if i == 0 {
                        format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/cdn/file\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    } else {
                        "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_owned()
                    };
                    let _ = s.write_all(reply.as_bytes());
                }
                seen
            });
            (port, handle)
        };
        assert_eq!(get_small(&local(port, "/start"), 100).unwrap(), b"hello");
        let seen = server.join().unwrap();
        assert!(seen[0].starts_with("GET /start ") && seen[1].starts_with("GET /cdn/file "), "{seen:?}");
    }

    #[test]
    fn a_redirect_off_github_is_not_followed() {
        for target in ["http://localhost:1/x", "https://evil.example/x", "https://github.com.evil.example/x", "ftp://127.0.0.1/x"] {
            let (port, server) = serve(vec![("302 Found", format!("Location: {target}\r\n"), Vec::new(), None)]);
            let err = get_small(&local(port, "/a"), 100).unwrap_err();
            assert!(err.contains("not followed") || err.contains("not acceptable"), "{target}: {err}");
            server.join().unwrap();
        }
    }

    #[test]
    fn a_relative_or_missing_location_is_refused() {
        for headers in ["Location: /other\r\n", ""] {
            let (port, server) = serve(vec![("302 Found", headers.to_owned(), Vec::new(), None)]);
            assert!(get_small(&local(port, "/a"), 100).is_err(), "{headers:?}");
            server.join().unwrap();
        }
    }

    #[test]
    fn redirects_are_limited() {
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        let handle = std::thread::spawn(move || {
            let mut count = 0;
            // answer redirects forever until the client gives up
            listener.set_nonblocking(false).unwrap();
            for _ in 0..8 {
                let Ok((mut s, _)) = listener.accept() else { break };
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                count += 1;
                let reply = format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/again\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = s.write_all(reply.as_bytes());
                if count == MAX_HOPS + 1 {
                    break;
                }
            }
            count
        });
        let err = get_small(&local(port, "/a"), 100).unwrap_err();
        assert!(err.contains("too many"), "{err}");
        assert_eq!(handle.join().unwrap(), MAX_HOPS + 1, "one first request and three hops, no more");
    }

    #[test]
    fn errors_and_oversized_files_are_refused() {
        let (port, server) = serve(vec![("404 Not Found", String::new(), Vec::new(), None)]);
        assert!(get_small(&local(port, "/a"), 100).unwrap_err().contains("404"));
        server.join().unwrap();
        // the server says it is too big
        let (port, server) = serve(vec![("200 OK", String::new(), vec![0; 50], Some(5_000))]);
        assert!(get_small(&local(port, "/a"), 100).unwrap_err().contains("larger"));
        let _ = server.join();
        // it really is too big, whatever it said (nothing past the limit is kept)
        let (port, server) = serve(vec![("200 OK", String::new(), vec![1; 5_000], None)]);
        let mut mem = Vec::new();
        let err = download(&local(port, "/a"), &LOCAL, 1_000, Dest::Memory(&mut mem, 1_000), &mut |_, _| {}).unwrap_err();
        assert!(err.contains("larger"), "{err}");
        assert!(mem.len() <= 1_000);
        let _ = server.join();
    }

    #[test]
    fn a_download_that_stops_early_is_an_error() {
        // claims 1000 bytes, sends 10, closes
        let (port, server) = serve(vec![("200 OK", String::new(), vec![7; 10], Some(1_000))]);
        let err = get_small(&local(port, "/a"), 5_000).unwrap_err();
        assert!(err.contains("cut") || err.contains("short"), "{err}");
        let _ = server.join();
    }

    /// A server that answers one connection by running `reply` on it; joins to the request it got.
    fn serve_with(reply: impl FnOnce(&mut TcpStream) + Send + 'static) -> (u16, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut s, _): (TcpStream, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = s.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
            }
            reply(&mut s);
            String::from_utf8_lossy(&req).into_owned()
        });
        (port, handle)
    }

    #[test]
    fn a_server_that_trickles_bytes_is_stopped_by_the_time_limit() {
        // one byte every 100 ms would take hours; every per-operation timeout is satisfied, so only the
        // limit on the whole download can stop it
        let (port, server) = serve_with(|s| {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n");
            for _ in 0..100_000 {
                if s.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let started = Instant::now();
        let mut mem = Vec::new();
        let url = local(port, "/slow");
        let err = download_within(Duration::from_millis(700), &url, &LOCAL, 1 << 20, Dest::Memory(&mut mem, 1 << 20), &mut |_, _| {}, &AtomicBool::new(false)).unwrap_err();
        assert!(err.contains("too long"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10), "it gave up at the limit, not when the server finished");
        assert!(mem.len() < 100, "only the few bytes that arrived");
        let _ = server.join();
    }

    #[test]
    fn a_cancelled_download_stops_at_the_next_chunk_and_says_so() {
        let body = vec![7u8; 600_000];
        let (port, server) = serve(vec![("200 OK", String::new(), body, None)]);
        let cancel = AtomicBool::new(false);
        let mut mem = Vec::new();
        let mut chunks = 0;
        let err = download_within(
            DOWNLOAD_TIME,
            &local(port, "/f"),
            &LOCAL,
            1 << 20,
            Dest::Memory(&mut mem, 1 << 20),
            &mut |_, _| {
                chunks += 1;
                if chunks == 2 {
                    cancel.store(true, Ordering::Release);
                }
            },
            &cancel,
        )
        .unwrap_err();
        assert_eq!(err, CANCELLED);
        assert!(mem.len() < 600_000, "it did not keep going to the end");
        let _ = server.join();
    }

    #[test]
    fn an_endless_chunked_answer_is_cut_off_at_the_limit() {
        // no Content-Length to check against: the size is counted as the data arrives
        let (port, server) = serve_with(|s| {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
            for _ in 0..200 {
                let mut chunk = b"3e8\r\n".to_vec();
                chunk.extend(vec![b'a'; 1000]);
                chunk.extend(b"\r\n");
                if s.write_all(&chunk).is_err() {
                    return;
                }
            }
            let _ = s.write_all(b"0\r\n\r\n");
        });
        let mut mem = Vec::new();
        let err = download(&local(port, "/c"), &LOCAL, 3_000, Dest::Memory(&mut mem, 3_000), &mut |_, _| {}).unwrap_err();
        assert!(err.contains("larger"), "{err}");
        assert!(mem.len() <= 3_000, "nothing past the limit is kept");
        let _ = server.join();
    }

    #[test]
    fn only_a_whole_file_is_accepted_and_no_one_is_signed_in_to() {
        for (status, headers, wants) in [
            ("206 Partial Content", "Content-Range: bytes 0-4/10\r\n", "206"),
            ("401 Unauthorized", "WWW-Authenticate: Negotiate\r\nWWW-Authenticate: NTLM\r\nWWW-Authenticate: Basic realm=\"x\"\r\n", "401"),
            ("407 Proxy Authentication Required", "Proxy-Authenticate: NTLM\r\nProxy-Authenticate: Negotiate\r\n", "proxy"),
            ("416 Range Not Satisfiable", "", "416"),
            ("500 Internal Server Error", "", "500"),
        ] {
            let (port, server) = serve(vec![(status, headers.to_owned(), b"hello".to_vec(), None)]);
            let err = get_small(&local(port, "/a"), 100).unwrap_err();
            assert!(err.contains(wants), "{status}: {err}");
            let request = server.join().unwrap().concat().to_ascii_lowercase();
            assert!(!request.contains("authorization"), "{status}: Windows must not sign in on its own: {request}");
            assert!(!request.contains("range:"), "{status}: whole files are asked for: {request}");
        }
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for b in data {
            crc ^= u32::from(*b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    /// A valid gzip file holding `data` (one stored block).
    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff, 1];
        out.extend((data.len() as u16).to_le_bytes());
        out.extend((!(data.len() as u16)).to_le_bytes());
        out.extend(data);
        out.extend(crc32(data).to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out
    }

    #[test]
    fn a_compressed_answer_is_taken_as_the_bytes_that_were_sent() {
        // Pasteroll never asks for compression and never turns decompression on, so a server that
        // compresses anyway (a compression bomb, say) is counted, capped and hashed as it arrived on the wire
        let body = gzip(b"hello hello hello hello hello hello");
        let (port, server) = serve(vec![("200 OK", "Content-Encoding: gzip\r\n".to_owned(), body.clone(), None)]);
        assert_eq!(get_small(&local(port, "/z"), 1_000).unwrap(), body);
        let request = server.join().unwrap().concat().to_ascii_lowercase();
        assert!(!request.contains("accept-encoding"), "{request}");
    }

    #[test]
    fn a_long_download_link_with_signed_query_text_is_acceptable() {
        // GitHub's file hosts answer with a long address carrying an encoded signature
        let url = format!(
            "https://release-assets.githubusercontent.com/github-production-release-asset/1422347446/c023ccdb-5b7b-474d-b84b-07501ad09148?sp=r&sv=2018-11-09&sr=b&spr=https&se=2026-10-21T18%3A05%3A31Z&rscd=attachment%3B+filename%3Dpasteroll-0.1.0-beta.9-windows-x64-setup.exe&rsct=application%2Foctet-stream&skoid=96c2d410-5711-43a1-aedd-ab1947aa7ab0&sktid=398a6654-997b-47e9-b12b-9515b896b4de&skt=2026-10-21T17%3A04%3A47Z&ske=2026-10-21T18%3A05%3A31Z&sks=b&skv=2018-11-09&sig={}&jwt={}&response-content-disposition=attachment%3B%20filename%3Dx&response-content-type=application%2Foctet-stream",
            "A".repeat(400),
            "e".repeat(900)
        );
        assert!(url.len() > 1_500 && url.len() < 8_192);
        let t = parse_target(&url).expect("a real-shaped address is accepted");
        assert_eq!(t.host, "release-assets.githubusercontent.com");
        assert!(host_allowed(&t.host, &GITHUB_HOSTS));
        assert!(parse_target(&format!("https://github.com/{}", "a".repeat(8_200))).is_none(), "but not without limit");
    }

    #[test]
    fn more_address_shapes_are_refused() {
        for bad in [
            "https://github.com:443/x",
            "HTTPS://github.com/x",
            "https://github.com%2f@evil.example/x",
            "https://github.com/x\0",
            "https://github.com/x\u{80}",
            "https://xn--github-9d0c.com/x",
            "https://github.com./x",
            "https://github.com/x\ny",
            "https:\\\\github.com/x",
        ] {
            let ok = parse_target(bad).filter(|t| host_allowed(&t.host, &GITHUB_HOSTS));
            assert!(ok.is_none(), "{bad:?}");
        }
    }

    /// Needs the internet: `cargo test -p pasteroll-win -- --ignored real_github`. Downloads the small
    /// checksum file of a published release through GitHub's real redirect to its file host.
    #[test]
    #[ignore]
    fn real_github_redirect_is_followed() {
        let url = "https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.8/SHA256SUMS";
        let body = download_small(url, &GITHUB_HOSTS, 4096).expect("download through GitHub's redirect");
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("pasteroll-0.1.0-beta.8-windows-x64-setup.exe"), "{text}");
    }

    /// Needs the internet: `cargo test -p pasteroll-win -- --ignored real_github`. Reads a file of the
    /// repository from `raw.githubusercontent.com`, the host the signed newest-version note comes from.
    #[test]
    #[ignore]
    fn real_github_raw_host_serves_repository_files() {
        let url = pasteroll_core::update_manifest::latest_url("../LICENSE-MIT").replace("/updates/../", "/");
        let body = download_small(&url, &pasteroll_core::update_manifest::LATEST_HOSTS, 4096).expect("a repository file from the raw host");
        assert!(String::from_utf8(body).unwrap().contains("MIT License"));
    }

    #[test]
    fn only_plain_github_addresses_are_acceptable() {
        let ok = parse_target("https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.9/x.exe?a=b").unwrap();
        assert_eq!((ok.secure, ok.host.as_str(), ok.port), (true, "github.com", 443));
        for bad in [
            "http://github.com/x",
            "https://github.com:8443/x",
            "https://user@github.com/x",
            "https://github.com@evil.example/x",
            "https://github.com/x#frag",
            "https://github.com\\@evil.example/x",
            "https://GitHub.com/x",
            "https://github.com/a b",
            "https://github.com",
            "https:///x",
            "https://.github.com/x",
            "https://git..hub.com/x",
            "//github.com/x",
            "https://github.com/\u{e9}",
        ] {
            assert!(parse_target(bad).is_none(), "{bad}");
        }
        assert!(host_allowed("release-assets.githubusercontent.com", &GITHUB_HOSTS));
        assert!(!host_allowed("github.com.evil.example", &GITHUB_HOSTS));
        assert!(!host_allowed("evilgithub.com", &GITHUB_HOSTS));
        assert!(!host_allowed("api.github.com", &GITHUB_HOSTS), "the API host is not a file host");
    }
}
