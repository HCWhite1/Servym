//! Aides communes aux tests : client HTTP brut (en-têtes maîtrisés, comme `http.request` de Node),
//! attente d'une condition, port libre, chemin des applications factices.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub use serde_json::json;
pub use tempfile::TempDir;

/// Vérifie qu'un texte correspond à une expression régulière, avec un message lisible.
#[macro_export]
macro_rules! assert_match {
    ($text:expr, $re:expr $(,)?) => {
        $crate::check_match(&$text, $re, true, String::new())
    };
    ($text:expr, $re:expr, $($msg:tt)+) => {
        $crate::check_match(&$text, $re, true, format!($($msg)+))
    };
}

/// Contraire de [`assert_match!`].
#[macro_export]
macro_rules! assert_no_match {
    ($text:expr, $re:expr $(,)?) => {
        $crate::check_match(&$text, $re, false, String::new())
    };
}

#[doc(hidden)]
#[track_caller]
pub fn check_match(text: &str, re: &str, expected: bool, context: String) {
    let matched = regex::Regex::new(re)
        .expect("expression régulière")
        .is_match(text);
    if matched != expected {
        let verb = if expected {
            "ne correspond pas à"
        } else {
            "correspond à"
        };
        panic!(
            "{context}{}« {text} » {verb} /{re}/",
            if context.is_empty() { "" } else { " : " }
        );
    }
}

pub async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// Attend qu'une condition soit vraie, en la vérifiant toutes les 100 ms.
pub async fn until<F, Fut>(mut check: F, timeout_ms: u64)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let end = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < end {
        if check().await {
            return;
        }
        sleep(100).await;
    }
    panic!("délai dépassé ({timeout_ms} ms)");
}

/// Port TCP libre sur 127.0.0.1 au moment de l'appel, jamais rendu deux fois dans un même
/// processus de test (le système peut redonner aussitôt un port qui vient d'être libéré : deux
/// projets d'une même configuration auraient alors le même port).
pub fn free_port() -> u16 {
    static GIVEN: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    let mut given = GIVEN.lock().unwrap();
    loop {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        if !given.contains(&port) {
            given.push(port);
            return port;
        }
    }
}

/// Un port est-il ouvert (quelqu'un écoute) sur 127.0.0.1 ?
pub async fn listening(port: u16) -> bool {
    connect(port).await.is_ok()
}

/// Connexion à 127.0.0.1 ; un port fermé est signalé tout de suite, même sous Windows (qui
/// réessaie sinon pendant une à deux secondes).
pub async fn connect(port: u16) -> std::io::Result<TcpStream> {
    let socket = tokio::net::TcpSocket::new_v4()?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        #[repr(C)]
        struct InitialRto {
            rtt: u16,
            max_syn_retransmissions: u8,
        }
        let params = InitialRto {
            rtt: 0xFE,
            max_syn_retransmissions: 0xFE,
        };
        let mut returned = 0u32;
        // SAFETY : SIO_TCP_INITIAL_RTO avec une structure valide pendant l'appel.
        unsafe {
            windows_sys::Win32::Networking::WinSock::WSAIoctl(
                socket.as_raw_socket() as usize,
                0x9800_0011,
                (&params as *const InitialRto).cast(),
                size_of::<InitialRto>() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
                None,
            );
        }
    }
    socket.connect(([127, 0, 0, 1], port).into()).await
}

/// Chemin du binaire des applications factices (`servym-fixture`), construit au besoin.
pub fn fixture_exe() -> PathBuf {
    static BUILT: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut built = BUILT.lock().unwrap();
    if let Some(path) = built.as_ref() {
        return path.clone();
    }
    let test_exe = std::env::current_exe().unwrap();
    let profile_dir = profile_dir_from_test_exe(&test_exe);
    let bin_name = format!("servym-fixture{}", std::env::consts::EXE_SUFFIX);
    if let Some(dir) = profile_dir.as_deref() {
        let candidate = dir.join(&bin_name);
        if candidate.is_file() {
            *built = Some(candidate.clone());
            return candidate;
        }
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut command = std::process::Command::new(cargo);
    command.args([
        "build",
        "-p",
        "servym-fixtures",
        "--bin",
        "servym-fixture",
        "--message-format=json",
    ]);
    if profile_dir
        .as_deref()
        .and_then(Path::file_name)
        .is_some_and(|n| n == "release")
    {
        command.arg("--release");
    }
    let output = command.output().expect("cargo build");
    assert!(
        output.status.success(),
        "construction de servym-fixture : {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = executable_from_cargo_json(&output.stdout)
        .or_else(|| profile_dir.map(|dir| dir.join(&bin_name)))
        .expect("binaire servym-fixture introuvable dans la sortie de cargo");
    assert!(
        exe.is_file(),
        "binaire servym-fixture introuvable : {}",
        exe.display()
    );
    *built = Some(exe.clone());
    exe
}

fn profile_dir_from_test_exe(test_exe: &Path) -> Option<PathBuf> {
    let parent = test_exe.parent()?;
    match parent.file_name().and_then(|n| n.to_str()) {
        // Ancienne structure Cargo : target/<profil>/deps/<test>
        Some("deps") => parent.parent().map(Path::to_path_buf),
        // Nouvelle structure Cargo (build-dir) : target/<profil>/build/<crate>/<hash>/out/<test>
        Some("out") => parent
            .ancestors()
            .skip(1)
            .take(4)
            .find(|dir| dir.file_name().and_then(|n| n.to_str()) == Some("build"))
            .and_then(Path::parent)
            .map(Path::to_path_buf),
        _ => None,
    }
}

fn executable_from_cargo_json(stdout: &[u8]) -> Option<PathBuf> {
    let text = std::str::from_utf8(stdout).ok()?;
    for line in text.lines() {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if msg.get("reason").and_then(Value::as_str) != Some("compiler-artifact") {
            continue;
        }
        if msg
            .get("target")
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            != Some("servym-fixture")
        {
            continue;
        }
        if let Some(exe) = msg.get("executable").and_then(Value::as_str) {
            return Some(PathBuf::from(exe));
        }
    }
    None
}

/// Dossier temporaire propre à un test.
pub fn temp_dir(prefix: &str) -> TempDir {
    tempfile::Builder::new().prefix(prefix).tempdir().unwrap()
}

/// Écrit un fichier (dossiers parents compris).
pub fn write(path: impl AsRef<Path>, content: impl AsRef<[u8]>) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

pub fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap()
}

pub fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&read(path)).unwrap()
}

/// Chemin sous forme de texte, pour les configurations JSON.
pub fn path_str(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

// ---- Client HTTP brut ----

/// Requête HTTP/1.1 envoyée telle quelle : seuls `Host`, `Content-Length` (s'il y a un corps)
/// et `Connection: close` sont ajoutés.
#[derive(Clone, Debug)]
pub struct Req {
    method: String,
    path: String,
    host: Option<String>,
    headers: Vec<(String, String)>,
    body: Option<String>,
}

impl Req {
    pub fn new(method: &str, path: &str) -> Self {
        Req {
            method: method.into(),
            path: path.into(),
            host: None,
            headers: Vec::new(),
            body: None,
        }
    }
    pub fn get(path: &str) -> Self {
        Self::new("GET", path)
    }
    pub fn post(path: &str) -> Self {
        Self::new("POST", path)
    }
    /// En-tête `Host` (par défaut `localhost:<port>`).
    pub fn host(mut self, host: &str) -> Self {
        self.host = Some(host.into());
        self
    }
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    /// En-tête exigé par Servym pour les requêtes de l'interface.
    pub fn servym(self) -> Self {
        self.header("x-servym-request", "1")
    }
    pub fn cookie(self, cookie: Option<&str>) -> Self {
        match cookie {
            Some(c) => self.header("cookie", c),
            None => self,
        }
    }
    /// Corps JSON, avec `Content-Type: application/json`.
    pub fn json(self, body: &Value) -> Self {
        self.header("content-type", "application/json")
            .body(&body.to_string())
    }
    /// Corps brut, sans `Content-Type`.
    pub fn body(mut self, body: &str) -> Self {
        self.body = Some(body.into());
        self
    }

    fn head(&self, port: u16) -> String {
        let host = self.host.clone().unwrap_or_else(|| format!("localhost:{port}"));
        let mut head = format!(
            "{} {} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n",
            self.method, self.path
        );
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        if let Some(body) = &self.body {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        head
    }

    pub async fn send(&self, port: u16) -> Res {
        self.try_send(port)
            .await
            .unwrap_or_else(|e| panic!("{} {} : {e}", self.method, self.path))
    }

    /// Comme [`Req::send`], mais une connexion refusée est une erreur et non une panique.
    pub async fn try_send(&self, port: u16) -> std::io::Result<Res> {
        let mut stream = connect(port).await?;
        stream.write_all(self.head(port).as_bytes()).await?;
        if let Some(body) = &self.body {
            stream.write_all(body.as_bytes()).await?;
        }
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;
        Res::parse(&raw).ok_or_else(|| std::io::Error::other("réponse HTTP illisible"))
    }

    /// Premiers octets du corps d'une réponse longue (flux SSE) : jusqu'à la première ligne vide.
    pub async fn first_event(&self, port: u16) -> String {
        let mut stream = connect(port).await.unwrap();
        stream.write_all(self.head(port).as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0, "flux fermé avant le premier évènement");
            raw.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&raw);
            if let Some(start) = text.find("\r\n\r\n") {
                let body = decode_chunked_prefix(
                    &text[start + 4..],
                    text[..start]
                        .to_ascii_lowercase()
                        .contains("transfer-encoding: chunked"),
                );
                if body.contains("\n\n") {
                    return body;
                }
            }
        }
    }
}

/// Corps d'un début de réponse, sans les tailles de morceaux en encodage « chunked ».
fn decode_chunked_prefix(body: &str, chunked: bool) -> String {
    if !chunked {
        return body.to_string();
    }
    let mut out = String::new();
    let mut rest = body;
    while let Some(line_end) = rest.find("\r\n") {
        let Ok(size) = usize::from_str_radix(rest[..line_end].trim(), 16) else {
            break;
        };
        let start = line_end + 2;
        let end = (start + size).min(rest.len());
        out.push_str(&rest[start..end]);
        if end + 2 > rest.len() {
            break;
        }
        rest = &rest[end + 2..];
    }
    out
}

/// Réponse HTTP.
#[derive(Clone, Debug)]
pub struct Res {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Res {
    fn parse(raw: &[u8]) -> Option<Res> {
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
        let head = String::from_utf8_lossy(&raw[..split]).into_owned();
        let mut lines = head.split("\r\n");
        let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let mut body = raw[split + 4..].to_vec();
        if headers
            .iter()
            .any(|(n, v)| n == "transfer-encoding" && v.eq_ignore_ascii_case("chunked"))
        {
            body = decode_chunked(&body);
        }
        Some(Res {
            status,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        })
    }

    /// Premier en-tête de ce nom.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    /// Tous les `Set-Cookie`.
    pub fn set_cookies(&self) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n == "set-cookie")
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// `Set-Cookie` complet du cookie `name`.
    pub fn set_cookie(&self, name: &str) -> Option<&str> {
        self.set_cookies()
            .into_iter()
            .find(|c| c.starts_with(&format!("{name}=")))
    }

    /// `name=valeur` d'un cookie posé par la réponse (valeur non vide), à renvoyer tel quel.
    pub fn cookie(&self, name: &str) -> Option<String> {
        let prefix = format!("{name}=");
        self.set_cookies()
            .into_iter()
            .map(|c| c.split(';').next().unwrap_or(""))
            .find(|c| c.starts_with(&prefix) && c.len() > prefix.len())
            .map(str::to_string)
    }

    /// Corps en JSON (`Value::Null` s'il n'en est pas).
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    /// Message d'erreur de l'API (`{ "error": … }`).
    pub fn error(&self) -> String {
        self.json()["error"].as_str().unwrap_or_default().to_string()
    }
}

fn decode_chunked(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(line_end) = body.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&body[..line_end]).trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = line_end + 2;
        let end = (start + size).min(body.len());
        out.extend_from_slice(&body[start..end]);
        body = &body[(end + 2).min(body.len())..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dossier_profil_depuis_binaire_de_test() {
        assert_eq!(
            profile_dir_from_test_exe(Path::new("/repo/target/debug/deps/manager-abc123")),
            Some(PathBuf::from("/repo/target/debug"))
        );
        assert_eq!(
            profile_dir_from_test_exe(Path::new(
                "/repo/target/debug/build/servym-core/5a295beef54adae6/out/manager-5a295beef54adae6"
            )),
            Some(PathBuf::from("/repo/target/debug"))
        );
        assert_eq!(
            profile_dir_from_test_exe(Path::new(
                "/repo/target/release/build/servym-core/1af95074a046d4cf/out/manager-1af95074a046d4cf.exe"
            )),
            Some(PathBuf::from("/repo/target/release"))
        );
        assert_eq!(profile_dir_from_test_exe(Path::new("/tmp/manager")), None);
    }

    #[test]
    fn executable_depuis_sortie_json_cargo() {
        let stdout = concat!(
            r#"{"reason":"compiler-artifact","target":{"name":"servym_fixtures"},"executable":null}"#,
            "\n",
            r#"{"reason":"compiler-artifact","target":{"name":"servym-fixture"},"executable":"/repo/target/debug/servym-fixture"}"#,
            "\n",
            r#"{"reason":"build-finished","success":true}"#,
            "\n",
        );
        assert_eq!(
            executable_from_cargo_json(stdout.as_bytes()),
            Some(PathBuf::from("/repo/target/debug/servym-fixture"))
        );
    }
}
