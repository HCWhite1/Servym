//! Ports TCP en écoute : `GetExtendedTcpTable` (0,9 ms au lieu de 80 ms pour deux netstat).
//! Adresses au format de netstat : `0.0.0.0`, `127.0.0.1`, `[::]`, `[::1]`, `[fe80::1%12]`.

use std::net::{Ipv4Addr, Ipv6Addr};

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
    MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

use crate::Listener;

/// Table brute d'une famille d'adresses (tampon agrandi tant qu'il est trop petit).
fn table(family: u16) -> Vec<u64> {
    // u64 : alignement suffisant pour les structures de la table.
    let mut buf: Vec<u64> = Vec::new();
    let mut size = 0u32;
    for _ in 0..8 {
        // SAFETY : le tampon fait au moins `size` octets (ou est vide avec size = 0).
        let ret = unsafe {
            GetExtendedTcpTable(
                buf.as_mut_ptr().cast(),
                &mut size,
                0,
                u32::from(family),
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        match ret {
            NO_ERROR => return buf,
            ERROR_INSUFFICIENT_BUFFER => buf = vec![0; (size as usize).div_ceil(8) + 1],
            _ => return Vec::new(),
        }
    }
    Vec::new()
}

/// Numéro de port d'une entrée (ordre réseau dans les 16 bits de poids faible).
fn port(raw: u32) -> u16 {
    u16::from_be(raw as u16)
}

/// Ports en écoute : IPv4 puis IPv6, chacun trié par adresse puis port (comme netstat).
pub fn listeners() -> Vec<Listener> {
    let mut v4: Vec<(u32, u16, u32)> = Vec::new();
    let buf = table(AF_INET);
    if !buf.is_empty() {
        let t = buf.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
        // SAFETY : la table remplie par le système annonce son nombre d'entrées, qui suivent l'en-tête.
        let rows: &[MIB_TCPROW_OWNER_PID] =
            unsafe { std::slice::from_raw_parts((*t).table.as_ptr(), (*t).dwNumEntries as usize) };
        v4.extend(
            rows.iter()
                .map(|r| (u32::from_be(r.dwLocalAddr), port(r.dwLocalPort), r.dwOwningPid)),
        );
    }
    v4.sort();
    let mut v6: Vec<([u8; 16], u32, u16, u32)> = Vec::new();
    let buf = table(AF_INET6);
    if !buf.is_empty() {
        let t = buf.as_ptr().cast::<MIB_TCP6TABLE_OWNER_PID>();
        // SAFETY : comme pour IPv4.
        let rows: &[MIB_TCP6ROW_OWNER_PID] =
            unsafe { std::slice::from_raw_parts((*t).table.as_ptr(), (*t).dwNumEntries as usize) };
        v6.extend(rows.iter().map(|r| {
            (
                r.ucLocalAddr,
                r.dwLocalScopeId,
                port(r.dwLocalPort),
                r.dwOwningPid,
            )
        }));
    }
    v6.sort();
    let mut list: Vec<Listener> = v4
        .into_iter()
        .map(|(addr, port, pid)| Listener {
            address: Ipv4Addr::from(addr).to_string(),
            port,
            pid: Some(pid),
        })
        .collect();
    list.extend(v6.into_iter().map(|(addr, scope, port, pid)| {
        let ip = Ipv6Addr::from(addr);
        let address = if scope != 0 {
            format!("[{ip}%{scope}]")
        } else {
            format!("[{ip}]")
        };
        Listener {
            address,
            port,
            pid: Some(pid),
        }
    }));
    list
}
