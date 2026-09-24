//! Durée d'une sonde de port (libre, puis occupé) sur cette machine.
#[tokio::main]
async fn main() {
    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let t = std::time::Instant::now();
    let used = servynex_core::health::port_in_use(free).await;
    println!("port libre {free} : {used} en {:?}", t.elapsed());
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let t = std::time::Instant::now();
    let used = servynex_core::health::port_in_use(busy.local_addr().unwrap().port()).await;
    println!("port occupé : {used} en {:?}", t.elapsed());
}
