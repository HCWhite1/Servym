//! Applications factices lancées par les tests, à la place des scripts node des tests JS.
//!
//! `servynex-fixture <mode>` ; port et adresse par `PORT` et `HOST`, comme un vrai projet.
//!
//! - `server` : répond `<title>Factice</title>` ; écrit « démarrage sur HOST PORT SERVYNEX_TEST_ENV ».
//! - `flaky` : comme `server`, compte ses lancements dans `runs.txt` (dossier courant) et plante
//!   1,5 s après chaque lancement tant qu'il n'a pas planté `CRASHES` fois.
//! - `respawn` : répond `<title>Relance</title>` ; `/restart` lance une nouvelle instance (même
//!   sortie) puis quitte, comme une application qui se relance elle-même.
//! - `launcher` : avertit que le serveur tourne déjà, puis quitte sans erreur (symfony).
//! - `crash` : écrit une ligne puis une erreur, quitte avec le code 3.
//! - `dead` : écrit une erreur, quitte avec le code 3, sans rien d'autre.
//! - `echo <sortie> <erreur> <code>` : écrit les deux textes tels quels et quitte avec ce code.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn listener() -> TcpListener {
    let host = env("HOST");
    let host = if host.is_empty() {
        "0.0.0.0".to_string()
    } else {
        host
    };
    let port: u16 = env("PORT").parse().expect("PORT");
    TcpListener::bind((host.as_str(), port)).expect("écoute")
}

/// Lit la ligne de requête et les en-têtes ; renvoie le chemin demandé.
fn read_path(stream: &TcpStream) -> String {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    let _ = reader.read_line(&mut first);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if line == "\r\n" || line == "\n" => break,
            Ok(_) => {}
        }
    }
    first.split_whitespace().nth(1).unwrap_or("/").to_string()
}

fn respond(mut stream: &TcpStream, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

fn serve(title: &str, on_restart: Option<fn()>) -> ! {
    let listener = listener();
    loop {
        let Ok((stream, _)) = listener.accept() else {
            continue;
        };
        let path = read_path(&stream);
        if path == "/restart" && on_restart.is_some() {
            respond(&stream, "ok");
            break;
        }
        respond(&stream, &format!("<title>{title}</title>"));
    }
    // Port libéré avant que la nouvelle instance ne le reprenne.
    drop(listener);
    on_restart.expect("relance")();
    unreachable!()
}

fn respawn() {
    // Nouvelle instance : même programme, même sortie (le journal), détachée de celle-ci.
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("respawn")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().expect("nouvelle instance");
    std::process::exit(0);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("server") => {
            println!(
                "démarrage sur {} {} {}",
                env("HOST"),
                env("PORT"),
                env("SERVYNEX_TEST_ENV")
            );
            serve("Factice", None)
        }
        Some("flaky") => {
            let runs = std::fs::read_to_string("runs.txt")
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0)
                + 1;
            std::fs::write("runs.txt", runs.to_string()).unwrap();
            let crashes: u32 = env("CRASHES").parse().unwrap_or(0);
            if runs <= crashes {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(1500));
                    eprintln!("Error: plantage {runs}");
                    std::process::exit(3);
                });
            }
            serve("Factice", None)
        }
        Some("respawn") => {
            println!("instance {}", std::process::id());
            serve("Relance", Some(respawn))
        }
        Some("launcher") => {
            println!(" [WARNING] The local web server is already running");
            println!("None");
        }
        Some("crash") => {
            println!("ok");
            eprintln!("Error: base introuvable");
            std::process::exit(3);
        }
        Some("dead") => {
            eprintln!("Error: base introuvable");
            std::process::exit(3);
        }
        Some("echo") => {
            println!("{}", args.get(1).map_or("", String::as_str));
            eprintln!("{}", args.get(2).map_or("", String::as_str));
            std::process::exit(args.get(3).and_then(|c| c.parse().ok()).unwrap_or(0));
        }
        other => {
            eprintln!("mode inconnu : {other:?}");
            std::process::exit(2);
        }
    }
}
