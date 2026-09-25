//! Vérification à la main sur une vraie machine : `cargo run --release --example check -p servym-sys [notify]`.
use std::time::Instant;

use servym_sys::native;

#[tokio::main]
async fn main() {
    let p = native();
    let t = Instant::now();
    let table = p.sample_processes().await;
    println!(
        "relevé : {} processus en {:.1} ms",
        table.len(),
        t.elapsed().as_secs_f64() * 1000.0
    );
    let t = Instant::now();
    let ports = p.listeners().await;
    println!(
        "ports : {} en {:.2} ms",
        ports.len(),
        t.elapsed().as_secs_f64() * 1000.0
    );
    let t = Instant::now();
    let nets = p.network_profiles().await;
    println!(
        "réseaux en {:.1} ms : {nets:?}",
        t.elapsed().as_secs_f64() * 1000.0
    );
    println!("outils : {:?}", p.tools());
    if std::env::args().any(|a| a == "notify") {
        let run = servym_sys::run_command();
        let n = servym_sys::notify::notifier_for(servym_sys::notify::Os::current(), run);
        let res = n
            .notify(servym_sys::notify::Notification {
                title: "Servym · essai".into(),
                body: "Notification native, sans PowerShell <b>&</b> é".into(),
                source: Some("Servym".into()),
            })
            .await;
        println!("notification : {res:?}");
    }
}
