//! Vérification à la main sur une vraie machine : `cargo run --release --example check -p servynex-sys [notify]`.
use std::time::Instant;

use servynex_sys::native;

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
        let run = servynex_sys::run_command();
        let n = servynex_sys::notify::notifier_for(servynex_sys::notify::Os::current(), run);
        let res = n
            .notify(servynex_sys::notify::Notification {
                title: "Servynex · essai".into(),
                body: "Notification native, sans PowerShell <b>&</b> é".into(),
                source: Some("Servynex".into()),
            })
            .await;
        println!("notification : {res:?}");
    }
}
