//! Connexions TCP locales qui échouent vite.
//!
//! Sous Windows, une connexion refusée (port fermé) est réessayée : environ 1 à 2 s avant l'échec,
//! même vers 127.0.0.1. Comme libuv (Node), on désactive ces réessais pour la boucle locale : la
//! réponse « fermé » arrive en quelques millisecondes.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpSocket, TcpStream};

#[cfg(windows)]
fn no_syn_retransmissions(socket: &TcpSocket) {
    use std::os::windows::io::AsRawSocket;

    use windows_sys::Win32::Networking::WinSock::WSAIoctl;

    /// `SIO_TCP_INITIAL_RTO` et `TCP_INITIAL_RTO_NO_SYN_RETRANSMISSIONS` (mstcpip.h).
    const SIO_TCP_INITIAL_RTO: u32 = 0x9800_0011;
    const NO_SYN_RETRANSMISSIONS: u8 = 0xFE;
    #[repr(C)]
    struct InitialRto {
        rtt: u16,
        max_syn_retransmissions: u8,
    }
    let params = InitialRto {
        rtt: u16::from(NO_SYN_RETRANSMISSIONS),
        max_syn_retransmissions: NO_SYN_RETRANSMISSIONS,
    };
    let mut returned = 0u32;
    // SAFETY : structure d'entrée valide pendant l'appel ; aucun tampon de sortie. Un échec (ancien
    // Windows) laisse simplement le comportement habituel.
    unsafe {
        WSAIoctl(
            socket.as_raw_socket() as usize,
            SIO_TCP_INITIAL_RTO,
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

/// Connexion TCP ; vers la boucle locale, un port fermé est signalé tout de suite.
pub async fn connect(addr: SocketAddr, timeout: Duration) -> std::io::Result<TcpStream> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    #[cfg(windows)]
    if addr.ip().is_loopback() {
        no_syn_retransmissions(&socket);
    }
    tokio::time::timeout(timeout, socket.connect(addr))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "délai dépassé"))?
}

/// Quelqu'un écoute-t-il sur cette adresse ?
pub async fn tcp_open(addr: SocketAddr, timeout: Duration) -> bool {
    connect(addr, timeout).await.is_ok()
}
