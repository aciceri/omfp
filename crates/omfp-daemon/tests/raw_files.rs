//! L2: the raw bytes of workspace files over HTTP (DMN-250..DMN-254, PRT-150) — the real daemon (`start_with`, real
//! router, fake launcher and git), requests written by hand so paths reach the daemon exactly as sent.

mod client;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use client::Client;
use omfp_daemon::agent::FakeLauncher;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, start_with};
use serde_json::json;
use sha1::Digest as _;
use tempfile::TempDir;

struct Daemon {
    handle: DaemonHandle,
    git: Arc<FakeGit>,
    _data: TempDir,
}

/// One HTTP response: status, headers (lowercase names) and body.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }
}

impl Daemon {
    async fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        let workspaces = data.path().join("workspaces");
        let args = [
            "daemon".to_owned(),
            "--bind".into(),
            "127.0.0.1:0".into(),
            "--data-dir".into(),
            data.path().display().to_string(),
            "--workspaces-root".into(),
            workspaces.display().to_string(),
        ];
        let config = Config::from_cli(Cli::try_parse_from(args).unwrap(), None, None).unwrap();
        let git = Arc::new(FakeGit::new());
        let ports = Ports {
            launcher: Arc::new(FakeLauncher::new()),
            git: Arc::clone(&git) as _,
            omp_program: "omp".into(),
            omp_version: omfp_core::OMP_VERSION.into(),
            omp_env: Vec::new(),
            user_omp_env: omfp_testkit::absent_standalone_omp(&data),
            warnings: Vec::new(),
            clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
        };
        let handle = start_with(config, ports).await.unwrap();
        Self {
            handle,
            git,
            _data: data,
        }
    }

    /// `system.hello`'s result.
    async fn hello(&self) -> serde_json::Value {
        let mut client = Client::connect(self.handle.addr()).await;
        client
            .call(
                "system.hello",
                json!({ "token": self.handle.token().expose(), "protocol": 1, "client": "test" }),
            )
            .await
            .unwrap()
    }

    /// A new workspace: its id and folder.
    async fn workspace(&self) -> (String, PathBuf) {
        let mut client =
            Client::hello(self.handle.addr(), self.handle.token().expose(), "test").await;
        let ws = client.ok("workspaces.create", json!({})).await;
        (
            ws["id"].as_str().unwrap().to_owned(),
            PathBuf::from(ws["path"].as_str().unwrap()),
        )
    }

    /// `<method> <path>` over HTTP/1.1 with extra header lines, the path sent as given.
    async fn request(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
        let mut stream = tokio::net::TcpStream::connect(self.handle.addr())
            .await
            .unwrap();
        let mut request =
            format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
        for (name, value) in headers {
            for part in [*name, ": ", *value, "\r\n"] {
                request.push_str(part);
            }
        }
        request.push_str("\r\n");
        tokio::io::AsyncWriteExt::write_all(&mut stream, request.as_bytes())
            .await
            .unwrap();
        let mut response = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response)
            .await
            .unwrap();
        let split = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        let head = String::from_utf8(response[..split].to_vec()).unwrap();
        let mut lines = head.lines();
        let status = lines.next().unwrap().split(' ').nth(1).unwrap();
        Reply {
            status: status.parse().unwrap(),
            headers: lines
                .filter_map(|line| line.split_once(": "))
                .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
                .collect(),
            body: response[split + 4..].to_vec(),
        }
    }

    async fn get(&self, path: &str, headers: &[(&str, &str)]) -> Reply {
        self.request("GET", path, headers).await
    }
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// Bytes that are not text, longer than one read of the route.
fn video_bytes() -> Vec<u8> {
    (0..200_000_u32)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect()
}

// spec: DMN-250, PRT-150
#[tokio::test]
async fn files_are_served_only_under_the_file_key_of_hello() {
    let daemon = Daemon::new().await;
    let hello = daemon.hello().await;
    let base = hello["fileBase"].as_str().unwrap().to_owned();
    let widget_base = hello["widgetBase"].as_str().unwrap();
    assert!(base.starts_with("/files/"), "{base}");
    assert_eq!(base.len(), "/files/".len() + 32, "128-bit key in hex");
    let widget_key = widget_base.trim_start_matches("/widgets/");
    assert_ne!(&base["/files/".len()..], widget_key, "a key of its own");
    assert_eq!(daemon.hello().await["fileBase"], base, "one key per start");
    let (id, folder) = daemon.workspace().await;
    write(&folder.join("notes/a b.txt"), b"hello");

    let served = daemon
        .get(&format!("{base}/{id}/notes/a%20b.txt"), &[])
        .await;
    assert_eq!(
        (served.status, served.body.as_slice()),
        (200, &b"hello"[..])
    );
    // A query only busts caches.
    let query = format!("{base}/{id}/notes/a%20b.txt?v=2");
    assert_eq!(daemon.get(&query, &[]).await.status, 200);

    for refused in [
        format!("/files/{}/{id}/notes/a%20b.txt", "0".repeat(32)),
        format!("/files/{widget_key}/{id}/notes/a%20b.txt"),
        format!("/files/{id}/notes/a%20b.txt"),
        format!("/files/{}/notes/a%20b.txt", &base["/files/".len()..]),
    ] {
        let reply = daemon.get(&refused, &[]).await;
        assert_eq!(reply.status, 404, "{refused}");
        assert_eq!(reply.header("cache-control"), Some("no-store"), "{refused}");
    }
}

// spec: DMN-250, DMN-312, DMN-314, PRT-197
#[tokio::test]
async fn paths_resolve_as_read_file_does_and_only_absolute_ones_leave_the_workspace() {
    let daemon = Daemon::new().await;
    let base = daemon.hello().await["fileBase"]
        .as_str()
        .unwrap()
        .to_owned();
    let (id, folder) = daemon.workspace().await;
    let outside = tempfile::tempdir().unwrap();
    write(&outside.path().join("secret.txt"), b"secret");
    write(&outside.path().join("repo/src/main.rs"), b"fn main() {}\n");
    std::os::unix::fs::symlink(outside.path().join("repo"), folder.join("app")).unwrap();
    write(&folder.join("dir/inside.txt"), b"inside");
    let parent = folder.parent().unwrap();
    write(&parent.join("secret.txt"), b"secret");

    // A linked repository is the workspace's; an absolute path inside the folder is served too.
    let linked = daemon
        .get(&format!("{base}/{id}/app/src/main.rs"), &[])
        .await;
    assert_eq!(
        (linked.status, linked.body.as_slice()),
        (200, &b"fn main() {}\n"[..])
    );
    let absolute = format!("{base}/{id}/{}", folder.join("dir/inside.txt").display());
    assert_eq!(daemon.get(&absolute, &[]).await.body, b"inside");

    // An absolute path outside the folder is served as it is, typed and with the headers of every file (DMN-314).
    let outside_absolute = outside.path().join("secret.txt");
    let served = daemon
        .get(&format!("{base}/{id}/{}", outside_absolute.display()), &[])
        .await;
    assert_eq!(
        (served.status, served.body.as_slice()),
        (200, &b"secret"[..])
    );
    assert_eq!(
        served.header("content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(served.header("content-security-policy"), Some("sandbox"));
    let dotted = format!("{base}/{id}/{}/%2e%2e/secret.txt", folder.display());
    assert_eq!(daemon.get(&dotted, &[]).await.body, b"secret");

    for refused in [
        format!("{base}/{id}/../secret.txt"),
        format!("{base}/{id}/%2e%2e/secret.txt"),
        format!("{base}/{id}/dir/..%2f..%2fsecret.txt"),
        format!(
            "{base}/{id}/{}",
            outside.path().join("missing.txt").display()
        ),
        format!("{base}/{id}/{}", outside.path().display()),
        format!("{base}/{id}/missing.txt"),
        format!("{base}/{id}/dir"),
        format!("{base}/{id}/"),
        format!("{base}/no-such-workspace/{}", outside_absolute.display()),
        format!(
            "/files/{}/{id}/{}",
            "0".repeat(32),
            outside_absolute.display()
        ),
    ] {
        assert_eq!(daemon.get(&refused, &[]).await.status, 404, "{refused}");
    }
}

// spec: DMN-311, DMN-314
#[tokio::test]
async fn rev_head_serves_an_outside_file_at_head_of_the_working_tree_holding_it() {
    let daemon = Daemon::new().await;
    let base = daemon.hello().await["fileBase"]
        .as_str()
        .unwrap()
        .to_owned();
    let (id, _) = daemon.workspace().await;
    let outside = tempfile::tempdir().unwrap();
    let other = outside.path().join("other");
    write(&other.join("img/logo.png"), b"working copy");
    write(
        &outside.path().join("loose.png"),
        b"outside every repository",
    );
    daemon.git.add_repo(&other, "main");
    daemon.git.commit_file(&other, "img/logo.png", b"at HEAD");

    let url = format!("{base}/{id}/{}", other.join("img/logo.png").display());
    assert_eq!(daemon.get(&url, &[]).await.body, b"working copy");
    let head = daemon.get(&format!("{url}?rev=HEAD"), &[]).await;
    assert_eq!((head.status, head.body.as_slice()), (200, &b"at HEAD"[..]));
    assert_eq!(head.header("content-type"), Some("image/png"));
    let loose = format!(
        "{base}/{id}/{}?rev=HEAD",
        outside.path().join("loose.png").display()
    );
    assert_eq!(daemon.get(&loose, &[]).await.status, 404);
}

// spec: DMN-252
#[tokio::test]
async fn one_byte_range_is_a_partial_response_and_the_whole_file_streams() {
    let daemon = Daemon::new().await;
    let base = daemon.hello().await["fileBase"]
        .as_str()
        .unwrap()
        .to_owned();
    let (id, folder) = daemon.workspace().await;
    let bytes = video_bytes();
    write(&folder.join("clip.webm"), &bytes);
    let url = format!("{base}/{id}/clip.webm");

    let whole = daemon.get(&url, &[]).await;
    assert_eq!(whole.status, 200);
    assert_eq!(whole.header("accept-ranges"), Some("bytes"));
    assert_eq!(whole.header("content-length"), Some("200000"));
    assert_eq!(whole.header("content-type"), Some("video/webm"));
    assert!(whole.body == bytes, "every byte, across reads");

    let part = daemon.get(&url, &[("Range", "bytes=65530-131080")]).await;
    assert_eq!(part.status, 206);
    assert_eq!(
        part.header("content-range"),
        Some("bytes 65530-131080/200000")
    );
    assert_eq!(part.header("content-length"), Some("65551"));
    assert!(part.body == bytes[65_530..=131_080], "the asked bytes");
    let tail = daemon.get(&url, &[("Range", "bytes=-10")]).await;
    assert_eq!(
        tail.header("content-range"),
        Some("bytes 199990-199999/200000")
    );
    assert_eq!(tail.body, bytes[199_990..]);

    let refused = daemon.get(&url, &[("Range", "bytes=200000-")]).await;
    assert_eq!(refused.status, 416);
    assert_eq!(refused.header("content-range"), Some("bytes */200000"));
    let several = daemon.get(&url, &[("Range", "bytes=0-1,5-6")]).await;
    assert_eq!((several.status, several.body.len()), (200, 200_000));

    let head = daemon
        .request("HEAD", &url, &[("Range", "bytes=0-9")])
        .await;
    assert_eq!(
        (head.status, head.header("content-length")),
        (206, Some("10"))
    );
    assert_eq!(head.body, Vec::<u8>::new());
}

// spec: DMN-253
#[tokio::test]
async fn validators_answer_a_current_copy_with_not_modified() {
    let daemon = Daemon::new().await;
    let base = daemon.hello().await["fileBase"]
        .as_str()
        .unwrap()
        .to_owned();
    let (id, folder) = daemon.workspace().await;
    write(&folder.join("song.mp3"), b"ID3 not really");
    let url = format!("{base}/{id}/song.mp3");

    let first = daemon.get(&url, &[]).await;
    let tag = first.header("etag").unwrap().to_owned();
    let date = first.header("last-modified").unwrap().to_owned();
    assert!(
        tag.starts_with('"') && tag.ends_with('"'),
        "a strong tag: {tag}"
    );
    assert!(date.ends_with(" GMT"), "an HTTP date: {date}");
    assert_eq!(first.header("content-type"), Some("audio/mpeg"));

    let same = daemon.get(&url, &[("If-None-Match", &tag)]).await;
    assert_eq!((same.status, same.body.len()), (304, 0));
    assert_eq!(same.header("etag"), Some(tag.as_str()));
    assert_eq!(
        daemon
            .get(&url, &[("If-Modified-Since", &date)])
            .await
            .status,
        304
    );
    let other = daemon.get(&url, &[("If-None-Match", "\"other\"")]).await;
    assert_eq!(other.status, 200);

    // A range of another version is answered with the whole current file.
    let stale = [("Range", "bytes=0-2"), ("If-Range", "\"other\"")];
    assert_eq!(daemon.get(&url, &stale).await.status, 200);
    let current = [("Range", "bytes=0-2"), ("If-Range", tag.as_str())];
    assert_eq!(daemon.get(&url, &current).await.body, b"ID3");

    std::fs::write(folder.join("song.mp3"), b"ID3 changed, longer").unwrap();
    let changed = daemon.get(&url, &[("If-None-Match", &tag)]).await;
    assert_eq!(changed.status, 200);
    assert_ne!(changed.header("etag"), Some(tag.as_str()));
}

// spec: DMN-251, DMN-254
#[tokio::test]
async fn files_have_their_type_and_never_run_in_the_daemons_origin() {
    let daemon = Daemon::new().await;
    let base = daemon.hello().await["fileBase"]
        .as_str()
        .unwrap()
        .to_owned();
    let (id, folder) = daemon.workspace().await;
    write(&folder.join("doc.pdf"), b"%PDF-1.4\n");
    write(&folder.join("scan"), b"%PDF-1.7\n");
    write(&folder.join("page.html"), b"<script>alert(1)</script>");
    write(&folder.join("data.bin"), b"\0\x01\x02");

    for (file, expected) in [
        ("doc.pdf", "application/pdf"),
        ("scan", "application/pdf"),
        ("page.html", "text/plain; charset=utf-8"),
        ("data.bin", "application/octet-stream"),
    ] {
        let reply = daemon.get(&format!("{base}/{id}/{file}"), &[]).await;
        assert_eq!(reply.header("content-type"), Some(expected), "{file}");
        assert_eq!(reply.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(reply.header("content-security-policy"), Some("sandbox"));
        assert_eq!(
            reply.header("cross-origin-resource-policy"),
            Some("same-origin")
        );
        assert_eq!(reply.header("cache-control"), Some("private, no-cache"));
    }
}

// spec: DMN-255
#[tokio::test]
async fn rev_head_serves_the_bytes_head_has_with_their_hash_as_validator() {
    let daemon = Daemon::new().await;
    let base = daemon.hello().await["fileBase"]
        .as_str()
        .unwrap()
        .to_owned();
    let (id, folder) = daemon.workspace().await;
    let app = folder.join("app");
    write(&app.join("logo.png"), b"working copy");
    write(&app.join("gone.png"), b"deleted below");
    write(&folder.join("notes.png"), b"outside every repository");
    daemon.git.add_repo(&app, "main");
    daemon.git.commit_file(&app, "logo.png", b"at HEAD");
    daemon.git.commit_file(&app, "gone.png", b"only at HEAD");
    std::fs::remove_file(app.join("gone.png")).unwrap();
    let mut client =
        Client::hello(daemon.handle.addr(), daemon.handle.token().expose(), "test").await;
    client
        .ok("workspaces.rescan", json!({ "workspaceId": id }))
        .await;

    let url = format!("{base}/{id}/app/logo.png");
    let head = daemon.get(&format!("{url}?v=1&rev=HEAD"), &[]).await;
    assert_eq!((head.status, head.body.as_slice()), (200, &b"at HEAD"[..]));
    // Hex SHA-1 of the bytes, as the `headHash` of `workspaces.read_file`.
    let etag = format!("\"{}\"", hex::encode(sha1::Sha1::digest(b"at HEAD")));
    assert_eq!(head.header("etag"), Some(etag.as_str()));
    for (name, value) in [
        ("content-type", "image/png"),
        ("accept-ranges", "none"),
        ("cache-control", "private, no-cache"),
        ("content-security-policy", "sandbox"),
        ("x-content-type-options", "nosniff"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        assert_eq!(head.header(name), Some(value), "{name}");
    }
    let same = daemon
        .get(&format!("{url}?rev=HEAD"), &[("If-None-Match", &etag)])
        .await;
    assert_eq!((same.status, same.body.len()), (304, 0));
    assert_eq!(daemon.get(&url, &[]).await.body, b"working copy");
    // A file deleted since is still at HEAD.
    assert_eq!(
        daemon
            .get(&format!("{base}/{id}/app/gone.png?rev=HEAD"), &[])
            .await
            .body,
        b"only at HEAD"
    );

    for refused in [
        format!("{base}/{id}/app/new.png?rev=HEAD"),
        format!("{base}/{id}/notes.png?rev=HEAD"),
        format!("{base}/{id}/../secret.png?rev=HEAD"),
        format!("/files/{}/{id}/app/logo.png?rev=HEAD", "0".repeat(32)),
    ] {
        assert_eq!(daemon.get(&refused, &[]).await.status, 404, "{refused}");
    }
}
